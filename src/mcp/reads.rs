//! The read-only tools that work something out (LIO-470): the plan checks
//! on the job as it is (`check_job`, `check_holding`, `stock_after`), the
//! part (`part_report`, `list_surfaces`, `surface_info`), speeds and feeds
//! (`cutting_data`), pictures (`view_model`), the machining rules (`guide`),
//! a hand setup's steps (`inspect {what: "steps"}`) and the NC review
//! (`inspect {what: "post"}`).
//!
//! A read that computes takes a `JobSnapshot` on the `App`'s thread and
//! works on it on a thread of its own, so the `App` keeps taking events
//! (a `cancel`, an E-stop) meanwhile. Its call is answered through the
//! engine's `command_id` and `wait`, as a command's is: within `wait_s`, or
//! `still_running` with the id.
//!
//! **Fresh.** The work state's fingerprint (`state::snapshot`) is taken when
//! the snapshot is, and again when the result is ready, both on the `App`'s
//! thread. When they differ the result says `"stale": true` and what
//! changed; `check_job` is then refused as stale instead.
//!
//! **Heavy and light.** A read that simulates toolpaths (`check_job`,
//! `check_holding`, `stock_after`, a `view_model` of an operation, the steps
//! of a guide still to build, an NC review the app has not checked for the
//! job as it is) is heavy: while the machine is `locked()` it
//! is refused, and its worker is counted in `App::planning_busy` from its
//! start until its thread has actually ended, so Cycle Start waits for it,
//! whatever became of its call (cancelled, timed out, the client gone). The
//! other reads draw or read the part only and run at any time.
use super::{
    commands::{self, Problem},
    engine::{Answer, Selection},
    rules,
    state::{self, Change},
};
use crate::{
    App, analysis, guide, look,
    mesh::Mesh,
    model::{Document, get, num},
    plan_check,
};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering::SeqCst},
    mpsc,
};

/// The key of a result that carries a picture: its PNG in base64, sent to
/// the client as an MCP image (`protocol::tool_result`).
pub const IMAGE: &str = "png_base64";

/// What a heavy read is refused with while the machine runs.
pub const LOCKED: &str =
    "The machine is running; this check simulates toolpaths, so it waits until the run stops.";

/// The tools of this module, as `tools/list` names them.
pub const TOOLS: [&str; 9] = [
    "check_job",
    "check_holding",
    "stock_after",
    "cutting_data",
    "surface_info",
    "part_report",
    "list_surfaces",
    "view_model",
    "guide",
];

/// Whether `tool` with `args` is one of this module's reads.
pub fn is_read(tool: &str, args: &Value) -> bool {
    TOOLS.contains(&tool)
        || (tool == "inspect"
            && matches!(
                args.get("what").and_then(Value::as_str),
                Some("steps" | "post")
            ))
}

/// Counts the heavy reads' workers still running; each stops counting when
/// its thread ends (`HeavyGuard`).
#[derive(Debug, Clone, Default)]
pub struct Heavy(Arc<AtomicUsize>);
/// One heavy worker running.
pub struct HeavyGuard(Arc<AtomicUsize>);
impl Heavy {
    pub fn running(&self) -> usize {
        self.0.load(SeqCst)
    }
    fn enter(&self) -> HeavyGuard {
        self.0.fetch_add(1, SeqCst);
        HeavyGuard(self.0.clone())
    }
}
impl Drop for HeavyGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, SeqCst);
    }
}

/// The job as a read works on it, off the `App`'s thread: a copy of the
/// `Document`, the part (shared), the selection, and the revision and the
/// work-state fingerprint it was taken at.
pub struct JobSnapshot {
    pub doc: Document,
    pub mesh: Option<Arc<Mesh>>,
    pub selection: Selection,
    pub revision: u64,
    pub fingerprint: u64,
}
impl JobSnapshot {
    /// The revision and the work-state fingerprint the read worked at.
    fn checked_at(&self) -> Value {
        json!({"revision": self.revision, "fingerprint": format!("{:016x}", self.fingerprint)})
    }
}
/// A `JobSnapshot` crosses to a worker's thread; the `App` never does.
const fn assert_send<T: Send>() {}
const _: () = assert_send::<JobSnapshot>();

/// A read under way.
struct Reading {
    id: String,
    tool: &'static str,
    /// Counted in `App::planning_busy` until its worker ends.
    heavy: bool,
    /// The work state when its snapshot was taken.
    before: state::Snapshot,
    revision: u64,
    /// `check_job`: refused when the job changed meanwhile.
    refuse_stale: bool,
    done: mpsc::Receiver<Answer>,
}

/// What starting a read came to.
pub enum Started {
    /// Answered now: a light read that needs no worker, a refusal, or an
    /// error in its arguments.
    Now(Answer),
    /// Working, under the id it was given.
    Working,
}

/// The reads under way, and the part they share.
#[derive(Default)]
pub struct Reads {
    running: Vec<Reading>,
    /// The part as the last snapshot shared it: the next takes it again
    /// while the `App`'s part is the same.
    mesh: Option<Arc<Mesh>>,
}

/// A refused read: nothing was worked out.
fn refused(why: impl Into<String>) -> Answer {
    Ok(json!({"outcome": "refused", "reason": why.into()}))
}
/// A read's result, done.
fn done(mut v: Value) -> Answer {
    if let Value::Object(o) = &mut v {
        o.insert("outcome".into(), json!("done"));
    }
    Ok(v)
}
/// A command's problem as a read's answer.
fn problem(p: Problem) -> Answer {
    match p {
        Problem::Args(e) => Err(e),
        Problem::Refused(why) => refused(why),
    }
}

impl Reads {
    /// Whether any read is still working.
    pub fn any(&self) -> bool {
        !self.running.is_empty()
    }
    /// Whether the read `id` is still working.
    pub fn working(&self, id: &str) -> bool {
        self.running.iter().any(|r| r.id == id)
    }
    /// Starts read `tool` with `args` on `app`'s job under `id`. `before`
    /// gives the work state now, taken only for a read that works.
    pub fn start(
        &mut self,
        app: &App,
        id: &str,
        tool: &str,
        args: &Value,
        before: impl FnOnce() -> state::Snapshot,
    ) -> Started {
        let plan = match self.plan(app, tool, args) {
            Ok(plan) => plan,
            Err(answer) => return Started::Now(answer),
        };
        let (name, heavy, refuse_stale, work) = match plan {
            Plan::Now(answer) => return Started::Now(answer),
            Plan::Work {
                name,
                heavy,
                refuse_stale,
                work,
            } => (name, heavy, refuse_stale, work),
        };
        if heavy && app.locked() {
            return Started::Now(refused(LOCKED));
        }
        let before = before();
        let snapshot = JobSnapshot {
            doc: app.doc.clone(),
            mesh: self.shared_mesh(app),
            selection: Selection::of(app),
            revision: app.mcp.log.revision,
            fingerprint: before.hash,
        };
        let guard = heavy.then(|| app.mcp.heavy.enter());
        #[cfg(test)]
        let slow = app.mcp.slow_reads;
        let (tx, done) = mpsc::channel();
        // A thread that does not start drops its closure, and the guard
        // with it; the read then fails (`finished`).
        let _ = std::thread::Builder::new()
            .name(format!("mcp-{name}"))
            .spawn(move || {
                let _guard = guard;
                #[cfg(test)]
                if let Some(slow) = slow {
                    std::thread::sleep(slow);
                }
                let answer =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&snapshot)))
                        .unwrap_or_else(|_| Err(format!("The {name} read failed.")));
                let _ = tx.send(answer);
            });
        self.running.push(Reading {
            id: id.into(),
            tool: name,
            heavy,
            before,
            revision: app.mcp.log.revision,
            refuse_stale,
            done,
        });
        Started::Working
    }

    /// The part for a snapshot: the one shared last while it is the same.
    fn shared_mesh(&mut self, app: &App) -> Option<Arc<Mesh>> {
        let mesh = app.part_mesh()?;
        let same = |m: &Mesh| {
            m.fingerprint == mesh.fingerprint
                && m.triangles == mesh.triangles
                && m.dimensions == mesh.dimensions
                && m.origin == mesh.origin
        };
        match &self.mesh {
            Some(m) if same(m) => Some(m.clone()),
            _ => {
                let m = Arc::new(mesh.clone());
                self.mesh = Some(m.clone());
                Some(m)
            }
        }
    }

    /// The reads whose workers have answered, each with its final result,
    /// checked for freshness against `app`'s work state now.
    pub fn finished(&mut self, app: &App) -> Vec<(String, Value)> {
        let mut out = vec![];
        let mut now: Option<state::Snapshot> = None;
        let mut i = 0;
        while i < self.running.len() {
            let answer = match self.running[i].done.try_recv() {
                Ok(answer) => answer,
                Err(mpsc::TryRecvError::Empty) => {
                    i += 1;
                    continue;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    Err("The read's worker could not run.".into())
                }
            };
            let r = self.running.remove(i);
            let now = now.get_or_insert_with(|| state::snapshot(app));
            out.push((r.id.clone(), fresh(&r, answer, now)));
        }
        out
    }

    /// Cancels read `id`: its final result, `cancelled`. Its worker runs on
    /// to its end, and a heavy one is counted until then. None when no such
    /// read is working.
    pub fn cancel(&mut self, id: &str) -> Option<Value> {
        let at = self.running.iter().position(|r| r.id == id)?;
        let r = self.running.remove(at);
        let reason = if r.heavy {
            "Cancelled at the agent's request. Its worker cannot be stopped and finishes on its own; until it has, the machine's Cycle Start waits for it."
        } else {
            "Cancelled at the agent's request. Its worker finishes on its own; its result is dropped."
        };
        Some(json!({
            "outcome": "cancelled",
            "command_id": r.id,
            "tool": r.tool,
            "reason": reason,
        }))
    }

    /// The answer to a call on read `id` whose time is up.
    pub fn still_running(&self, id: &str) -> Value {
        let tool = (self.running.iter().find(|r| r.id == id)).map_or("read", |r| r.tool);
        json!({
            "outcome": "still_running",
            "command_id": id,
            "tool": tool,
            "reason": format!("The {tool} read is still working. Call wait with its command_id for its result."),
        })
    }
}

/// A read's final result: its answer with its id, the revision and
/// fingerprint it was worked out at, and whether the job changed since
/// (`now`): then `stale` and what changed, or for `check_job` refused.
fn fresh(r: &Reading, answer: Answer, now: &state::Snapshot) -> Value {
    let changed: Vec<Change> = if now.hash == r.before.hash {
        vec![]
    } else {
        state::diff(&r.before, now)
    };
    let stale = now.hash != r.before.hash;
    let checked_at =
        json!({"revision": r.revision, "fingerprint": format!("{:016x}", r.before.hash)});
    let mut v = match answer {
        _ if stale && r.refuse_stale => json!({
            "outcome": "refused",
            "reason": format!(
                "The job changed while it was being checked ({}), so this check is not of the job as it is now. Nothing was reported from it: call {} again.",
                named(&changed),
                r.tool
            ),
        }),
        Ok(v) => v,
        Err(why) => json!({"outcome": "failed", "reason": why}),
    };
    if let Value::Object(o) = &mut v {
        o.insert("command_id".into(), json!(r.id));
        o.entry("checked_at").or_insert(checked_at);
        o.insert("stale".into(), json!(stale));
        if r.tool == "post" {
            o.insert("current".into(), json!(!stale));
        }
        if stale {
            o.insert("changed_meanwhile".into(), json!(changed));
        }
    }
    v
}
/// What changed, in a few words: "setup 1a2b changed, operation 3c4d added".
fn named(changed: &[Change]) -> String {
    let words: Vec<String> = (changed.iter().take(6))
        .map(|c| {
            let what = format!("{:?}", c.what).to_lowercase();
            if c.id.is_empty() {
                format!("{} {what}", c.kind.replace('_', " "))
            } else {
                format!("{} {} {what}", c.kind, c.id)
            }
        })
        .collect();
    let more = changed.len().saturating_sub(words.len());
    match more {
        0 => words.join(", "),
        n => format!("{} and {n} more", words.join(", ")),
    }
}

/// A worker's work on its snapshot.
type Work = Box<dyn FnOnce(&JobSnapshot) -> Answer + Send>;
/// How a read is answered: now, or by a worker.
enum Plan {
    Now(Answer),
    Work {
        name: &'static str,
        heavy: bool,
        refuse_stale: bool,
        work: Work,
    },
}
fn work(
    name: &'static str,
    heavy: bool,
    work: impl FnOnce(&JobSnapshot) -> Answer + Send + 'static,
) -> Result<Plan, Answer> {
    Ok(Plan::Work {
        name,
        heavy,
        refuse_stale: name == "check_job",
        work: Box::new(work),
    })
}

/// A setup's place: index, id and name.
fn setup_ref(doc: &Document, si: usize) -> Value {
    let s = &doc.setups[si];
    json!({"index": si, "id": s.id, "name": s.name()})
}
/// An operation's place in setup `si`: index, id and name.
fn operation_ref(doc: &Document, si: usize, oi: usize) -> Value {
    match doc.setups.get(si).and_then(|s| s.operations.get(oi)) {
        Some(o) => json!({
            "index": oi,
            "id": o.id,
            "name": crate::threading::shown_name(o, &doc.tools),
        }),
        None => Value::Null,
    }
}
/// The setup `args` name (`setup`, an id or index), else the active one.
fn setup_arg(app: &App, args: &Value) -> Result<usize, Answer> {
    match commands::resolve(commands::On::Setup, app, args) {
        Ok((Some(si), _)) => Ok(si),
        Ok((None, _)) => Err(refused("The job has no setup: add one first.")),
        Err(p) => Err(problem(p)),
    }
}
/// The part, or the refusal that says there is none.
fn needs_part(app: &App) -> Result<(), Answer> {
    match app.part_mesh() {
        Some(_) => Ok(()),
        None => Err(refused(
            "There is no part: import one (import_model) or open the sample first.",
        )),
    }
}
/// The snapshot's part; a read that needs one checked for it first.
fn part(s: &JobSnapshot) -> Result<&Mesh, String> {
    s.mesh.as_deref().ok_or_else(|| "There is no part.".into())
}
/// The part report of `doc` (`analysis::report`), for the checks. When only
/// setup 1's workholding stops it (the report refuses a holder that cannot
/// hold the raw stock), the report of the same job with no holding, for what
/// it says of the part: the checks then say what is wrong with the holding.
fn part_report(doc: &Document, mesh: &Mesh) -> Result<analysis::PartReport, String> {
    report_with(doc, mesh, analysis::report)
}
/// `part_report` for what the part's surfaces and planes are, a picture or
/// a list: an open mesh has them too (`analysis::report_unchecked`).
fn surfaces_report(doc: &Document, mesh: &Mesh) -> Result<analysis::PartReport, String> {
    report_with(doc, mesh, analysis::report_unchecked)
}
fn report_with(
    doc: &Document,
    mesh: &Mesh,
    report: fn(&Document, &Mesh) -> Result<analysis::PartReport, String>,
) -> Result<analysis::PartReport, String> {
    match report(doc, mesh) {
        Err(e) if e.starts_with("Setup 1's workholding") => {
            let mut bare = doc.clone();
            for s in &mut bare.setups {
                s.p.insert("holding".into(), "None".into());
            }
            report(&bare, mesh).map_err(|_| e)
        }
        other => other,
    }
}
/// The last enabled operation of setup `si`, as `simulation::sequence_walk`
/// stops after it; None when it has none (the walk goes on to the end).
fn last_enabled(doc: &Document, si: usize) -> Option<(usize, usize)> {
    let oi = doc.setups[si].operations.iter().rposition(|o| o.enabled)?;
    Some((si, oi))
}
/// The first enabled operation of setup `si`, which the walk has begun the
/// setup for.
fn first_enabled(doc: &Document, si: usize) -> Option<(usize, usize)> {
    let oi = doc.setups[si].operations.iter().position(|o| o.enabled)?;
    Some((si, oi))
}
/// Three numbers rounded to the thousandth.
fn rounded3(v: [f32; 3]) -> [f32; 3] {
    v.map(|x| (x * 1000.).round() / 1000. + 0.)
}

impl Reads {
    /// How read `tool` is answered.
    fn plan(&mut self, app: &App, tool: &str, args: &Value) -> Result<Plan, Answer> {
        match tool {
            "check_job" => {
                needs_part(app)?;
                work("check_job", true, check_job)
            }
            "check_holding" => {
                let si = setup_arg(app, args)?;
                needs_part(app)?;
                work("check_holding", true, move |s| check_holding(s, si))
            }
            "stock_after" => {
                let si = setup_arg(app, args)?;
                needs_part(app)?;
                work("stock_after", true, move |s| stock_after(s, si))
            }
            "cutting_data" => Ok(Plan::Now(cutting_data(app, args))),
            "surface_info" => {
                let id = (args.get("id").and_then(Value::as_u64))
                    .ok_or_else(|| Err("`id` is a surface id from list_surfaces.".into()))?
                    as usize;
                needs_part(app)?;
                work("surface_info", false, move |s| {
                    let mesh = part(s)?;
                    let report = surfaces_report(&s.doc, mesh)?;
                    match analysis::surface_info(&s.doc, mesh, &report, id) {
                        Ok(info) => done(serde_json::to_value(&info).map_err(|e| e.to_string())?),
                        Err(why) => refused(why),
                    }
                })
            }
            "part_report" => {
                needs_part(app)?;
                work("part_report", false, |s| {
                    let mesh = part(s)?;
                    match analysis::report(&s.doc, mesh) {
                        Ok(report) => done(json!({"report": report})),
                        // What the part is, with why the checks cannot use it.
                        Err(why) => match analysis::report_unchecked(&s.doc, mesh) {
                            Ok(report) => done(json!({"report": report, "problem": why})),
                            Err(_) => refused(why),
                        },
                    }
                })
            }
            "list_surfaces" => list_surfaces(app, args),
            "view_model" => view_model(app, args),
            "guide" => Ok(Plan::Now(guide_topic(args))),
            "inspect" if args.get("what") == Some(&json!("post")) => post_review(app, args),
            "inspect" => steps(app, args),
            other => Err(Err(format!("There is no read {other}."))),
        }
    }
}

/// `check_job`: the plan checks on the job as it is (`plan_check::check_as_is`).
fn check_job(s: &JobSnapshot) -> Answer {
    let mesh = part(s)?;
    let doc = &s.doc;
    let report = match part_report(doc, mesh) {
        Ok(report) => report,
        Err(why) => {
            return done(json!({
                "passed": false,
                "problems": [{"setup": 0, "operation": null, "stage": "part", "message": why}],
                "operations": [],
                "total_minutes": 0,
                "stocks": [],
            }));
        }
    };
    let (check, _) = plan_check::check_as_is(doc, mesh, &report);
    let mut v = serde_json::to_value(&check).map_err(|e| e.to_string())?;
    // Each problem and operation with the ids the commands take.
    let ids = |item: &mut Value| {
        let si = item["setup"]
            .as_u64()
            .and_then(|n| (n as usize).checked_sub(1));
        let oi = item["operation"]
            .as_u64()
            .and_then(|n| (n as usize).checked_sub(1));
        if let Some(si) = si.filter(|si| *si < doc.setups.len()) {
            item["setup_id"] = json!(doc.setups[si].id);
            if let Some(oi) = oi {
                item["operation_id"] = operation_ref(doc, si, oi)["id"].clone();
            }
        }
    };
    for key in ["problems", "operations"] {
        if let Some(items) = v[key].as_array_mut() {
            items.iter_mut().for_each(ids);
        }
    }
    v["numbering"] = json!(
        "setup and operation count from 1 in this report; setup_id and operation_id are the ids the commands take"
    );
    v["checked_at"] = s.checked_at();
    v["selection"] = json!(s.selection);
    done(v)
}

/// The placement keys of a setup's holding kind, as saved.
fn placement_keys(holding: &str, layout: Option<&crate::holding::Layout>) -> Vec<&'static str> {
    match holding {
        "Vise" => vec!["vise_axis", "vise_offset", "vise_grip"],
        "Clamps" => vec!["clamps"],
        "Chuck" => vec!["grip", "stickout"],
        _ => layout.map_or(vec![], |l| {
            crate::holding::placement(l)
                .into_iter()
                .map(|(k, _)| k)
                .collect()
        }),
    }
}

/// `check_holding {setup}`: setup `si`'s holder and placement exactly as
/// saved, checked as the plan check's workholding stage checks them.
/// Nothing is suggested or filled in: a missing or unreadable placement is
/// reported as such.
fn check_holding(s: &JobSnapshot, si: usize) -> Answer {
    use crate::holding::{Layout, Side, layout};
    let mesh = part(s)?;
    let doc = &s.doc;
    let setup = &doc.setups[si];
    let holding = get(&setup.p, "holding");
    let holder = doc.holder(get(&setup.p, "holder"));
    let laid = layout(setup);
    let placement: serde_json::Map<String, Value> = (placement_keys(holding, laid.as_ref().ok())
        .into_iter())
    .map(|k| (k.to_string(), json!(get(&setup.p, k))))
    .collect();
    let mut v = json!({
        "setup": setup_ref(doc, si),
        "holding": if holding.is_empty() { "None" } else { holding },
        "holder": holder.map(|h| json!({"id": h.id, "name": h.name(), "kind": h.kind()})),
        "placement": placement,
        "gap": get(&setup.p, "holding_gap"),
    });
    if let Ok(Layout::Clamps { clamps, .. }) = &laid {
        v["clamps"] = json!(
            (clamps.iter())
                .map(|(side, offset): &(Side, f32)| json!({"side": side.label(), "offset": offset}))
                .collect::<Vec<_>>()
        );
    }
    if let Ok(Layout::Vise { axis_y, .. }) = &laid {
        v["jaws_on"] = json!(if *axis_y { "±Y" } else { "±X" });
    }
    let verdict = |mut v: Value, problems: Vec<Value>| {
        v["holds"] = json!(problems.is_empty());
        v["problems"] = json!(problems);
        done(v)
    };
    let whole = |message: String| json!({"operation": null, "message": message});
    if matches!(laid, Ok(Layout::None)) && !crate::sheet::screwed(setup) {
        v["holds"] = Value::Null;
        v["problems"] = json!([]);
        v["note"] = json!(
            "The setup models no vise, clamps or chuck: nothing holding its stock is checked. Choose its holding in the setup's fields to have it checked."
        );
        return done(v);
    }
    if let Some(why) = plan_check::missing_placement(setup, si) {
        return verdict(v, vec![whole(why)]);
    }
    if let Err(why) = &laid {
        return verdict(v, vec![whole(format!("Setup {}: {why}", si + 1))]);
    }
    let setups = plan_check::walked_setups(doc, mesh, last_enabled(doc, si));
    let Some(start) = setups.get(si) else {
        return verdict(
            v,
            vec![whole(format!(
                "The stock setup {} starts from is not known: an operation before it could not be generated (check_job says which).",
                si + 1
            ))],
        );
    };
    v["stock_size"] = json!(start.stock.dimensions().map(plan_check::hundredths));
    let problems = (plan_check::setup_holding(doc, mesh, si, start).into_iter())
        .map(|(oi, message)| {
            json!({
                "operation": oi.map(|oi| operation_ref(doc, si, oi)),
                "message": crate::proposal::clip(&message, 300),
            })
        })
        .collect();
    verdict(v, problems)
}

/// `stock_after {setup}`: the stock setup `si` starts from, in its axes, as
/// the earlier setups leave it.
fn stock_after(s: &JobSnapshot, si: usize) -> Answer {
    let mesh = part(s)?;
    let doc = &s.doc;
    let stop =
        first_enabled(doc, si).or_else(|| si.checked_sub(1).and_then(|p| last_enabled(doc, p)));
    let setups = plan_check::walked_setups(doc, mesh, stop);
    match setups.get(si) {
        Some(start) => {
            let mut v = serde_json::to_value(&start.summary).map_err(|e| e.to_string())?;
            v["setup"] = setup_ref(doc, si);
            done(v)
        }
        None => refused(format!(
            "The stock before setup {} is not known: an operation before it could not be generated (check_job says which).",
            si + 1
        )),
    }
}

/// `cutting_data {tool, op?}`: the speeds and feeds a tool can run at in
/// the project's material, on the operation's machine (or the active
/// setup's).
fn cutting_data(app: &App, args: &Value) -> Answer {
    let doc = &app.doc;
    let (si, op) = match args.get("op") {
        None | Some(Value::Null) => match setup_arg(app, &json!({})) {
            Ok(si) => (si, None),
            Err(answer) => return answer,
        },
        Some(v) => match commands::operation_of(app, None, v) {
            Ok((si, oi)) => (si, Some(oi)),
            Err(p) => return problem(p),
        },
    };
    // Without `tool`, the operation's own.
    let own = op.map(|oi| doc.setups[si].operations[oi].tool.as_str());
    let tool = match (args.get("tool"), own) {
        (Some(Value::String(id)), _) => doc.tools.iter().find(|t| t.id == *id),
        (Some(Value::Number(n)), _) => {
            let n = n.as_u64().and_then(|n| u32::try_from(n).ok());
            doc.tools.iter().find(|t| n.is_some() && t.number() == n)
        }
        (None | Some(Value::Null), Some(id)) => doc.tools.iter().find(|t| t.id == id),
        _ => return Err("`tool` is a tool's id, or its number.".into()),
    };
    let Some(tool) = tool else {
        return refused(match args.get("tool") {
            Some(t) if !t.is_null() => format!("There is no tool {t} in the library."),
            _ => "The operation has no tool: choose one first.".into(),
        });
    };
    if tool.is_lathe() {
        return refused(format!(
            "{} is a lathe tool: the lathe's speeds are chosen from the material by hand (guide topic chip_load).",
            tool.name()
        ));
    }
    let machine = &doc.setups[si].machine;
    let material = match analysis::material_report(doc) {
        Ok(m) => m,
        Err(why) => return refused(why),
    };
    let limits = match analysis::machine_report(machine) {
        Ok(m) => m,
        Err(why) => return refused(why),
    };
    let diameter = num(&tool.p, "diameter").unwrap_or(0.);
    let flutes = num(&tool.p, "flutes")
        .filter(|f| *f >= 1. && f.fract() == 0.)
        .unwrap_or(0.) as u32;
    let named = tool.name().to_string();
    let data = match analysis::cutting_data(
        &material,
        &limits,
        (tool.number().unwrap_or(0), &named),
        diameter,
        flutes,
    ) {
        Ok(data) => data,
        Err(why) => return refused(why),
    };
    let mut v = serde_json::to_value(&data).map_err(|e| e.to_string())?;
    v["tool"] = json!({"id": tool.id, "name": tool.name(), "number": tool.number()});
    v["machine"] =
        json!({"name": machine.name(), "max_rpm": limits.max_rpm, "max_feed": limits.max_feed});
    v["material"] = json!(material.name);
    if let Some(oi) = op {
        let o = &doc.setups[si].operations[oi];
        let mut at = operation_ref(doc, si, oi);
        at["setup_id"] = json!(doc.setups[si].id);
        at["rpm"] = json!(get(&o.p, "rpm"));
        at["feed"] = json!(get(&o.p, "feed"));
        at["tool_id"] = json!(o.tool);
        v["operation"] = at;
    }
    done(v)
}

/// The most surfaces `list_surfaces` lists by default, and at most.
const SURFACES: usize = 300;
const MOST_SURFACES: usize = 5000;

/// `list_surfaces {setup?, limit?}`: the part's surfaces, the largest
/// first, and the part report's planes from the setup's orientation; on the
/// built-in sample, its features.
fn list_surfaces(app: &App, args: &Value) -> Result<Plan, Answer> {
    let si = setup_arg(app, args)?;
    if app.is_sample() {
        let features: Vec<Value> = (crate::model::FEATURES.iter().enumerate())
            .map(|(id, name)| json!({"id": id, "name": name}))
            .collect();
        return Ok(Plan::Now(done(json!({
            "sample": true,
            "setup": setup_ref(&app.doc, si),
            "features": features,
            "note": "The built-in sample is picked by its features: pick_features takes these ids, not pick_surfaces.",
        }))));
    }
    needs_part(app)?;
    let limit = (args.get("limit").and_then(Value::as_u64))
        .map_or(SURFACES, |n| (n as usize).clamp(1, MOST_SURFACES));
    work("list_surfaces", false, move |s| {
        let mesh = part(s)?;
        let doc = &s.doc;
        let setup = &doc.setups[si];
        let scale = mesh.display_scale();
        let mut order: Vec<usize> = (0..mesh.surfaces.len()).collect();
        order.sort_by(|a, b| mesh.surfaces[*b].area.total_cmp(&mesh.surfaces[*a].area));
        let surfaces: Vec<Value> = (order.iter().take(limit))
            .map(|&id| {
                let f = &mesh.surfaces[id];
                let centre: [f32; 3] =
                    std::array::from_fn(|k| (f.center[k] - mesh.origin[k]) / scale);
                json!({
                    "id": id,
                    "normal": rounded3(f.normal),
                    "centre": rounded3(centre),
                    "area": (f.area / (scale * scale) * 100.).round() / 100.,
                    "region": mesh.regions.get(id).copied().unwrap_or(id),
                })
            })
            .collect();
        let up = analysis::up_axis(setup);
        let mut v = json!({
            "setup": setup_ref(doc, si),
            "up": up,
            "total": mesh.surfaces.len(),
            "listed": surfaces.len(),
            "surfaces": surfaces,
            "frame": "normal and centre (mm) in the part's own frame as imported; area in mm²; region: the smooth face the surface belongs to (a curved face is many surfaces)",
        });
        match surfaces_report(doc, mesh) {
            Ok(report) => {
                let planes = (report.orientations.iter())
                    .find(|o| Some(o.up) == up)
                    .map(|o| &o.planes);
                v["planes"] = json!(planes);
            }
            Err(why) => v["planes_unavailable"] = json!(why),
        }
        done(v)
    })
}

/// `view_model {view, …}`: a picture of the part (`look::draw`), with its
/// stock and holding, or an operation's toolpaths and the stock it leaves.
fn view_model(app: &App, args: &Value) -> Result<Plan, Answer> {
    needs_part(app)?;
    let view = (args.get("view").and_then(Value::as_str)).unwrap_or("iso_front_left");
    let operation = match args.get("operation") {
        None | Some(Value::Null) => Value::Null,
        Some(v) => match commands::operation_of(app, None, v) {
            Ok((si, oi)) => json!({"setup": si + 1, "operation": oi + 1}),
            Err(p) => return Err(problem(p)),
        },
    };
    let list = |key: &str| args.get(key).cloned().unwrap_or(json!([]));
    let flag = |key: &str| args.get(key).and_then(Value::as_bool).unwrap_or(true);
    let heavy = !operation.is_null();
    let arguments = json!({
        "orientation": args.get("orientation").cloned().unwrap_or(Value::Null),
        "angle": view,
        "focus_surfaces": list("focus_surfaces"),
        "highlight_surfaces": list("highlight_surfaces"),
        "show_stock": flag("show_stock"),
        "show_workholding": flag("show_workholding"),
        "labels": flag("labels"),
        "operation": operation,
        "section": args.get("section").cloned().unwrap_or(Value::Null),
        "purpose": args.get("purpose").cloned().unwrap_or(json!("")),
    })
    .to_string();
    work("view_model", heavy, move |s| {
        use base64::Engine;
        let mesh = s.mesh.clone().ok_or("There is no part.")?;
        let report = surfaces_report(&s.doc, &mesh)?;
        let doc = Arc::new(s.doc.clone());
        let scene = look::Scene {
            doc: doc.clone(),
            mesh,
            report: Arc::new(report),
            job: Some(doc),
        };
        match look::draw(&scene, &arguments) {
            Ok(picture) => done(json!({
                "description": picture.description,
                IMAGE: base64::engine::general_purpose::STANDARD.encode(picture.png.as_slice()),
            })),
            Err(why) => refused(why),
        }
    })
}

/// `guide {topic}`: the machining rules of a topic.
fn guide_topic(args: &Value) -> Answer {
    let topics: Vec<Value> = (rules::TOPICS.iter())
        .map(|(name, what, _)| json!({"topic": name, "holds": what}))
        .collect();
    match args.get("topic").and_then(Value::as_str) {
        None => done(json!({"topics": topics})),
        Some(topic) => match rules::text(topic) {
            Some(text) => done(json!({"topic": topic, "rules": text})),
            None => Err(format!(
                "There is no topic {topic}: the topics are {}.",
                rules::TOPICS.map(|t| t.0).join(", ")
            )),
        },
    }
}

/// `inspect {what: "steps", setup, step?}`: a step of a hand setup's guide,
/// read with a cursor of its own: never the machinist's place, and nothing
/// of the `App`'s guide is touched. The guide the app has built for the
/// active setup as the job now is (`App::current_guide`) is read at once;
/// another is built on a snapshot, which simulates the setup (heavy).
fn steps(app: &App, args: &Value) -> Result<Plan, Answer> {
    let si = setup_arg(app, args)?;
    let doc = &app.doc;
    let setup = &doc.setups[si];
    if !crate::manual::is_hand(&setup.machine) {
        return Err(refused(format!(
            "Setup {} runs on a CNC: it has a program, not steps. Review its NC instead.",
            si + 1
        )));
    }
    let step = match args.get("step") {
        None | Some(Value::Null) => None,
        Some(v) => Some(
            v.as_u64()
                .ok_or_else(|| Err("`step` is a step's index from 0.".into()))?
                as usize,
        ),
    };
    if setup.id == doc.active
        && let Some(built) = app.current_guide()
    {
        return Ok(Plan::Now(match built {
            Ok(shown) => steps_of(doc, &shown.guide, step, "cached"),
            Err(why) => refused(why.clone()),
        }));
    }
    needs_part(app)?;
    work("steps", true, move |s| {
        let mesh = part(s)?;
        match guide::build(&s.doc, mesh, si) {
            Ok(guide) => steps_of(&s.doc, &guide, step, "built"),
            Err(why) => refused(why),
        }
    })
}

/// The most G-code `inspect {what: "post", program: true}` gives, bytes.
pub const PROGRAM_CAP: usize = 200_000;

/// `inspect {what: "post", setup?, program?}`: the NC review as the app's
/// NC review page has it (`post::post_job`, which export writes from): for
/// each setup, or the one asked, its programs (file names as export names
/// them, lines, tools, operations and estimated time) or what its checks
/// refuse, each problem with its operation; and with `program: true` the
/// G-code of the programs of `setup` (default the active one), at most
/// `PROGRAM_CAP` bytes. The app's own check is read when it is of the job
/// as it is now (`App::post_current`); otherwise it is worked out on a
/// snapshot, which simulates the job (heavy).
fn post_review(app: &App, args: &Value) -> Result<Plan, Answer> {
    let asked = match args.get("setup") {
        None | Some(Value::Null) => None,
        Some(_) => Some(setup_arg(app, args)?),
    };
    let program = match args.get("program") {
        None | Some(Value::Null) => false,
        Some(v) => v
            .as_bool()
            .ok_or_else(|| Err("`program` is true or false.".into()))?,
    };
    let text_of = match (program, asked) {
        (false, _) => None,
        (true, Some(si)) => Some(si),
        (true, None) => Some(setup_arg(app, args)?),
    };
    let app_check = if app.post_current() {
        "current"
    } else if app.post_busy {
        "running"
    } else {
        "not_current"
    };
    if app.post_current()
        && let Some(job) = &app.post
    {
        let mut v = review(&app.doc, job, asked, text_of);
        v["source"] = json!("app");
        v["app_check"] = json!(app_check);
        v["current"] = json!(true);
        v["stale"] = json!(false);
        let now = state::snapshot(app);
        v["checked_at"] =
            json!({"revision": app.mcp.log.revision, "fingerprint": format!("{:016x}", now.hash)});
        return Ok(Plan::Now(done(v)));
    }
    work("post", true, move |s| {
        let job = match &s.mesh {
            Some(mesh) => crate::post::post_job(&s.doc, mesh),
            None => crate::post::JobPost::blocked("Load the imported part before posting."),
        };
        let mut v = review(&s.doc, &job, asked, text_of);
        v["source"] = json!("worked_out");
        v["app_check"] = json!(app_check);
        v["checked_at"] = s.checked_at();
        done(v)
    })
}

/// The NC review of `doc` that `job` checked: every setup, or `only`; with
/// the G-code of setup `text_of`'s programs.
fn review(
    doc: &Document,
    job: &crate::post::JobPost,
    only: Option<usize>,
    text_of: Option<usize>,
) -> Value {
    let setups: Vec<Value> = (job.setups.iter())
        .filter(|s| only.is_none_or(|si| s.setup == si))
        .filter(|s| s.setup < doc.setups.len())
        .map(|s| setup_review(doc, s))
        .collect();
    let lines = crate::ui::speed_feed_lines_of(doc);
    let speeds: Vec<Value> = (lines.iter())
        .map(|(problem, line)| json!({"line": line, "problem": problem}))
        .collect();
    let mut v = json!({
        "ready": job.ready(),
        "errors": job.errors,
        "setups": setups,
        "speeds_and_feeds": speeds,
        "numbering": "setup and operation index count from 0, as in inspect; id is the id the commands take",
    });
    if !speeds.is_empty() {
        v["speeds_and_feeds_note"] = json!(crate::ui::speed_feed_heading(&lines));
    }
    if let Some(si) = text_of {
        v["program"] = program_text(doc, job, si);
    }
    v
}

/// One setup of the NC review: how it is run, whether it passes, its
/// programs, and its problems each with its operation.
fn setup_review(doc: &Document, s: &crate::post::SetupPost) -> Value {
    let setup = &doc.setups[s.setup];
    let hand = crate::manual::is_hand(&setup.machine);
    let uc100 = get(&setup.machine.p, "controller") == "UC100";
    let problems_of = |problems: &[String]| -> Vec<Value> {
        (problems.iter().enumerate())
            .map(|(k, p)| {
                // The operation the check recorded, by its id; names can repeat.
                let recorded = s.problem_operations.get(k).and_then(Option::as_ref);
                let (operation, message) = match crate::ui::nc_operation(doc, s.setup, p, recorded)
                {
                    Some((oi, rest)) => (operation_ref(doc, s.setup, oi), rest),
                    None => (Value::Null, p.as_str()),
                };
                json!({
                    "setup": setup_ref(doc, s.setup),
                    "operation": operation,
                    "message": message,
                    "waits_for_earlier": crate::post::is_waiting(p),
                })
            })
            .collect()
    };
    let mut v = json!({
        "setup": setup_ref(doc, s.setup),
        "controller": if hand { "hand" } else if uc100 { "UC100" } else { "LinuxCNC" },
    });
    if hand {
        v["passes"] = json!(true);
        v["programs"] = json!([]);
        v["problems"] = json!([]);
        v["note"] = json!(crate::manual::no_gcode_on(&setup.machine, &s.name));
        return v;
    }
    v["passes"] = json!(s.passes());
    let programs: Vec<Value> = (s.programs().iter())
        .map(|p| {
            json!({
                "file_name": p.file_name,
                "lines": p.lines,
                "tools": p.tools,
                "operations": p.operations,
                "estimated_minutes": (p.seconds / 60. * 10.).round() / 10.,
            })
        })
        .collect();
    v["programs"] = json!(programs);
    v["problems"] = json!(match (&s.run, &s.result) {
        (Some(Err(problems)), _) | (None, Err(problems)) => problems_of(problems),
        _ => vec![],
    });
    if uc100 {
        v["note"] = json!(crate::ui::NO_EXPORT_FILE);
    }
    v
}

/// The G-code of setup `si`'s programs as export writes them, at most
/// `PROGRAM_CAP` bytes in all, each cut at a line's end and marked
/// `truncated` when it is cut; or why the setup has none.
fn program_text(doc: &Document, job: &crate::post::JobPost, si: usize) -> Value {
    let mut v = json!({"setup": setup_ref(doc, si), "cap_bytes": PROGRAM_CAP});
    let Some(s) = job.setups.iter().find(|s| s.setup == si) else {
        v["files"] = json!([]);
        v["why_none"] = json!(
            job.errors
                .first()
                .cloned()
                .unwrap_or_else(|| "The job has no programs.".into())
        );
        return v;
    };
    let mut left = PROGRAM_CAP;
    let files: Vec<Value> = (s.programs().iter())
        .map(|p| {
            let cut = if p.text.len() <= left {
                p.text.len()
            } else {
                // At a line's end within what is left.
                p.text[..p.text.floor_char_boundary(left)]
                    .rfind('\n')
                    .map_or(0, |i| i + 1)
            };
            left -= cut;
            json!({
                "file_name": p.file_name,
                "bytes": p.text.len(),
                "truncated": cut < p.text.len(),
                "text": &p.text[..cut],
            })
        })
        .collect();
    if files.is_empty() {
        let setup = &doc.setups[si];
        v["why_none"] = json!(if crate::manual::is_hand(&setup.machine) {
            crate::manual::no_gcode_on(&setup.machine, &s.name)
        } else if s.run.is_some() {
            crate::ui::NO_EXPORT_FILE.to_string()
        } else {
            "Its checks refuse it: see its problems.".to_string()
        });
    }
    v["files"] = json!(files);
    v
}

/// The steps of `guide` read at `step` (None: the first), with its
/// sections and where the machinist's own place is.
fn steps_of(doc: &Document, guide: &guide::Guide, step: Option<usize>, how: &str) -> Answer {
    let n = guide.steps.len();
    let at = step.unwrap_or(0);
    if at >= n {
        return refused(format!(
            "Setup {} has steps 0 to {}.",
            guide.setup + 1,
            n.saturating_sub(1)
        ));
    }
    let si = guide.setup;
    let sections: Vec<Value> = (guide.sections.iter())
        .map(|s| {
            json!({
                "name": s.name,
                "operation": s.op.map(|oi| operation_ref(doc, si, oi)),
                "first_step": s.steps.start,
                "last_step": s.steps.end.saturating_sub(1),
                "count": s.count,
                "minutes": (s.minutes * 10.).round() / 10.,
            })
        })
        .collect();
    let section = guide.sections.iter().position(|s| s.steps.contains(&at));
    let mut detail = step_detail(doc, si, &guide.steps[at]);
    detail["index"] = json!(at);
    detail["section"] = json!(section.map(|k| &guide.sections[k].name));
    done(json!({
        "setup": setup_ref(doc, si),
        "guide": how,
        "steps": n,
        "minutes": (guide.minutes * 10.).round() / 10.,
        "sections": sections,
        "machinist_place": guide::place(doc, guide),
        "step": detail,
        "next": (at + 1 < n).then_some(at + 1),
        "note": "This reads the steps with a cursor of its own: the machinist's place (machinist_place) is not moved.",
    }))
}

/// What a step says: its kind, its operation, a line of text, on the mill
/// what it cuts (LIO-688), and all it holds (`detail`).
pub(crate) fn step_detail(doc: &Document, si: usize, step: &guide::Step) -> Value {
    use guide::{Depth, Features, Step as S};
    let tool_name = |id: &str| {
        (doc.tools.iter().find(|t| t.id == id)).map_or(id.to_string(), |t| t.name().to_string())
    };
    let r3 = |v: f32| (v * 1000.).round() / 1000.;
    let (kind, op, text, features): (&str, Option<usize>, String, Vec<Value>) = match step {
        S::Zero { stock, vise, .. } => {
            let [x, y, z] = stock.dimensions().map(plan_check::plain);
            let held = match vise {
                Some(v) => format!(
                    ", in the vise across {} gripping {} mm",
                    ["X", "Y"][v.across.min(1)],
                    plan_check::plain(v.grip)
                ),
                None => String::new(),
            };
            // The parallels it stands on, as the Zero page says them
            // (LIO-537).
            let held = match doc.setups.get(si).and_then(crate::parallels::sentence) {
                Some(on) => format!("{held}. {}", on.trim_end_matches('.')),
                None => held,
            };
            // The setup's notes say what holds and supports the part, as
            // the Zero page shows them (LIO-487).
            let notes = (doc.setups.get(si)).map_or("", |s| get(&s.p, "notes").trim());
            let notes = if notes.is_empty() {
                String::new()
            } else {
                format!(" Setup notes: {notes}")
            };
            (
                "Zero",
                None,
                format!(
                    "The {x} × {y} × {z} mm stock{held}. The work zero: the stock runs from X {} to {}, Y {} to {}, Z {} to {}.{notes}",
                    plan_check::plain(stock.min[0]),
                    plan_check::plain(stock.max[0]),
                    plan_check::plain(stock.min[1]),
                    plan_check::plain(stock.max[1]),
                    plan_check::plain(stock.min[2]),
                    plan_check::plain(stock.max[2]),
                ),
                vec![],
            )
        }
        S::Sheet {
            op,
            tool,
            rpm,
            features,
        } => {
            let speed = match rpm {
                Some(r) => format!("at {} rpm", plan_check::plain(*r)),
                None => "by hand, the spindle off".into(),
            };
            let (what, listed): (String, Vec<Value>) = match features {
                Features::Holes {
                    diameter,
                    thread,
                    holes,
                } => (
                    format!(
                        "{} hole(s), {}",
                        holes.len(),
                        thread
                            .clone()
                            .unwrap_or_else(|| format!("Ø{}", plan_check::plain(*diameter)))
                    ),
                    (holes.iter().enumerate())
                        .map(|(k, h)| {
                            let (deep, tip) = match h.depth {
                                Depth::Through { tip } => (None, tip),
                                Depth::Blind { deep, tip } => (Some(r3(deep)), tip),
                            };
                            json!({"n": k + 1, "x": r3(h.at[0]), "y": r3(h.at[1]), "mouth": r3(h.mouth),
                                "through": deep.is_none(), "deep": deep, "tip": r3(tip)})
                        })
                        .collect(),
                ),
                Features::Box { lo, hi, through } => (
                    "a box".into(),
                    vec![json!({"n": 1, "min": lo.map(r3), "max": hi.map(r3), "through": through})],
                ),
                Features::Slot {
                    a,
                    b,
                    width,
                    top,
                    floor,
                    through,
                } => (
                    "a slot".into(),
                    vec![json!({"n": 1, "a": a.map(r3), "b": b.map(r3), "width": r3(*width),
                        "top": r3(*top), "floor": r3(*floor), "through": through})],
                ),
                Features::Corners(c) => (
                    format!("{} corner(s)", c.len()),
                    (c.iter().enumerate())
                        .map(|(k, c)| {
                            json!({"n": k + 1, "place": c.place, "centre": c.centre.map(r3),
                                "radius": r3(c.radius), "top": r3(c.top), "bottom": r3(c.bottom)})
                        })
                        .collect(),
                ),
            };
            (
                "Sheet",
                Some(*op),
                format!("With {} {speed}: {what}.", tool_name(tool)),
                listed,
            )
        }
        S::HandOver { next } => (
            "HandOver",
            None,
            match next {
                Some(n) => format!("The stock goes on to setup {}.", n + 1),
                None => "The setup is done.".into(),
            },
            vec![],
        ),
        S::Lathe(l) => (l.title(), lathe_op(l), format!("{}.", l.title()), vec![]),
    };
    json!({
        "kind": kind,
        "operation": op.map(|oi| operation_ref(doc, si, oi)),
        "text": text,
        "features": features,
        "detail": format!("{step:?}"),
    })
}
/// The operation a lathe step is of, when it is one's.
fn lathe_op(l: &guide::LatheStep) -> Option<usize> {
    // Every lathe step of an operation holds it as its first field, `op`:
    // read from what the step says rather than listing each kind.
    let said = format!("{l:?}");
    let (_, rest) = said.split_once(" { op: ")?;
    rest.chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .ok()
}

/// The read tools as `tools/list` lists them.
pub fn tools() -> Vec<Value> {
    let wait = json!({
        "type": "number",
        "minimum": 0,
        "maximum": super::engine::MAX_WAIT,
        "description": "How long to wait for the result, s (default 25, at most 50); after that the answer is still_running with a command_id for wait.",
    });
    let setup = json!({"type": ["string", "integer"], "description": "The setup's id, or its index from 0; default the active setup."});
    let addressed = |what: &str| json!({"type": ["string", "integer"], "description": format!("The {what}'s id, or its index from 0 in the active setup.")});
    let heavy = " It simulates the job's toolpaths: while the machine runs it is refused, and Cycle Start waits for it.";
    let fresh = " The result carries `checked_at` (revision and fingerprint) and `stale`: true, with `changed_meanwhile`, when the job changed while it worked.";
    vec![
        json!({
            "name": "check_job",
            "title": "Check the job as it is",
            "description": format!("Runs the plan checks (generation, gouges, cutting data, workholding, export) on the job exactly as the user sees it: nothing is filled in or suggested. A setup with clamps but no positions, or a vise or chuck with no grip, is a problem. Answers `passed`, `problems` by setup and operation (numbered from 1, with their ids), `operations` with their minutes, `coverage`, `total_minutes` and `stocks`, the stock before each setup.{heavy} If the job changes while it runs it is refused as stale: check again."),
            "inputSchema": {"type": "object", "properties": {"wait_s": wait}},
            "annotations": {"readOnlyHint": true},
        }),
        json!({
            "name": "check_holding",
            "title": "Check a setup's holding as saved",
            "description": format!("The setup's holder and placement exactly as saved (a vise's axis, offset and grip; the clamps' positions; a lathe chuck's grip and stickout), checked as check_job checks them on the stock the setup starts from: `holds` and the `problems`. Nothing is substituted: a missing or unreadable placement is a problem. Only suggest_layout suggests one, and it changes the job.{heavy}{fresh}"),
            "inputSchema": {"type": "object", "properties": {"setup": setup, "wait_s": wait}},
            "annotations": {"readOnlyHint": true},
        }),
        json!({
            "name": "stock_after",
            "title": "The stock a setup starts from",
            "description": format!("The stock a setup starts from, as the earlier setups leave it, in that setup's axes: its `size`, its box from the part's -X/-Y/bottom corner, what the earlier setups `removed` from each side in mm, and `by` which operations.{heavy}{fresh}"),
            "inputSchema": {"type": "object", "properties": {"setup": setup, "wait_s": wait}},
            "annotations": {"readOnlyHint": true},
        }),
        json!({
            "name": "cutting_data",
            "title": "Speeds and feeds for a tool",
            "description": "The spindle speed and feed a milling tool can run at in the project's material on the machine of `op` (else the active setup's): the rpm range from the material's surface speed and the machine's maximum, a recommended rpm, and at that rpm the feed range (mm/min) from the chip-load band of the tool's diameter and its flutes, with a recommended feed. With `op`, the operation's own rpm and feed beside them.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "tool": {"type": ["string", "integer"], "description": "The tool's id, or its number; default the operation's own tool."},
                    "op": addressed("operation"),
                },
            },
            "annotations": {"readOnlyHint": true},
        }),
        json!({
            "name": "surface_info",
            "title": "One surface of the part",
            "description": format!("What the part report and the part say of one surface: every orientation whose planes list it (kind, depth below the top, area, bbox, the largest cutter that fits, holes), its own area in mm² and its mean normal in setup 1's frame.{fresh}"),
            "inputSchema": {
                "type": "object",
                "properties": {"id": {"type": "integer", "minimum": 0, "description": "A surface id from list_surfaces."}, "wait_s": wait},
                "required": ["id"],
            },
            "annotations": {"readOnlyHint": true},
        }),
        json!({
            "name": "part_report",
            "title": "The part report",
            "description": format!("The part as the checks read it, for setup 1: its size, the material and its cutting data, the machine's limits, the tools by number, the workholding library and setup 1's holding, and for each orientation its stock margins and its horizontal planes (surface ids, depth, area, holes, the largest cutter that fits).{fresh}"),
            "inputSchema": {"type": "object", "properties": {"wait_s": wait}},
            "annotations": {"readOnlyHint": true},
        }),
        json!({
            "name": "list_surfaces",
            "title": "List the part's surfaces",
            "description": format!("The part's surfaces, the largest first: id (what pick_surfaces takes), normal, centre, area and the smooth region it belongs to, with the part report's planes from the setup's orientation. On the built-in sample, its features instead, which pick_features takes.{fresh}"),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "setup": setup,
                    "limit": {"type": "integer", "minimum": 1, "maximum": MOST_SURFACES, "description": format!("How many surfaces to list, the largest first (default {SURFACES}).")},
                    "wait_s": wait,
                },
            },
            "annotations": {"readOnlyHint": true},
        }),
        json!({
            "name": "view_model",
            "title": "Look at the part",
            "description": format!("A picture of the part (PNG) with a short description: from a view angle, in an orientation, zoomed to surfaces or with them highlighted, with the stock and the holding, cut away on a section, or for an `operation` its toolpaths and the stock left after it (that one simulates the job: refused while the machine runs).{fresh}"),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "view": {"type": "string", "enum": look::ANGLES.map(|a| a.0), "description": "The angle to look from (default iso_front_left)."},
                    "orientation": {"type": "string", "enum": analysis::ORIENTATIONS.map(|o| o.0), "description": "The part axis to draw pointing up; default setup 1's. Ignored with `operation`, which is drawn in its own setup."},
                    "focus_surfaces": {"type": "array", "items": {"type": "integer"}, "description": "Zoom to these surface ids."},
                    "highlight_surfaces": {"type": "array", "items": {"type": "integer"}, "description": "Surface ids drawn in blue."},
                    "show_stock": {"type": "boolean", "description": "Default true."},
                    "show_workholding": {"type": "boolean", "description": "Default true."},
                    "labels": {"type": "boolean", "description": "Tags with each plane's surface ids and each hole's diameter; default true."},
                    "operation": addressed("operation"),
                    "section": {
                        "type": "object",
                        "properties": {
                            "axis": {"type": "string", "enum": ["X", "Y", "Z"]},
                            "surface": {"type": "integer"},
                            "at": {"type": "number", "description": "With axis: mm from the part's -X side, -Y side or bottom. With surface: mm along its outward normal, below 0 into the part."},
                            "cut_away": {"type": "string", "enum": ["positive", "negative"]},
                        },
                        "required": ["at", "cut_away"],
                        "description": "A cut-away view on a plane square to `axis`, or through `surface`.",
                    },
                    "purpose": {"type": "string"},
                    "wait_s": wait,
                },
            },
            "annotations": {"readOnlyHint": true},
        }),
        json!({
            "name": "guide",
            "title": "The machining rules",
            "description": format!("The machining rules the checks hold a plan to, by topic: {}. With no topic, the list of topics.", rules::TOPICS.map(|t| t.0).join(", ")),
            "inputSchema": {
                "type": "object",
                "properties": {"topic": {"type": "string", "enum": rules::TOPICS.map(|t| t.0)}},
            },
            "annotations": {"readOnlyHint": true},
        }),
    ]
}

/// Whether a read is heavy, for `describe_command`: always, sometimes or
/// never.
pub fn heaviness(tool: &str) -> &'static str {
    match tool {
        "check_job" | "check_holding" | "stock_after" => "heavy",
        "view_model" => "heavy with operation, light otherwise",
        "inspect" => {
            "light; heavy with what: post when the app's own NC review check is not of the job as it is now, and with what: steps when the guide has to be built"
        }
        _ => "light",
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{
        Message,
        mcp::commands::tests::{Direct, direct},
        model::ModelFile,
    };
    use std::time::{Duration, Instant};

    /// A tool call, answered: a read's worker answers on its own thread, so
    /// the agent is polled until it has.
    fn ask(d: &mut Direct, tool: &str, args: Value) -> Value {
        let args_wait = args.get("wait_s").cloned();
        let mut agent = d.agent.take().unwrap();
        let answer = Arc::new(std::sync::Mutex::new(None));
        let into = answer.clone();
        agent.call(
            d,
            tool,
            args,
            Box::new(move |a| *into.lock().unwrap() = Some(a)),
        );
        let until = Instant::now() + Duration::from_secs(600);
        let got = loop {
            if let Some(a) = answer.lock().unwrap().take() {
                break a;
            }
            assert!(Instant::now() < until, "no answer to {tool}");
            agent.poll(d);
            std::thread::sleep(Duration::from_millis(5));
        };
        d.agent = Some(agent);
        let got = got.unwrap_or_else(|e| panic!("{tool}: {e}"));
        // Building a guide in a debug build may take longer than a call's
        // time: wait on, unless the test chose the time.
        if got["outcome"] == "still_running" && (args_wait.is_none() || tool == "wait") {
            let id = got["command_id"].clone();
            return ask(d, "wait", json!({"command_id": id, "wait_s": 50}));
        }
        got
    }
    /// `doc` and its part as the job of `d`, as imported.
    fn install(d: &mut Direct, mut doc: Document, mesh: Mesh) {
        if doc.file.is_none() {
            doc.file = Some(ModelFile {
                name: "part.3mf".into(),
                path: "part.3mf".into(),
                format: "3mf".into(),
                bytes: 1,
                thickness: None,
            });
        }
        doc.active = doc.setups[0].id.clone();
        d.app.replace_document(doc, None);
        d.app.imported_mesh = Some(mesh);
        d.app.selected = Some(0);
        d.app.dirty = false;
    }
    /// The example block with its Face alone, its setup held as `edit`
    /// holds it.
    fn block(edit: impl FnOnce(&mut Document)) -> Direct {
        let mut d = direct();
        let (mut doc, mesh, _) = crate::proposal::tests::block();
        doc.setups[0].operations.truncate(1);
        edit(&mut doc);
        install(&mut d, doc, mesh);
        d
    }
    fn vise(doc: &mut Document, offset: &str) {
        crate::proposal::tests::hold(
            doc,
            "Vise",
            &[
                ("vise_axis", "Y"),
                ("vise_grip", "8"),
                ("vise_offset", offset),
            ],
        );
    }
    fn messages(v: &Value) -> Vec<String> {
        (v["problems"].as_array().unwrap().iter())
            .map(|p| p["message"].as_str().unwrap().to_string())
            .collect()
    }
    /// The lathe's mockup sleeve on its 3-jaw.
    fn lathe() -> Direct {
        let mut d = direct();
        let (doc, mesh) = crate::guide::tests::sample("Diameter");
        install(&mut d, doc, mesh);
        d
    }
    /// The manual mill's job with its guide built, as its Steps page built
    /// it; then on the Work page, where no `Tick` builds it again.
    fn manual() -> Direct {
        let mut d = direct();
        d.app = crate::steps::tests::app("No");
        // The page's own build, whose task the test dropped, is over.
        d.app.guide_job = None;
        d.app.page = crate::Page::Work;
        d.app.dirty = false;
        d
    }
    /// A simulated UC100 added to `d`'s library, connected, referenced and
    /// jogging +X: the machine runs (`App::locked`) until `stop`.
    pub(crate) fn jogging(d: &mut Direct) {
        use crate::control::Action;
        use uc100::controller::State;
        if d.app.control.is_some() {
            let _ = d.app.update(Message::Machine(Action::JogPress(0, false)));
            tick_until(&mut d.app, "jogging", |s| s.state == Some(State::Jog));
            assert!(d.app.locked());
            return;
        }
        let profile = d.folder.join("uc100.toml");
        std::fs::write(&profile, uc100::config::TEMPLATE).unwrap();
        let mut m = crate::model::Machine::new(3);
        m.p.insert("name".into(), "JY5800".into());
        m.p.insert("controller".into(), "UC100".into());
        m.p.insert("uc100_profile".into(), profile.display().to_string());
        m.p.insert("uc100_connection".into(), "Simulated".into());
        d.app.doc.machines.push(m);
        let _ = d.app.update(Message::Connect);
        tick_until(&mut d.app, "E-stop", |s| s.state == Some(State::EStop));
        let _ = d.app.update(Message::MachineReset);
        tick_until(&mut d.app, "Idle", |s| s.state == Some(State::Idle));
        let _ = d.app.update(Message::Machine(Action::MachineZero(None)));
        tick_until(&mut d.app, "referenced", |s| {
            s.referenced[..3].iter().all(|r| *r)
        });
        let _ = d.app.update(Message::Machine(Action::JogPress(0, false)));
        tick_until(&mut d.app, "jogging", |s| s.state == Some(State::Jog));
        assert!(d.app.locked());
    }
    fn tick_until(app: &mut App, what: &str, ok: impl Fn(&uc100::controller::Snapshot) -> bool) {
        let end = Instant::now() + Duration::from_secs(60);
        loop {
            let _ = app.update(Message::Tick);
            if ok(&app.control.as_ref().expect("connected").snapshot) {
                return;
            }
            assert!(Instant::now() < end, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    /// The jog ends: the machine stops.
    pub(crate) fn stop(d: &mut Direct) {
        use uc100::controller::State;
        let _ = d
            .app
            .update(Message::Machine(crate::control::Action::JogRelease));
        tick_until(&mut d.app, "Idle", |s| s.state == Some(State::Idle));
        assert!(!d.app.locked());
    }

    /// The review's probe: a vise saved 10 000 mm off the stock passed the
    /// planner's tool on a suggestion of its own. Here the saved placement
    /// is checked as it is, and fails; placed on the stock it holds; clamps
    /// with no positions fail check_holding and check_job.
    #[test]
    fn check_holding_takes_a_mill_setups_placement_as_saved() {
        let mut d = block(|doc| vise(doc, "10000"));
        let off = ask(&mut d, "check_holding", json!({}));
        assert_eq!(off["outcome"], "done", "{off}");
        assert_eq!(off["holds"], false, "{off}");
        assert_eq!(off["placement"]["vise_offset"], "10000", "{off}");
        assert!(!messages(&off).is_empty(), "{off}");
        assert_eq!(off["stale"], false);

        let mut d = block(|doc| vise(doc, "0"));
        let on = ask(&mut d, "check_holding", json!({"setup": 0}));
        assert_eq!(on["holds"], true, "{on}");
        assert_eq!(on["jaws_on"], "±Y", "{on}");
        assert_eq!(on["setup"]["id"], d.app.doc.setups[0].id.as_str());
        assert!(on["checked_at"]["fingerprint"].is_string(), "{on}");

        let mut d = block(|doc| crate::proposal::tests::hold(doc, "Clamp", &[]));
        let before = serde_json::to_string(&d.app.doc).unwrap();
        let none = ask(&mut d, "check_holding", json!({}));
        assert_eq!(none["holds"], false, "{none}");
        assert_eq!(
            messages(&none),
            ["Setup 1 has no clamp positions: use suggest_layout or set_clamps."]
        );
        let check = ask(&mut d, "check_job", json!({}));
        assert_eq!(check["outcome"], "done", "{check}");
        assert_eq!(check["passed"], false, "{check}");
        let problem = (check["problems"].as_array().unwrap().iter())
            .find(|p| p["stage"] == "workholding")
            .unwrap_or_else(|| panic!("{check}"));
        assert_eq!(
            problem["message"],
            "Setup 1 has no clamp positions: use suggest_layout or set_clamps."
        );
        assert_eq!(problem["setup_id"], d.app.doc.setups[0].id.as_str());
        assert!(check["checked_at"]["revision"].is_u64(), "{check}");
        // Nothing was filled in.
        assert_eq!(serde_json::to_string(&d.app.doc).unwrap(), before);
        assert_eq!(d.app.mcp.heavy.running(), 0);
    }

    /// A lathe setup's chuck as saved: its grip and stickout hold the
    /// sleeve; with no stickout it is a missing placement.
    #[test]
    fn check_holding_takes_a_lathe_chuck_as_saved() {
        let mut d = lathe();
        let grip = get(&d.app.doc.setups[0].p, "grip").to_string();
        assert_eq!(get(&d.app.doc.setups[0].p, "holding"), "Chuck");
        let held = ask(&mut d, "check_holding", json!({}));
        assert_eq!(held["holds"], true, "{held}");
        assert_eq!(held["placement"]["grip"], grip.as_str(), "{held}");
        d.app.doc.setups[0]
            .p
            .insert("stickout".into(), String::new());
        let none = ask(&mut d, "check_holding", json!({}));
        assert_eq!(none["holds"], false, "{none}");
        assert!(
            messages(&none)[0].contains("no chuck grip or stickout"),
            "{none}"
        );
    }

    /// A heavy read started before Cycle Start keeps it waiting
    /// (`planning_busy`) until its worker has ended, cancelled or not.
    #[test]
    fn a_heavy_reads_worker_keeps_cycle_start_waiting_until_it_ends() {
        let mut d = block(|doc| vise(doc, "0"));
        assert_eq!(d.app.planning_busy(), None);
        d.app.mcp.slow_reads = Some(Duration::from_millis(1500));
        let first = ask(&mut d, "stock_after", json!({"setup": 0, "wait_s": 0.1}));
        assert_eq!(first["outcome"], "still_running", "{first}");
        let id = first["command_id"].as_str().unwrap().to_string();
        let busy = Some("An agent's check is still simulating the job: wait for it.");
        assert_eq!(d.app.planning_busy(), busy);
        let cancelled = ask(&mut d, "cancel", json!({"command_id": id}));
        assert_eq!(cancelled["outcome"], "cancelled", "{cancelled}");
        assert_eq!(d.app.planning_busy(), busy, "the worker still runs");
        let again = ask(&mut d, "wait", json!({"command_id": id, "wait_s": 0}));
        assert_eq!(again["outcome"], "cancelled", "{again}");
        let end = Instant::now() + Duration::from_secs(120);
        while d.app.planning_busy().is_some() {
            assert!(Instant::now() < end, "the worker never ended");
            std::thread::sleep(Duration::from_millis(20));
        }
        // A light read is not counted.
        d.app.mcp.slow_reads = None;
        let guide = ask(&mut d, "guide", json!({"topic": "holding"}));
        assert!(guide["rules"].as_str().unwrap().contains("vise"));
        // Waited for, a heavy read answers in full.
        let stock = ask(&mut d, "stock_after", json!({"setup": 0}));
        assert_eq!(stock["outcome"], "done", "{stock}");
        assert!(stock["size"][2].as_f64().unwrap() > 0., "{stock}");
    }

    /// While the machine runs, the heavy reads are refused and the light
    /// ones answer: inspect, the rules, speeds and feeds, the surfaces and
    /// a picture of the part.
    #[test]
    fn heavy_reads_wait_for_the_machine_and_light_ones_answer() {
        let mut d = block(|doc| vise(doc, "0"));
        jogging(&mut d);
        for (tool, args) in [
            ("check_job", json!({})),
            ("check_holding", json!({})),
            ("stock_after", json!({"setup": 0})),
            ("view_model", json!({"operation": 0})),
        ] {
            let r = ask(&mut d, tool, args);
            assert_eq!(
                (r["outcome"].as_str(), r["reason"].as_str()),
                (Some("refused"), Some(LOCKED)),
                "{tool}: {r}"
            );
        }
        assert_eq!(d.app.mcp.heavy.running(), 0);
        let state = ask(&mut d, "inspect", json!({}));
        assert!(state["setups"].is_array(), "{state}");
        let tool = d.app.doc.tools[0].id.clone();
        let speeds = ask(&mut d, "cutting_data", json!({"tool": tool, "op": 0}));
        assert_eq!(speeds["outcome"], "done", "{speeds}");
        assert!(
            speeds["rpm"]["recommended"].as_u64().unwrap() >= 100,
            "{speeds}"
        );
        assert_eq!(speeds["operation"]["index"], 0, "{speeds}");
        // Without a tool, the operation's own.
        let own = ask(&mut d, "cutting_data", json!({"op": 0}));
        let op_tool = d.app.doc.setups[0].operations[0].tool.clone();
        assert_eq!(own["tool"]["id"], op_tool.as_str(), "{own}");
        let listed = ask(&mut d, "list_surfaces", json!({"limit": 5}));
        assert_eq!(listed["outcome"], "done", "{listed}");
        let picture = ask(&mut d, "view_model", json!({"view": "top"}));
        assert_eq!(picture["outcome"], "done", "{picture}");
        stop(&mut d);
        let check = ask(&mut d, "check_job", json!({}));
        assert_eq!(check["outcome"], "done", "{check}");
    }

    /// The picture comes back as PNG, and goes to the client as an MCP
    /// image item beside its description.
    #[test]
    fn view_model_returns_a_png() {
        use base64::Engine;
        let mut d = direct();
        let picture = ask(&mut d, "view_model", json!({"view": "iso_front_left"}));
        assert_eq!(picture["outcome"], "done", "{picture}");
        let png = base64::engine::general_purpose::STANDARD
            .decode(picture[IMAGE].as_str().unwrap())
            .unwrap();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        let sent = crate::mcp::protocol::tool_result(Ok(picture));
        assert_eq!(sent["content"][0]["type"], "image");
        assert_eq!(sent["content"][0]["mimeType"], "image/png");
        assert_eq!(sent["content"][1]["type"], "text");
        assert!(sent["structuredContent"].get(IMAGE).is_none());
        assert!(sent["structuredContent"]["description"].is_string());
        // A view that cannot be drawn says why.
        let bad = ask(&mut d, "view_model", json!({"view": "sideways"}));
        assert_eq!(bad["outcome"], "refused", "{bad}");
    }

    /// The surfaces of an imported part, with the planes of the setup's
    /// orientation; the built-in sample's features; one surface's info, the
    /// part report and the rules.
    #[test]
    fn the_part_is_read_by_its_surfaces() {
        let mut d = block(|_| {});
        let listed = ask(&mut d, "list_surfaces", json!({}));
        assert_eq!(listed["up"], "+Z", "{listed}");
        let surfaces = listed["surfaces"].as_array().unwrap();
        assert!(!surfaces.is_empty() && listed["total"].as_u64().unwrap() >= surfaces.len() as u64);
        let areas: Vec<f64> = surfaces
            .iter()
            .map(|s| s["area"].as_f64().unwrap())
            .collect();
        assert!(areas.windows(2).all(|w| w[0] >= w[1]), "the largest first");
        assert!(!listed["planes"].as_array().unwrap().is_empty(), "{listed}");
        let top = listed["planes"][0]["surfaces"][0].as_u64().unwrap();
        let info = ask(&mut d, "surface_info", json!({"id": top}));
        assert_eq!(info["outcome"], "done", "{info}");
        assert_eq!(info["orientations"][0]["up"], "+Z", "{info}");
        let report = ask(&mut d, "part_report", json!({}));
        assert!(report["report"]["orientations"].is_array(), "{report}");
        let missing = ask(&mut d, "surface_info", json!({"id": 1_000_000}));
        assert_eq!(missing["outcome"], "refused", "{missing}");

        let mut sample = direct();
        let features = ask(&mut sample, "list_surfaces", json!({}));
        assert_eq!(features["sample"], true, "{features}");
        assert_eq!(
            features["features"].as_array().unwrap().len(),
            crate::model::FEATURES.len()
        );
        let topics = ask(&mut sample, "guide", json!({}));
        assert_eq!(
            topics["topics"].as_array().unwrap().len(),
            rules::TOPICS.len()
        );
        for (topic, ..) in rules::TOPICS {
            let rules = ask(&mut sample, "guide", json!({"topic": topic}));
            assert!(!rules["rules"].as_str().unwrap().is_empty(), "{topic}");
        }
    }

    /// The steps of a manual mill setup and a lathe setup, read with a
    /// cursor of their own: a guide not built is built (heavy: refused while
    /// the machine runs), a guide built for the job as it is is read at once,
    /// during a run too. The machinist's place is never moved.
    #[test]
    fn inspect_reads_the_steps_of_a_hand_setup_without_moving_the_place() {
        // The manual mill, its guide built by the Steps page.
        let mut d = manual();
        let place = d.app.doc.guide.clone();
        let si = d.app.doc.index();
        assert!(d.app.current_guide().is_some());
        let cached = ask(&mut d, "inspect", json!({"what": "steps", "step": 1}));
        assert_eq!(cached["outcome"], "done", "{cached}");
        assert_eq!(cached["guide"], "cached", "{cached}");
        assert_eq!(cached["step"]["index"], 1, "{cached}");
        // Any step, with an earlier operation selected: the Holes' page.
        d.app.selected = Some(0);
        let n = cached["steps"].as_u64().unwrap() as usize;
        let step = |d: &mut Direct, k: usize| {
            ask(
                d,
                "inspect",
                json!({"what": "steps", "setup": si, "step": k}),
            )
        };
        let holes = (0..n)
            .find(|&k| {
                let s = step(&mut d, k);
                s["step"]["kind"] == "Sheet" && s["step"]["text"].as_str().unwrap().contains("hole")
            })
            .expect("the Holes' page");
        let beyond = step(&mut d, n);
        assert_eq!(beyond["outcome"], "refused", "{beyond}");
        assert_eq!(d.app.doc.guide, place);
        // Not built for the job as it is: built on a snapshot.
        d.app.guide = None;
        let at = step(&mut d, holes);
        assert_eq!(at["guide"], "built", "{at}");
        assert_eq!(at["step"]["kind"], "Sheet", "{at}");
        // Each hole with its place and depth, as the page lists them
        // (LIO-688).
        let holes_read = at["step"]["features"].as_array().unwrap();
        assert!(!holes_read.is_empty(), "{at}");
        assert!(
            holes_read
                .iter()
                .all(|h| h["x"].is_number() && h["tip"].is_number()),
            "{at}"
        );
        assert!(at["step"]["operation"]["id"].is_string(), "{at}");
        assert_eq!(d.app.doc.guide, place);
        assert!(d.app.guide.is_none(), "the app's guide is not touched");
        // While the machine runs: built, refused; cached, read.
        jogging(&mut d);
        let refused = ask(&mut d, "inspect", json!({"what": "steps"}));
        assert_eq!(refused["reason"], LOCKED, "{refused}");
        assert_eq!(d.app.mcp.heavy.running(), 0);
        crate::steps::tests::build(&mut d.app);
        let read = step(&mut d, holes);
        assert_eq!(read["guide"], "cached", "{read}");
        assert_eq!(read["step"], at["step"]);
        stop(&mut d);
        assert_eq!(d.app.doc.guide, place);
        // The lathe: not built, then built.
        let mut d = lathe();
        let place = d.app.doc.guide.clone();
        jogging(&mut d);
        let refused = ask(&mut d, "inspect", json!({"what": "steps"}));
        assert_eq!(refused["reason"], LOCKED, "{refused}");
        stop(&mut d);
        let built = ask(&mut d, "inspect", json!({"what": "steps", "step": 2}));
        assert_eq!(built["guide"], "built", "{built}");
        assert!(built["step"]["kind"].is_string(), "{built}");
        crate::steps::tests::build(&mut d.app);
        jogging(&mut d);
        let read = ask(&mut d, "inspect", json!({"what": "steps", "step": 2}));
        assert_eq!(read["guide"], "cached", "{read}");
        assert_eq!(read["step"], built["step"]);
        stop(&mut d);
        assert_eq!(d.app.doc.guide, place);
        // A CNC setup has no steps.
        let mut d = block(|_| {});
        let cnc = ask(&mut d, "inspect", json!({"what": "steps"}));
        assert_eq!(cnc["outcome"], "refused", "{cnc}");
    }

    /// The tools are listed with schemas, and described.
    #[test]
    fn the_reads_are_listed_and_described() {
        let listed = crate::mcp::engine::tools();
        for name in TOOLS {
            let tool = (listed.as_array().unwrap().iter())
                .find(|t| t["name"] == name)
                .unwrap_or_else(|| panic!("{name}"));
            assert_eq!(tool["inputSchema"]["type"], "object", "{name}");
            assert!(crate::mcp::engine::is_tool(name));
        }
        assert!(crate::mcp::protocol::INSTRUCTIONS.contains("check_job"));
    }

    /// The example block's job of the post fixture, its Face alone.
    fn faced() -> Direct {
        let mut d = direct();
        let (mut doc, mesh) = crate::post::fixture::job();
        doc.setups[0].operations.truncate(1);
        install(&mut d, doc, mesh);
        d
    }

    /// The NC review (LIO-470): what the app's NC review page has, read
    /// only. Generated, the job's program is listed as export names it,
    /// with no problem; the app's own check is read once the page has run
    /// it; `program: true` gives what export_gcode writes; a missing tool is
    /// a problem of its operation.
    #[test]
    fn inspect_reads_the_nc_review() {
        let mut d = faced();
        let all = ask(&mut d, "execute", json!({"command": "generate_all"}));
        assert_eq!(all["outcome"], "done", "{all}");
        // The page not open: worked out for the call.
        let review = ask(&mut d, "inspect", json!({"what": "post"}));
        assert_eq!(review["outcome"], "done", "{review}");
        assert_eq!(review["source"], "worked_out", "{review}");
        assert_eq!(review["app_check"], "not_current", "{review}");
        assert_eq!(
            (&review["current"], &review["stale"]),
            (&json!(true), &json!(false))
        );
        assert!(review["command_id"].is_string(), "{review}");
        assert_eq!(review["ready"], true, "{review}");
        assert_eq!(review["errors"], json!([]), "{review}");
        let setup = &review["setups"][0];
        assert_eq!(setup["setup"]["id"], d.app.doc.setups[0].id.as_str());
        assert_eq!(
            (&setup["controller"], &setup["passes"], &setup["problems"]),
            (&json!("LinuxCNC"), &json!(true), &json!([])),
            "{setup}"
        );
        let program = &setup["programs"][0];
        let posted = crate::post::post_job(&d.app.doc, d.app.part_mesh().unwrap());
        let expected = &posted.setups[0].programs()[0];
        assert_eq!(
            program["file_name"],
            crate::post::file_name(&d.app.doc, 0, None)
        );
        assert_eq!(program["lines"], expected.lines, "{program}");
        assert_eq!(program["tools"], json!(expected.tools), "{program}");
        assert_eq!(program["operations"], json!(["Face"]), "{program}");
        assert!(
            program["estimated_minutes"].as_f64().unwrap() > 0.,
            "{program}"
        );
        assert_eq!(d.app.mcp.heavy.running(), 0);

        // The page open: the app's own check, read as it has it.
        let page = ask(
            &mut d,
            "execute",
            json!({"command": "goto_page", "args": {"page": "Post"}}),
        );
        assert_eq!(page["outcome"], "done", "{page}");
        assert!(d.app.post_current(), "the page checked the job");
        let app = ask(&mut d, "inspect", json!({"what": "post"}));
        assert_eq!(
            (&app["source"], &app["app_check"], &app["current"]),
            (&json!("app"), &json!("current"), &json!(true)),
            "{app}"
        );
        assert_eq!(app["setups"], review["setups"]);

        // The program as export_gcode writes it.
        let folder = tempfile::tempdir().unwrap();
        let export = json!({"command": "export_gcode", "args": {"folder": folder.path().display().to_string()}});
        let written = ask(&mut d, "execute", export);
        assert_eq!(written["outcome"], "done", "{written}");
        let text = ask(
            &mut d,
            "inspect",
            json!({"what": "post", "setup": 0, "program": true}),
        );
        let file = &text["program"]["files"][0];
        let on_disk =
            std::fs::read_to_string(folder.path().join(file["file_name"].as_str().unwrap()))
                .unwrap();
        assert_eq!(file["text"], on_disk.as_str());
        assert_eq!(file["truncated"], false);
        assert_eq!(file["bytes"], on_disk.len());

        // Its tool gone from the library: the Face's problem.
        let mut doc = d.app.doc.clone();
        doc.setups[0].operations[0].tool = "no-such-tool".into();
        let mesh = d.app.part_mesh().unwrap().clone();
        install(&mut d, doc, mesh);
        let broken = ask(&mut d, "inspect", json!({"what": "post"}));
        assert_eq!(broken["ready"], false, "{broken}");
        let setup = &broken["setups"][0];
        assert_eq!(setup["passes"], false, "{setup}");
        assert_eq!(setup["programs"], json!([]), "{setup}");
        let problem = &setup["problems"][0];
        assert_eq!(problem["operation"]["index"], 0, "{problem}");
        assert_eq!(
            problem["operation"]["id"],
            d.app.doc.setups[0].operations[0].id.as_str()
        );
        assert_eq!(
            problem["message"], "Choose a tool from the library.",
            "{problem}"
        );
        let none = ask(&mut d, "inspect", json!({"what": "post", "program": true}));
        assert_eq!(none["program"]["files"], json!([]), "{none}");
        assert!(none["program"]["why_none"].is_string(), "{none}");
    }

    /// Two operations of one setup both named "Pocket", only the second
    /// without its cutter: the problem is the second's, by its id (names
    /// repeat), and the agent's repair of that id clears it (LIO-470).
    #[test]
    fn an_nc_problem_names_its_operation_by_id_when_two_share_a_name() {
        let mut d = direct();
        let (mut doc, mesh) = crate::post::fixture::job();
        doc.setups[0].operations.truncate(1);
        let tool = doc.setups[0].operations[0].tool.clone();
        let mut pockets = vec![];
        for id in ["pocket-one", "pocket-two"] {
            let mut o = doc.setups[0].operations[0].clone();
            o.id = id.into();
            o.p.insert("name".into(), "Pocket".into());
            pockets.push(o);
        }
        pockets[1].tool = "no-such-tool".into();
        doc.setups[0].operations = pockets;
        install(&mut d, doc, mesh);
        let broken = ask(&mut d, "inspect", json!({"what": "post"}));
        let problems = &broken["setups"][0]["problems"];
        assert_eq!(problems.as_array().map(Vec::len), Some(1), "{broken}");
        assert_eq!(problems[0]["operation"]["id"], "pocket-two", "{broken}");
        assert_eq!(problems[0]["operation"]["index"], 1, "{broken}");
        assert_eq!(
            problems[0]["message"], "Choose a tool from the library.",
            "{broken}"
        );
        let fixed = ask(
            &mut d,
            "execute",
            json!({"command": "set_operation_tool", "args": {"operation": "pocket-two", "id": tool}}),
        );
        assert_eq!(fixed["outcome"], "done", "{fixed}");
        // The page's check runs again for the job as it is now.
        let page = ask(
            &mut d,
            "execute",
            json!({"command": "goto_page", "args": {"page": "Post"}}),
        );
        assert_eq!(page["outcome"], "done", "{page}");
        let after = ask(&mut d, "inspect", json!({"what": "post"}));
        assert_eq!(after["setups"][0]["problems"], json!([]), "{after}");
        assert_eq!(after["setups"][0]["passes"], true, "{after}");
    }

    /// The NC review of a job changed since its check: the app's own check
    /// is not current (or still running) and is not read; one changed
    /// while it was worked out says so (`current: false`, `stale`); while
    /// the machine runs one to work out is refused, as a heavy read.
    #[test]
    fn an_nc_review_of_a_job_changed_since_says_it_is_not_current() {
        let mut d = faced();
        let page = ask(
            &mut d,
            "execute",
            json!({"command": "goto_page", "args": {"page": "Post"}}),
        );
        assert_eq!(page["outcome"], "done", "{page}");
        assert!(d.app.post_current());
        // The user's edit, the page's check not run again yet.
        d.app.doc.setups[0].operations[0]
            .p
            .insert("feed".into(), "777".into());
        assert!(d.app.post.is_some() && !d.app.post_current());
        let edited = ask(&mut d, "inspect", json!({"what": "post"}));
        assert_eq!(
            (&edited["source"], &edited["app_check"], &edited["current"]),
            (&json!("worked_out"), &json!("not_current"), &json!(true)),
            "{edited}"
        );
        d.app.post = None;
        d.app.post_busy = true;
        let running = ask(&mut d, "inspect", json!({"what": "post"}));
        assert_eq!(running["app_check"], "running", "{running}");
        d.app.post_busy = false;
        // Pending, then changed while it was worked out.
        d.app.mcp.slow_reads = Some(Duration::from_millis(800));
        let pending = ask(&mut d, "inspect", json!({"what": "post", "wait_s": 0.05}));
        assert_eq!(pending["outcome"], "still_running", "{pending}");
        let busy = Some("An agent's check is still simulating the job: wait for it.");
        assert_eq!(d.app.planning_busy(), busy);
        d.app.doc.setups[0].operations[0]
            .p
            .insert("feed".into(), "555".into());
        let id = pending["command_id"].clone();
        let late = ask(&mut d, "wait", json!({"command_id": id, "wait_s": 50}));
        assert_eq!(late["outcome"], "done", "{late}");
        assert_eq!(
            (&late["current"], &late["stale"]),
            (&json!(false), &json!(true)),
            "{late}"
        );
        assert!(
            late["changed_meanwhile"]
                .as_array()
                .is_some_and(|c| !c.is_empty())
        );
        d.app.mcp.slow_reads = None;
        // While the machine runs.
        jogging(&mut d);
        let refused = ask(&mut d, "inspect", json!({"what": "post"}));
        assert_eq!(
            (refused["outcome"].as_str(), refused["reason"].as_str()),
            (Some("refused"), Some(LOCKED)),
            "{refused}"
        );
        stop(&mut d);
    }
}
