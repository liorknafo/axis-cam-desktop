//! Picking (LIO-470): the agent picks the part's surfaces, the example's
//! features, the holes the part drops over the fixture pins and the pins'
//! place in the stock's leftover, by the clicks a user makes, and checks
//! what each click did. The spec's *Picks*.
//!
//! - **Two kinds of picking** (`App::pick_kind`, which the click handler
//!   decides by too): a **fill** click takes one face that fills the
//!   operation and replaces its pick (a lathe cut, a Slot, a Box); a
//!   **toggle** click adds or drops a whole group, as the strategy groups
//!   faces (a hole's walls, a corner, a smooth face).
//! - **The engine does not copy the grouping.** Each id is one real click,
//!   and the picks after it are compared with the picks before it. A click
//!   that drops what was picked (the id is in a group picked already) is
//!   undone in full and reported; one that picks nothing gives its reason.
//! - **Undoing a click** puts back everything a click writes: the whole
//!   `Document` (every operation, the centre drillings linked to drills, the
//!   pick-review notes) and the `App`'s fields a click sets (`Before`). The
//!   work it cleared (the simulation, the audit report, a generation) is
//!   worked out again as after any edit, and the window waits for it. An
//!   undone click leaves no difference, so no change-log entry.
//! - **Picking mode is temporary** (`View`): turned on for the clicks, and
//!   the selection, the tab and the picking modes set back after them.
//!
//! A command's job runs in two turns of its ownership window: `begin` turns
//! picking on as it is sent, and `run` makes the clicks once the app has
//! settled (the part's holes for picking worked out, as the window does
//! before a user can click), then sets the view back.
use super::{commands::Verdict, engine::Backend};
use crate::{App, LastEdit, Message, Page, PickKind, Target, model::Document, viewport};
use serde_json::{Value, json};
use std::{collections::BTreeSet, time::Instant};

/// How a pick command's ids change the picks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Pick each id's group, unless it is picked.
    Add,
    /// Drop each id's group, if it is picked.
    Remove,
    /// The picks cleared, then `Add`.
    Replace,
}
impl Mode {
    pub const NAMES: [&'static str; 3] = ["add", "remove", "replace"];
    pub fn of(name: &str) -> Option<Mode> {
        match name.trim().to_ascii_lowercase().as_str() {
            "add" => Some(Mode::Add),
            "remove" => Some(Mode::Remove),
            "replace" => Some(Mode::Replace),
            _ => None,
        }
    }
}

/// What a pick command clicks, and on what.
#[derive(Debug, Clone)]
pub enum Job {
    /// The part's surfaces, for the operation of this id.
    Surfaces {
        op: String,
        ids: Vec<usize>,
        mode: Mode,
    },
    /// The example project's features (`model::FEATURES`), for the
    /// operation of this id.
    Features {
        op: String,
        ids: Vec<usize>,
        mode: Mode,
    },
    /// The holes the part drops over the pins, by surfaces in them, for the
    /// setup of this id.
    PinHoles {
        setup: String,
        ids: Vec<usize>,
        mode: Mode,
    },
    /// Pin 1's place in the stock's leftover, in the setup's plan (mm).
    Place { setup: String, at: [f32; 2] },
}

/// What a job came to: the command's verdict, the picks afterwards, and
/// what each id did.
pub struct Done {
    pub verdict: Verdict,
    pub picks: Value,
    pub clicks: Vec<Value>,
}

/// The view a pick command sets back when its clicks are done: the
/// selection, the setup editor's step, the operation's tab and the
/// picking modes.
#[derive(Debug, Clone, PartialEq)]
pub struct View {
    pub selected: Option<usize>,
    pub setup_step: &'static str,
    pub tab: String,
    pub picking: bool,
    pub picking_pins: bool,
    pub placing_pins: bool,
}
impl View {
    pub fn of(app: &App) -> View {
        View {
            selected: app.selected,
            setup_step: app.setup_step,
            tab: app.tab.clone(),
            picking: app.picking,
            picking_pins: app.picking_pins,
            placing_pins: app.placing_pins,
        }
    }
    /// Sets the view back by the clicks a user would make: the pins' modes
    /// off, the setup editor's step and the operation, the tab, then
    /// picking (a tab turns it on or off by itself).
    fn put_back(&self, b: &mut impl Backend) {
        if b.app().picking_pins != self.picking_pins {
            b.send(Message::PickPinHoles);
        }
        if b.app().placing_pins != self.placing_pins {
            b.send(Message::PlacePins);
        }
        if b.app().setup_step != self.setup_step {
            b.send(Message::SetupStep(self.setup_step));
        }
        if let Some(i) = self.selected
            && b.app().selected != Some(i)
        {
            b.send(Message::SelectOp(i));
        }
        if b.app().tab != self.tab {
            b.send(Message::Tab(self.tab.clone()));
        }
        if b.app().picking != self.picking {
            b.send(Message::PickMode);
        }
    }
}

/// Everything a click writes, kept before it so that it can be undone in
/// full: the whole `Document` and the `App`'s fields a click sets.
struct Before {
    doc: Document,
    project_dirty: bool,
    status: String,
    status_problem: bool,
    status_page: Page,
    typed_last: Option<LastEdit>,
    box_refused: Option<(String, String)>,
    page: Page,
    view: View,
    pin_aim: Option<[f32; 2]>,
    last_edit: Instant,
}
impl Before {
    fn of(app: &App) -> Before {
        Before {
            doc: app.doc.clone(),
            project_dirty: app.project_dirty,
            status: app.status.clone(),
            status_problem: app.status_problem,
            status_page: app.status_page,
            typed_last: app.typed_last.clone(),
            box_refused: app.box_refused.clone(),
            page: app.page,
            view: View::of(app),
            pin_aim: app.pin_aim,
            last_edit: app.last_edit,
        }
    }
    /// Whether the click edited the job (`App::touch`, or an edit).
    fn edited(&self, app: &App) -> bool {
        app.last_edit > self.last_edit
    }
    /// What the click said: the status line, when it changed it.
    fn said(&self, app: &App) -> Option<String> {
        (app.status != self.status && !app.status.is_empty()).then(|| app.status.clone())
    }
    /// Undoes the click: everything it wrote put back. What it cleared is
    /// worked out again, as after any edit.
    fn put_back(self, app: &mut App) {
        app.doc = self.doc;
        app.project_dirty = self.project_dirty;
        app.status = self.status;
        app.status_problem = self.status_problem;
        app.status_page = self.status_page;
        app.typed_last = self.typed_last;
        app.box_refused = self.box_refused;
        app.page = self.page;
        app.selected = self.view.selected;
        app.setup_step = self.view.setup_step;
        app.tab = self.view.tab;
        app.picking = self.view.picking;
        app.picking_pins = self.view.picking_pins;
        app.placing_pins = self.view.placing_pins;
        app.pin_aim = self.pin_aim;
        app.dirty = true;
        app.last_edit = Instant::now();
    }
}

/// The setup editor on its Work zero step, where the pins are picked.
fn open_work_zero(b: &mut impl Backend) {
    let app = b.app();
    if app.selected.is_some() || app.page != Page::Work || app.setup_step != "Work zero" {
        b.send(Message::SetupStep("Work zero"));
    }
}

impl Job {
    /// Every message the job may send, for the boundary's check before
    /// anything is sent.
    pub fn messages(&self) -> Vec<Message> {
        let view = [
            Message::Tab("Geometry".into()),
            Message::SelectOp(0),
            Message::SetupStep("Work zero"),
            Message::PickMode,
            Message::PickPinHoles,
            Message::PlacePins,
        ];
        let mut m: Vec<Message> = view.into();
        match self {
            Job::Surfaces { ids, .. } => {
                m.push(Message::ClearSurfaces);
                m.extend(ids.iter().map(|&n| Message::Surface(n)));
            }
            Job::Features { ids, .. } => m.extend(ids.iter().map(|&n| Message::Feature(n))),
            Job::PinHoles { ids, .. } => {
                m.push(Message::Edit(Target::Setup, "pin_holes", String::new()));
                m.extend(ids.iter().map(|&n| Message::PinHoleSurface(n)));
            }
            Job::Place { at, .. } => {
                m.push(Message::Viewport(viewport::Event::PlacePins(*at)));
            }
        }
        m
    }

    /// Turns picking on for the job, as it is sent: the view to set back.
    pub fn begin(&self, b: &mut impl Backend) -> View {
        let view = View::of(b.app());
        match self {
            Job::Surfaces { .. } | Job::Features { .. } => {
                if !b.app().picking {
                    b.send(Message::PickMode);
                }
            }
            Job::PinHoles { .. } => {
                open_work_zero(b);
                if !b.app().picking_pins {
                    b.send(Message::PickPinHoles);
                }
            }
            Job::Place { .. } => {
                open_work_zero(b);
                if !b.app().placing_pins {
                    b.send(Message::PlacePins);
                }
            }
        }
        view
    }

    /// Makes the clicks, once the app has settled, and sets `view` back.
    pub fn run(self, b: &mut impl Backend, view: &View) -> Done {
        let (verdict, clicks) = match &self {
            Job::Surfaces { op, ids, mode } => {
                if b.app().operation().is_none_or(|o| o.id != *op) {
                    gone("operation")
                } else if (b.app().operation())
                    .is_some_and(|o| b.app().pick_kind(o) == PickKind::Fill)
                {
                    fill(b, op, ids[0])
                } else {
                    let picks = |a: &App| surface_picks(a, op);
                    toggle(b, &picks, Message::Surface, ids, *mode, |b, _| {
                        b.send(Message::ClearSurfaces)
                    })
                }
            }
            Job::Features { op, ids, mode } => {
                if b.app().operation().is_none_or(|o| o.id != *op) {
                    gone("operation")
                } else {
                    let picks = |a: &App| feature_picks(a, op);
                    // The features are dropped as they are picked: a click
                    // on each.
                    toggle(b, &picks, Message::Feature, ids, *mode, |b, now| {
                        for n in now {
                            b.send(Message::Feature(n));
                        }
                    })
                }
            }
            Job::PinHoles { setup, ids, mode } => {
                if b.app().doc.active_setup().is_none_or(|s| s.id != *setup) {
                    gone("setup")
                } else if !b.app().picking_pin_holes() {
                    (
                        Err("The part's holes could not be picked for the pins: the Work zero step did not take picks.".into()),
                        vec![],
                    )
                } else {
                    pin_holes(b, setup, ids, *mode)
                }
            }
            Job::Place { setup, at } => {
                if b.app().doc.active_setup().is_none_or(|s| s.id != *setup) {
                    gone("setup")
                } else if !b.app().placing_leftover_pins() {
                    (
                        Err("The pins could not be placed: the Work zero step did not take a place.".into()),
                        vec![],
                    )
                } else {
                    place(b, setup, *at)
                }
            }
        };
        view.put_back(b);
        Done {
            verdict,
            picks: self.picks(b.app()),
            clicks,
        }
    }

    /// The picks afterwards, as the app lists them.
    fn picks(&self, app: &App) -> Value {
        match self {
            Job::Surfaces { op, .. } => listed(app, op),
            Job::Features { op, .. } => {
                let features: Vec<Value> = (feature_picks(app, op).into_iter())
                    .map(|n| json!({"id": n, "name": crate::model::FEATURES.get(n)}))
                    .collect();
                json!({"operation": op, "features": features})
            }
            Job::PinHoles { setup, .. } | Job::Place { setup, .. } => {
                let s = app.doc.setups.iter().find(|s| s.id == *setup);
                json!({
                    "setup": setup,
                    "pin_holes": s.map(crate::post::chosen_pin_holes),
                    "pin_place": s.map(|s| crate::model::get(&s.p, "pin_place")),
                })
            }
        }
    }
}

/// The verdict when the operation or setup the job was for is no longer
/// the one open: the user moved on while its picking mode came on.
fn gone(what: &str) -> (Verdict, Vec<Value>) {
    (
        Err(format!(
            "Nothing was picked: the {what} was no longer the one open (the user changed the selection meanwhile)."
        )),
        vec![],
    )
}

/// Operation `op`'s picked surfaces, of the part loaded now.
fn surface_picks(app: &App, op: &str) -> BTreeSet<usize> {
    let mesh = app.part_mesh();
    (app.doc.setups.iter().flat_map(|s| &s.operations))
        .find(|o| o.id == op)
        .and_then(|o| o.surfaces.as_ref())
        .filter(|s| mesh.is_some_and(|m| s.matches(m)))
        .map_or(BTreeSet::new(), |s| s.ids.iter().copied().collect())
}
/// Operation `op`'s features picked on the example part.
fn feature_picks(app: &App, op: &str) -> BTreeSet<usize> {
    (app.doc.setups.iter().flat_map(|s| &s.operations))
        .find(|o| o.id == op)
        .map_or(BTreeSet::new(), |o| o.geometry.iter().copied().collect())
}
fn list(set: &BTreeSet<usize>) -> Vec<usize> {
    set.iter().copied().collect()
}

/// Toggle picking: one click per id, each checked against the picks before
/// it, and undone in full when it dropped what was picked. `clear` empties
/// the picks for `Replace`, given them.
fn toggle<B: Backend>(
    b: &mut B,
    picks: &dyn Fn(&App) -> BTreeSet<usize>,
    click: fn(usize) -> Message,
    ids: &[usize],
    mode: Mode,
    clear: impl FnOnce(&mut B, BTreeSet<usize>),
) -> (Verdict, Vec<Value>) {
    let start = Before::of(b.app());
    let initial = picks(b.app());
    if mode == Mode::Replace && !initial.is_empty() {
        clear(b, initial.clone());
    }
    let mut clicks = vec![];
    let mut failed = vec![];
    for &id in ids {
        let now = picks(b.app());
        if mode == Mode::Remove {
            if !now.contains(&id) {
                // Dropped with an earlier id's group, or never picked.
                let did = if initial.contains(&id) {
                    "removed"
                } else {
                    "not picked"
                };
                clicks.push(json!({"id": id, "did": did}));
                continue;
            }
            let before = Before::of(b.app());
            b.send(click(id));
            let after = picks(b.app());
            if after.contains(&id) || after.difference(&now).next().is_some() {
                let reason =
                    (before.said(b.app())).unwrap_or_else(|| "The click did not drop it.".into());
                before.put_back(b.app_mut());
                failed.push(format!("{id}: {reason}"));
                clicks.push(json!({"id": id, "did": "refused", "reason": reason}));
            } else {
                let group = list(&now.difference(&after).copied().collect());
                clicks.push(json!({"id": id, "did": "removed", "group": group}));
            }
            continue;
        }
        if now.contains(&id) {
            clicks.push(json!({"id": id, "did": "already picked"}));
            continue;
        }
        let before = Before::of(b.app());
        b.send(click(id));
        let after = picks(b.app());
        let dropped: BTreeSet<usize> = now.difference(&after).copied().collect();
        let added: BTreeSet<usize> = after.difference(&now).copied().collect();
        if !dropped.is_empty() {
            // The id is in a group picked already: the click would drop it.
            let said = before.said(b.app()).unwrap_or_default();
            before.put_back(b.app_mut());
            clicks.push(json!({
                "id": id,
                "did": "already picked with group",
                "group": list(&dropped),
                "reason": said,
            }));
        } else if added.is_empty() {
            let reason =
                (before.said(b.app())).unwrap_or_else(|| "The click picked nothing.".into());
            if before.edited(b.app()) {
                before.put_back(b.app_mut());
            }
            failed.push(format!("{id}: {reason}"));
            clicks.push(json!({"id": id, "did": "refused", "reason": reason}));
        } else {
            clicks.push(json!({"id": id, "did": "added", "group": list(&added)}));
        }
    }
    let verdict = if !failed.is_empty() {
        Err(format!(
            "Not every id was picked as asked ({}). The picks are as listed.",
            failed.join("; ")
        ))
    } else if picks(b.app()) == initial {
        // Cleared and picked again as they were: nothing changed.
        if mode == Mode::Replace && start.edited(b.app()) {
            start.put_back(b.app_mut());
        }
        Ok(Some(match mode {
            Mode::Remove => "None of the ids was picked.".to_string(),
            _ => "Every id was picked already.".to_string(),
        }))
    } else {
        Ok(None)
    };
    (verdict, clicks)
}

/// A fill click: one face fills the operation and replaces its pick. The
/// same face again, with the fields it fills unchanged, is undone.
fn fill(b: &mut impl Backend, op: &str, id: usize) -> (Verdict, Vec<Value>) {
    let state = |a: &App| {
        (a.doc.setups.iter().flat_map(|s| &s.operations))
            .find(|o| o.id == op)
            .map(|o| {
                let pick =
                    (o.surfaces.as_ref()).map(|s| (s.mesh_fingerprint.clone(), s.ids.clone()));
                (pick, o.p.clone(), o.geometry.clone())
            })
    };
    let was = state(b.app());
    let before = Before::of(b.app());
    b.send(Message::Surface(id));
    if !before.edited(b.app()) {
        let reason = (before.said(b.app()))
            .or_else(|| b.app().box_refused.as_ref().map(|(_, e)| e.clone()))
            .unwrap_or_else(|| "That face fills nothing.".into());
        let click = json!({"id": id, "did": "refused", "reason": reason});
        return (Err(reason), vec![click]);
    }
    if state(b.app()) == was {
        before.put_back(b.app_mut());
        let click = json!({"id": id, "did": "already picked"});
        return (
            Ok(Some(
                "That face is picked, and fills the fields as they are, already.".into(),
            )),
            vec![click],
        );
    }
    let said = b.app().status.clone();
    (
        Ok(None),
        vec![json!({"id": id, "did": "filled", "reason": said})],
    )
}

/// The holes chosen for setup `setup`'s pins.
fn chosen(app: &App, setup: &str) -> Vec<[f32; 3]> {
    (app.doc.setups.iter().find(|s| s.id == setup))
        .map(crate::post::chosen_pin_holes)
        .unwrap_or_default()
}
/// The holes of `a` not in `b`.
fn holes_off(a: &[[f32; 3]], b: &[[f32; 3]]) -> Vec<[f32; 3]> {
    a.iter().filter(|h| !b.contains(h)).copied().collect()
}

/// The pin holes: a click in each hole (`PinHoleSurface`), checked as a
/// toggle click is. A hole is known by the click, so whether an id's hole
/// is chosen is too.
fn pin_holes(
    b: &mut impl Backend,
    setup: &str,
    ids: &[usize],
    mode: Mode,
) -> (Verdict, Vec<Value>) {
    let start = Before::of(b.app());
    let initial = chosen(b.app(), setup);
    if mode == Mode::Replace && !initial.is_empty() {
        b.send(Message::Edit(Target::Setup, "pin_holes", String::new()));
    }
    let mut clicks = vec![];
    let mut failed = vec![];
    for &id in ids {
        let now = chosen(b.app(), setup);
        let before = Before::of(b.app());
        b.send(Message::PinHoleSurface(id));
        let after = chosen(b.app(), setup);
        let (added, dropped) = (holes_off(&after, &now), holes_off(&now, &after));
        let (wanted, unwanted, did, skip) = match mode {
            Mode::Remove => (&dropped, &added, "removed", "not picked"),
            _ => (&added, &dropped, "added", "already picked with group"),
        };
        if !unwanted.is_empty() {
            // The other way: a hole chosen already (or not chosen) by an
            // earlier id or before.
            let group = unwanted.clone();
            before.put_back(b.app_mut());
            clicks.push(json!({"id": id, "did": skip, "group": group}));
        } else if wanted.is_empty() {
            let reason =
                (before.said(b.app())).unwrap_or_else(|| "The click chose no hole.".into());
            if before.edited(b.app()) {
                before.put_back(b.app_mut());
            }
            failed.push(format!("{id}: {reason}"));
            clicks.push(json!({"id": id, "did": "refused", "reason": reason}));
        } else {
            clicks.push(json!({"id": id, "did": did, "group": wanted}));
        }
    }
    let same =
        |a: &[[f32; 3]], b: &[[f32; 3]]| holes_off(a, b).is_empty() && holes_off(b, a).is_empty();
    let verdict = if !failed.is_empty() {
        Err(format!(
            "Not every id was picked as asked ({}). The pin holes are as listed.",
            failed.join("; ")
        ))
    } else if same(&chosen(b.app(), setup), &initial) {
        if start.edited(b.app()) {
            start.put_back(b.app_mut());
        }
        Ok(Some(match mode {
            Mode::Remove => "None of these holes was chosen.".to_string(),
            _ => "Every hole was chosen already.".to_string(),
        }))
    } else {
        Ok(None)
    };
    (verdict, clicks)
}

/// The pins' place: a click on the leftover with the pins placing.
fn place(b: &mut impl Backend, setup: &str, at: [f32; 2]) -> (Verdict, Vec<Value>) {
    let place_of = |a: &App| {
        (a.doc.setups.iter().find(|s| s.id == setup))
            .map(|s| crate::model::get(&s.p, "pin_place").to_string())
    };
    let was = place_of(b.app());
    let before = Before::of(b.app());
    b.send(Message::Viewport(viewport::Event::PlacePins(at)));
    if place_of(b.app()) != was {
        return (Ok(None), vec![]);
    }
    if before.edited(b.app()) {
        before.put_back(b.app_mut());
        return (Ok(Some("The pins stand there already.".into())), vec![]);
    }
    let reason = (before.said(b.app())).unwrap_or_else(|| "The pins were not placed.".into());
    (Err(reason), vec![])
}

/// Operation `op`'s picks as the app lists them: the surfaces, with the
/// holes a Pocket, a Contour or a Drill reads from them, the corners a
/// Round corners does, and the faces a Parallel or Waterline finishing
/// takes; and the part's fingerprint they belong to.
pub fn listed(app: &App, op: &str) -> Value {
    let Some(o) = (app.doc.setups.iter().flat_map(|s| &s.operations)).find(|o| o.id == op) else {
        return json!({"operation": op});
    };
    let mesh = app.part_mesh();
    let selection = o.surfaces.as_ref();
    let current = selection.is_some_and(|s| mesh.is_some_and(|m| s.matches(m)));
    let mut out = json!({
        "operation": op,
        "mesh_fingerprint": mesh.map(|m| m.fingerprint.clone()),
        "ids": surface_picks(app, op).into_iter().collect::<Vec<_>>(),
    });
    if selection.is_some() && !current {
        out["note"] =
            json!("The picks are of another version of the part: dismiss_pick_note or pick again.");
    }
    let Some(mesh) = mesh.filter(|_| current) else {
        return out;
    };
    let mm = |v: f64| (v * 1000.).round() / 1000.;
    if crate::picks_holes(o) {
        match &*app.pocket_holes(o, mesh) {
            Ok(holes) => {
                let holes: Vec<Value> = (holes.iter())
                    .map(|h| {
                        let ((x, y), (w, d)) = (h.centre(), h.size());
                        json!({"x": mm(x), "y": mm(y), "width": mm(w), "depth_y": mm(d), "walls": h.walls.len(), "a": h.a})
                    })
                    .collect();
                out["holes"] = json!(holes);
            }
            Err(e) => out["holes_error"] = json!(e),
        }
        if o.strategy == crate::model::Strategy::Drill {
            match &*app.drill_holes(o, mesh) {
                Ok(holes) => {
                    let drilled: Vec<Value> = (holes.iter())
                        .map(|h| {
                            json!({
                                "diameter": mm(h.diameter as f64),
                                "x": mm(h.center[0] as f64),
                                "y": mm(h.center[1] as f64),
                                "depth": mm((h.top - h.bottom) as f64),
                                "through": h.through,
                                "arc": h.arc.is_some(),
                                "a": h.a,
                            })
                        })
                        .collect();
                    out["drilled"] = json!(drilled);
                }
                Err(e) => out["drilled_error"] = json!(e),
            }
        }
    }
    if o.strategy == crate::model::Strategy::Corner {
        match crate::corner::picked(app.doc.setup(), mesh, o) {
            Ok(corners) => {
                let corners: Vec<Value> = (corners.iter())
                    .map(|c| json!({"place": c.place(), "r": mm(c.r as f64), "walls": c.walls.len()}))
                    .collect();
                out["corners"] = json!(corners);
            }
            Err(e) => out["corners_error"] = json!(e),
        }
    }
    if crate::picks_faces(o) {
        let faces: BTreeSet<usize> = (selection.into_iter().flat_map(|s| &s.ids))
            .map(|&id| mesh.regions.get(id).copied().unwrap_or(id))
            .collect();
        out["faces"] = json!(faces.len());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::commands::tests::{Direct, direct, imported, outcome, pinned, run};
    use crate::mcp::engine::Backend;
    use crate::mesh::Mesh;
    use crate::model::{ModelFile, Operation, Strategy, Tool};

    /// The view a pick command must leave as it found it.
    fn view(d: &Direct) -> (Option<usize>, String, bool, bool, bool) {
        let a = &d.app;
        (
            a.selected,
            a.tab.clone(),
            a.picking,
            a.picking_pins,
            a.placing_pins,
        )
    }
    /// A pick command run, the view checked to be as it was.
    fn pick(d: &mut Direct, command: &str, args: Value) -> Value {
        let before = view(d);
        let r = run(d, command, args).unwrap();
        assert_eq!(view(d), before, "{command}: {r}");
        r
    }
    /// The job as a project saves it.
    fn saved(d: &Direct) -> Value {
        serde_json::to_value(&d.app.doc).unwrap()
    }
    /// The selected operation's picks.
    fn ids(d: &Direct) -> BTreeSet<usize> {
        let op = d.app.operation().unwrap().id.clone();
        surface_picks(&d.app, &op)
    }
    fn file() -> ModelFile {
        ModelFile {
            name: "part.stl".into(),
            path: "part.stl".into(),
            format: "stl".into(),
            bytes: 1,
            thickness: None,
        }
    }
    /// A tool of `kind` ("Drill", "Flat end mill", "Ball end mill"), Ø`d`.
    fn tool(kind: &str, d: &str) -> Tool {
        let mut t = Tool::new(&format!("{kind} {d}"), kind, d);
        for (k, v) in [
            ("length", "40"),
            ("stickout", "40"),
            ("flutes", "2"),
            ("number", "9"),
        ] {
            t.p.insert(k.into(), v.into());
        }
        t
    }
    /// `mesh` imported into the sample job, its first operation a
    /// `strategy` cutting with `tool`, selected, on its Tool tab.
    fn part(mesh: Mesh, strategy: Strategy, tool: Tool) -> Direct {
        let mut d = direct();
        d.send(Message::FileImported(Some(Ok((file(), Some(mesh))))));
        let id = tool.id.clone();
        d.app.doc.tools.push(tool);
        d.app.doc.setup_mut().operations[0] = Operation::new(strategy, id);
        d.app.page = Page::Work;
        d.app.selected = Some(0);
        d.send(Message::Tab("Tool".into()));
        d.app.dirty = false;
        d
    }
    /// The part's surfaces whose centres `test` takes: X and Y as the setup
    /// places them, Z over the part's bottom (mm).
    fn surfaces_where(d: &Direct, test: impl Fn([f32; 3]) -> bool) -> Vec<usize> {
        let mesh = d.app.part_mesh().unwrap();
        let placement = crate::stock::Placement::new(mesh, Some(d.app.doc.setup()));
        let scale = mesh.display_scale();
        let low = placement.bounds.min[2] / scale;
        (0..mesh.surfaces.len())
            .filter(|&id| {
                let c = placement
                    .transform(mesh.surfaces[id].center)
                    .map(|v| v / scale);
                test([c[0], c[1], c[2] - low])
            })
            .collect()
    }
    /// `pocket::chamfered_hole`'s walls (24) and one facet of its chamfer.
    fn hole_walls_and_chamfer(d: &Direct) -> (Vec<usize>, usize) {
        let walls = surfaces_where(d, |c| (c[2] - 4.).abs() < 1. && c[0].hypot(c[1]) < 3.2);
        let chamfer = surfaces_where(d, |c| (c[2] - 9.5).abs() < 0.1 && c[0].hypot(c[1]) < 4.);
        assert_eq!(walls.len(), 24);
        (walls, chamfer[0])
    }
    fn did(r: &Value) -> Vec<&str> {
        (r["clicks"].as_array().unwrap().iter())
            .map(|c| c["did"].as_str().unwrap())
            .collect()
    }

    #[test]
    fn a_toggle_pick_takes_each_group_once_whatever_ids_of_it_are_sent() {
        // Three facets of one rounded corner in one call pick the corner once
        // (sent one by one as clicks, they picked it and dropped it).
        let mut d = part(
            crate::corner::tests::adapter(),
            Strategy::Corner,
            tool("Flat end mill", "10"),
        );
        let corners = crate::corner::all(d.app.doc.setup(), d.app.part_mesh().unwrap());
        let corner = corners[0].walls.clone();
        let r = pick(
            &mut d,
            "pick_surfaces",
            json!({"ids": &corner[..3], "mode": "add"}),
        );
        assert_eq!(outcome(&r), "done", "{r}");
        assert_eq!(ids(&d), corner.iter().copied().collect());
        assert_eq!(did(&r), ["added", "already picked", "already picked"]);
        assert_eq!(r["picks"]["corners"].as_array().unwrap().len(), 1, "{r}");
        assert_eq!(
            r["picks"]["mesh_fingerprint"],
            d.app.part_mesh().unwrap().fingerprint.as_str()
        );
        // The same again: nothing is clicked.
        let again = pick(
            &mut d,
            "pick_surfaces",
            json!({"ids": &corner[..3], "mode": "add"}),
        );
        assert_eq!(outcome(&again), "no_change", "{again}");
        assert!(again["changed"].as_array().unwrap().is_empty());
        // An id not picked, removed: nothing changes.
        let other = corners[1].walls[0];
        let r = pick(
            &mut d,
            "pick_surfaces",
            json!({"ids": [other], "mode": "remove"}),
        );
        assert_eq!(outcome(&r), "no_change", "{r}");
        // One face of the picked corner removed drops the corner.
        let r = pick(
            &mut d,
            "pick_surfaces",
            json!({"ids": [corner[5]], "mode": "remove"}),
        );
        assert_eq!(outcome(&r), "done", "{r}");
        assert!(ids(&d).is_empty());
        assert_eq!(did(&r), ["removed"]);
        assert_eq!(
            r["clicks"][0]["group"].as_array().unwrap().len(),
            corner.len()
        );

        // Three walls of one hole in a Pocket pick its 24 walls once.
        let mut d = part(
            crate::mesh::normalize(crate::pocket::chamfered_hole()).unwrap(),
            Strategy::Pocket,
            tool("Flat end mill", "4"),
        );
        let (walls, _) = hole_walls_and_chamfer(&d);
        let r = pick(
            &mut d,
            "pick_surfaces",
            json!({"ids": [walls[0], walls[7], walls[15]]}),
        );
        assert_eq!(outcome(&r), "done", "{r}");
        assert_eq!(ids(&d), walls.iter().copied().collect());
        assert_eq!(did(&r), ["added", "already picked", "already picked"]);
        assert_eq!(r["picks"]["holes"].as_array().unwrap().len(), 1, "{r}");

        // Three surfaces of one smooth face (the dome) in a Parallel
        // finishing pick the face once.
        let mut d = part(
            crate::cam::tests::half_ball_on_plate(),
            Strategy::Parallel,
            tool("Ball end mill", "6"),
        );
        let whole = pick(&mut d, "pick_surfaces", json!({"ids": [0]}));
        assert!(
            whole["reason"]
                .as_str()
                .unwrap()
                .contains("Selected geometry"),
            "{whole}"
        );
        let bounded =
            json!({"target": "operation", "key": "boundary", "value": "Selected geometry"});
        assert_eq!(
            outcome(&run(&mut d, "edit_field", bounded).unwrap()),
            "done"
        );
        let mesh = d.app.part_mesh().unwrap().clone();
        let sloped = (0..mesh.surfaces.len())
            .find(|&i| (0.3..0.9).contains(&mesh.surfaces[i].normal[2].abs()))
            .unwrap();
        let face = mesh.region_of(sloped);
        assert!(face.len() > 3);
        let r = pick(
            &mut d,
            "pick_surfaces",
            json!({"ids": &face[..3], "mode": "add"}),
        );
        assert_eq!(outcome(&r), "done", "{r}");
        assert_eq!(ids(&d), face.iter().copied().collect());
        assert_eq!(r["picks"]["faces"], 1, "{r}");
        // Replaced by the same face: nothing changed.
        let r = pick(
            &mut d,
            "pick_surfaces",
            json!({"ids": [face[4]], "mode": "replace"}),
        );
        assert_eq!(outcome(&r), "no_change", "{r}");
        assert!(r["changed"].as_array().unwrap().is_empty(), "{r}");
    }

    #[test]
    fn a_click_that_would_drop_a_picked_group_is_undone_in_full() {
        let mut d = part(
            crate::mesh::normalize(crate::pocket::chamfered_hole()).unwrap(),
            Strategy::Drill,
            tool("Drill", "6"),
        );
        let (walls, chamfer) = hole_walls_and_chamfer(&d);
        let r = pick(&mut d, "pick_surfaces", json!({"ids": [walls[0]]}));
        assert_eq!(outcome(&r), "done", "{r}");
        assert_eq!(ids(&d).len(), 24);
        // A centre drilling linked to the Drill, and a note on its picks.
        d.send(Message::AddCentring);
        let ops = &d.app.doc.setup().operations;
        let drill = ops[1].id.clone();
        assert_eq!(crate::centre::centres(&ops[0]), Some(drill.as_str()));
        let picked = |o: &Operation| o.surfaces.as_ref().map(|s| s.ids.clone());
        assert_eq!(picked(&ops[0]), picked(&ops[1]));
        assert_eq!(d.app.selected, Some(1));
        d.app
            .doc
            .pick_checks
            .insert(drill.clone(), "The part changed: check the picks.".into());
        d.app.status = "Checked before.".into();
        d.app.status_page = Page::Audit;
        let revision =
            run(&mut d, "select_operation", json!({"operation": 1})).unwrap()["revision"].clone();
        let (doc, status, page, tab) = (
            saved(&d),
            d.app.status.clone(),
            d.app.status_page,
            d.app.tab.clone(),
        );
        // The chamfer's click would drop the hole: it is undone.
        let r = pick(
            &mut d,
            "pick_surfaces",
            json!({"ids": [chamfer], "mode": "add"}),
        );
        assert_eq!(outcome(&r), "no_change", "{r}");
        assert_eq!(did(&r), ["already picked with group"]);
        assert_eq!(r["clicks"][0]["group"].as_array().unwrap().len(), 24, "{r}");
        assert_eq!(saved(&d), doc);
        assert!(d.app.doc.pick_checks.contains_key(&drill));
        assert_eq!(
            d.app.doc.setup().operations[0]
                .surfaces
                .as_ref()
                .unwrap()
                .ids
                .len(),
            24
        );
        assert_eq!(
            (d.app.status.as_str(), d.app.status_page, &d.app.tab),
            (status.as_str(), page, &tab)
        );
        // Inside the window, and nothing differs: no change-log entry.
        assert!(r["changed"].as_array().unwrap().is_empty(), "{r}");
        assert_eq!(r["revision"], revision);
    }

    #[test]
    fn a_fill_pick_takes_one_face_with_replace() {
        // A Box on the fixture block.
        let mut d = imported();
        run(&mut d, "add_operation", json!({"strategy": "Box"})).unwrap();
        run(&mut d, "set_tab", json!({"tab": "Tool"})).unwrap();
        let mesh = d.app.part_mesh().unwrap().clone();
        let top_z = (mesh.surfaces.iter())
            .filter(|s| s.normal[2] > 0.99999)
            .map(|s| s.center[2])
            .fold(f32::MIN, f32::max);
        let top = (mesh.surfaces.iter())
            .position(|s| s.normal[2] > 0.99999 && s.center[2] == top_z)
            .unwrap();
        fills(&mut d, "Box", top, (top + 1) % mesh.surfaces.len());

        // A Slot on the manual mill.
        let mut d = direct();
        let mesh = crate::mesh::normalize(crate::cam::arcs::tests::slot(5.)).unwrap();
        let setup = crate::corner::tests::setup("0");
        let mut cutter = crate::corner::tests::end_mill("10");
        cutter.p.insert("center_cutting".into(), "Yes".into());
        cutter.p.insert("machines".into(), setup.machine.id.clone());
        let mut doc = crate::model::Document::sample();
        doc.sample_model = false;
        doc.file = Some(file());
        doc.machines.push(setup.machine.clone());
        let mut s = setup.clone();
        s.operations = vec![Operation::new(Strategy::Slot, cutter.id.clone())];
        doc.active = s.id.clone();
        doc.setups = vec![s];
        doc.tools.push(cutter);
        d.app.replace_document(doc, None);
        d.app.imported_mesh = Some(mesh.clone());
        d.app.selected = Some(0);
        d.send(Message::Tab("Tool".into()));
        let arcs =
            crate::cam::arcs::arcs_among(&setup, &mesh, &crate::cam::arcs::upright(&setup, &mesh));
        fills(&mut d, "Slot", arcs[0].walls[0], arcs[1].walls[0]);

        // A lathe Outside: the Ø16 pin of a Ø30 body.
        use crate::turn::tests::{job, tube};
        let mut part = tube(0., 15., 0., 30.);
        part.extend(tube(0., 8., 30., 50.));
        let (mut doc, mesh) = job(50., 100., part, -52.);
        doc.file = Some(file());
        let mut d = direct();
        d.app.replace_document(doc, None);
        d.app.imported_mesh = Some(mesh);
        run(&mut d, "add_turn", json!({"kind": "Outside"})).unwrap();
        run(&mut d, "set_tab", json!({"tab": "Tool"})).unwrap();
        let g = crate::cam::Geometry::new(d.app.doc.setup(), d.app.part_mesh().unwrap());
        let face = |keep: &dyn Fn([f32; 3]) -> bool| {
            let mut all = std::collections::BTreeMap::new();
            for t in &g.triangles {
                if let Some(f) = t.feature {
                    *all.entry(f).or_insert(true) &= t.p.iter().all(|p| keep(*p));
                }
            }
            all.into_iter().find(|(_, ok)| *ok).unwrap().0
        };
        let pin = face(&|p| (p[0].hypot(p[1]) - 8.).abs() < 0.01);
        let body = face(&|p| (p[0].hypot(p[1]) - 15.).abs() < 0.01);
        fills(&mut d, "Turn", pin, body);
        assert_eq!(
            crate::model::get(&d.app.operation().unwrap().p, "diameter"),
            "16"
        );
    }
    /// A fill operation selected in `d`: `face` fills it, `other` is
    /// another face. Two ids and add are refused with nothing changed; one
    /// id with replace fills; the same again changes nothing; clear_picks
    /// clears it. The face is left picked.
    fn fills(d: &mut Direct, strategy: &str, face: usize, other: usize) {
        let doc = saved(d);
        let takes_one = format!("{strategy} takes one face, which fills it");
        for args in [
            json!({"ids": [face, other], "mode": "replace"}),
            json!({"ids": [face], "mode": "add"}),
            json!({"ids": [face]}),
            json!({"ids": [face], "mode": "remove"}),
        ] {
            let r = pick(d, "pick_surfaces", args);
            assert_eq!(outcome(&r), "refused", "{strategy}: {r}");
            assert!(r["reason"].as_str().unwrap().starts_with(&takes_one), "{r}");
            assert_eq!(saved(d), doc);
        }
        let one = json!({"ids": [face], "mode": "replace"});
        let r = pick(d, "pick_surfaces", one.clone());
        assert_eq!(outcome(&r), "done", "{strategy}: {r}");
        assert_eq!(did(&r), ["filled"]);
        assert!(!r["picks"]["ids"].as_array().unwrap().is_empty(), "{r}");
        let filled = saved(d);
        let r = pick(d, "pick_surfaces", one.clone());
        assert_eq!(outcome(&r), "no_change", "{strategy}: {r}");
        assert!(r["changed"].as_array().unwrap().is_empty(), "{r}");
        assert_eq!(saved(d), filled);
        let r = run(d, "clear_picks", json!({})).unwrap();
        assert_eq!(outcome(&r), "done", "{r}");
        assert!(d.app.operation().unwrap().surfaces.is_none());
        assert_eq!(outcome(&pick(d, "pick_surfaces", one)), "done");
    }

    #[test]
    fn the_examples_features_are_picked_as_clicks_pick_them() {
        let mut d = direct();
        let o = d.app.operation().unwrap().clone();
        let f = *o.strategy.features().last().unwrap();
        if o.geometry.contains(&f) {
            let r = pick(
                &mut d,
                "pick_features",
                json!({"ids": [f], "mode": "remove"}),
            );
            assert_eq!(outcome(&r), "done", "{r}");
        }
        let r = pick(&mut d, "pick_features", json!({"ids": [f]}));
        assert_eq!(outcome(&r), "done", "{r}");
        assert!(d.app.operation().unwrap().geometry.contains(&f));
        assert_eq!(
            r["picks"]["features"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|x| x["id"] == f)
                .count(),
            1
        );
        let r = pick(&mut d, "pick_features", json!({"ids": [f]}));
        assert_eq!(outcome(&r), "no_change", "{r}");
        let r = pick(
            &mut d,
            "pick_features",
            json!({"ids": [f], "mode": "remove"}),
        );
        assert_eq!(outcome(&r), "done", "{r}");
        assert!(!d.app.operation().unwrap().geometry.contains(&f));
        let r = pick(
            &mut d,
            "pick_features",
            json!({"ids": [f], "mode": "remove"}),
        );
        assert_eq!(outcome(&r), "no_change", "{r}");
        let off = (0..crate::model::FEATURES.len())
            .find(|n| !o.strategy.features().contains(n))
            .unwrap();
        let r = pick(&mut d, "pick_features", json!({"ids": [off]}));
        assert_eq!(outcome(&r), "refused", "{r}");
        // The example's part has no surfaces to pick.
        let r = pick(&mut d, "pick_surfaces", json!({"ids": [0]}));
        assert_eq!(outcome(&r), "refused", "{r}");
    }

    #[test]
    fn the_pin_holes_are_chosen_and_the_pins_placed_by_clicks() {
        let mut d = direct();
        pinned(&mut d, crate::post::fixture::pinned_flip("rx", true));
        // An operation of the setup open, on its Geometry tab, picking.
        d.send(Message::SelectOp(0));
        d.send(Message::Tab("Geometry".into()));
        let cleared = run(&mut d, "clear_pin_holes", json!({})).unwrap();
        assert!(
            matches!(outcome(&cleared), "done" | "no_change"),
            "{cleared}"
        );
        let mesh = d.app.part_mesh().unwrap().clone();
        let setup = d.app.doc.setups[1].clone();
        let holes = crate::cam::bottom_holes(&setup, &mesh).unwrap();
        assert_eq!(holes.len(), 2);
        let placement = crate::stock::Placement::new(&mesh, Some(&setup));
        let normal_z = |i: usize| {
            placement.transform(mesh.surfaces[i].normal)[2] - placement.transform([0.; 3])[2]
        };
        let of = |i| crate::cam::surface_hole(&setup, &mesh, &holes, i);
        let wall = (0..mesh.surfaces.len())
            .find(|&i| of(i) == Some(0) && normal_z(i).abs() < 0.01)
            .unwrap();
        let end = (0..mesh.surfaces.len())
            .find(|&i| of(i) == Some(1) && normal_z(i).abs() > 0.99)
            .unwrap();
        let none = (0..mesh.surfaces.len()).find(|&i| of(i).is_none()).unwrap();
        let chosen = |d: &Direct| crate::post::chosen_pin_holes(&d.app.doc.setups[1]).len();
        let r = pick(&mut d, "pick_pin_holes", json!({"ids": [wall, end]}));
        assert_eq!(outcome(&r), "done", "{r}");
        assert_eq!(chosen(&d), 2);
        assert_eq!(r["picks"]["pin_holes"].as_array().unwrap().len(), 2);
        // A hole chosen already: its click undone in full.
        let doc = saved(&d);
        let r = pick(
            &mut d,
            "pick_pin_holes",
            json!({"ids": [wall], "mode": "add"}),
        );
        assert_eq!(outcome(&r), "no_change", "{r}");
        assert_eq!(did(&r), ["already picked with group"]);
        assert_eq!(saved(&d), doc);
        // A face in no hole says why.
        let r = pick(&mut d, "pick_pin_holes", json!({"ids": [none]}));
        assert_eq!(outcome(&r), "failed", "{r}");
        assert!(
            r["reason"]
                .as_str()
                .unwrap()
                .contains("not in a round hole"),
            "{r}"
        );
        assert_eq!(saved(&d), doc);
        let r = pick(
            &mut d,
            "pick_pin_holes",
            json!({"ids": [end], "mode": "remove"}),
        );
        assert_eq!(outcome(&r), "done", "{r}");
        assert_eq!(chosen(&d), 1);
        let r = pick(
            &mut d,
            "pick_pin_holes",
            json!({"ids": [end], "mode": "remove"}),
        );
        assert_eq!(outcome(&r), "no_change", "{r}");
        assert_eq!(did(&r), ["not picked"]);
        assert_eq!(
            outcome(&run(&mut d, "clear_pin_holes", json!({})).unwrap()),
            "done"
        );
        assert_eq!(chosen(&d), 0);
        assert_eq!(
            outcome(&run(&mut d, "clear_pin_holes", json!({})).unwrap()),
            "no_change"
        );
        // These pins go in the part's holes: there is no place to click.
        let r = pick(&mut d, "place_pins", json!({"x": 0, "y": 0}));
        assert_eq!(outcome(&r), "refused", "{r}");

        // Pins in the stock's leftover.
        let mut d = direct();
        pinned(&mut d, crate::post::fixture::leftover_flip("rx", None));
        let mesh = d.app.part_mesh().unwrap().clone();
        let placed = crate::post::leftover_places(&d.app.doc, &mesh, 1).unwrap();
        let part = crate::simulation::part_box(&d.app.doc.setups[1], &mesh);
        let near = |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).hypot(a[1] - b[1]) < 0.01;
        let to = [placed[0][0] - 2., placed[0][1]];
        let r = pick(&mut d, "place_pins", json!({"x": to[0], "y": to[1]}));
        assert_eq!(outcome(&r), "done", "{r}");
        assert!(near(
            crate::post::leftover_places(&d.app.doc, &mesh, 1).unwrap()[0],
            to
        ));
        assert!(!r["picks"]["pin_place"].as_str().unwrap().is_empty());
        let r = pick(&mut d, "place_pins", json!({"x": to[0], "y": to[1]}));
        assert_eq!(outcome(&r), "no_change", "{r}");
        // Under the part they do not fit, and stay.
        let doc = saved(&d);
        let y = (part.min[1] + part.max[1]) / 2.;
        let r = pick(&mut d, "place_pins", json!({"x": part.min[0] + 5., "y": y}));
        assert_eq!(outcome(&r), "failed", "{r}");
        assert!(
            r["reason"]
                .as_str()
                .unwrap()
                .starts_with("The pins do not fit there"),
            "{r}"
        );
        assert_eq!(saved(&d), doc);
        assert_eq!(
            outcome(&run(&mut d, "clear_pin_place", json!({})).unwrap()),
            "done"
        );
        assert_eq!(
            outcome(&run(&mut d, "clear_pin_place", json!({})).unwrap()),
            "no_change"
        );
        let r = pick(&mut d, "pick_pin_holes", json!({"ids": [0]}));
        assert_eq!(outcome(&r), "refused", "{r}");
    }
}
