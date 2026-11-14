//! The headless session (LIO-470): an `App` with no window, on a thread of
//! its own, for an MCP client when no AXIS CAM window is open.
//!
//! The `App` is built on that thread and never leaves it: it is not `Send`
//! on macOS, where the controller's `ActivityToken` is held in it. Only tool
//! calls, their answers and the messages tasks produce cross threads. The
//! thread loops over a channel of events, as the window's event loop
//! would: tool calls, the messages of the tasks the `App` started, and a
//! `Tick` at the window's rate (`App::ticks_fast`). Tasks run on a
//! multi-thread tokio runtime (`iced_runtime::task::into_stream`); their
//! window and widget actions have no window to act on and are dropped.
//!
//! What the engine sends is the agent's (`App::update_by_agent`), and so are
//! the messages of the work it starts: a task carries who started it. A
//! `Tick` and the work it starts are the app's own.
use super::engine::{Agent, Backend, Reply};
use super::session::SessionFile;
use crate::{App, Message};
use iced::Task;
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering::SeqCst},
        mpsc,
    },
    time::{Duration, Instant},
};

/// The `Tick` rates of the window's subscription (`App::subscription`).
const FAST_TICK: Duration = Duration::from_millis(33);
const SLOW_TICK: Duration = Duration::from_millis(500);

/// A headless session: its id and the folder it is kept in,
/// `AXIS CAM Studio/mcp-sessions/<id>/`, which holds its own workspace
/// (`session` says what else). Open, it holds the folder's lock: no other
/// process opens it until the last of its clones goes.
#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub folder: PathBuf,
    /// Held, never read: dropped with the last clone, it lets go.
    _lock: Option<Arc<super::session::Lock>>,
}
impl Session {
    /// The folder the sessions are kept in, beside the user's workspace.
    pub fn root() -> PathBuf {
        crate::storage::state_path().with_file_name("mcp-sessions")
    }
    /// Session `id` in `root`, whose folder must be there and which no
    /// other live process has open (`session::Lock`), or with no id a new
    /// session, which starts from a copy of the user's `workspace` (when
    /// there is one) so it has their libraries. The user's workspace is
    /// only read: the session's `App` saves its own copy.
    pub fn open(root: &Path, id: Option<&str>, workspace: &Path) -> Result<Session, String> {
        if let Some(id) = id {
            if !super::session::valid_id(id) {
                return Err(format!("\"{id}\" is not a session id."));
            }
            let folder = root.join(id);
            if !folder.is_dir() {
                return Err(format!(
                    "There is no MCP session {id} in {}.",
                    root.display()
                ));
            }
            let lock = super::session::Lock::take(&folder, id)?;
            return Ok(Session {
                id: id.into(),
                folder,
                _lock: Some(Arc::new(lock)),
            });
        }
        let id = crate::model::id();
        let folder = root.join(&id);
        std::fs::create_dir_all(&folder).map_err(|e| {
            format!(
                "Could not make the session folder {}: {e}",
                folder.display()
            )
        })?;
        let lock = super::session::Lock::take(&folder, &id)?;
        let session = Session {
            id,
            folder,
            _lock: Some(Arc::new(lock)),
        };
        if workspace.is_file() {
            std::fs::copy(workspace, session.workspace()).map_err(|e| {
                format!(
                    "Could not copy the workspace {} into the session: {e}",
                    workspace.display()
                )
            })?;
        }
        Ok(session)
    }
    /// The session's own workspace, which its `App` loads and saves.
    pub fn workspace(&self) -> PathBuf {
        self.folder.join("workspace.json")
    }
}

/// A tool call for the session, and where its answer goes.
struct Request {
    tool: String,
    args: Value,
    reply: Reply,
}

/// What the session's thread is woken by.
enum Event {
    Request(Request),
    /// A message a task produced, boxed: a `Message` is large; and whether
    /// the agent started the task.
    Output(Box<Message>, bool),
    /// A message from the window's own handlers: the tests' user.
    #[cfg(test)]
    User(Box<Message>),
    /// The client has gone: the session is saved and the thread ends.
    Stop,
}

/// Sets up the `App` on the session's thread before it starts (tests).
type Prepare = Box<dyn FnOnce(&mut App) + Send>;

/// The handle to a running headless session.
pub struct Runner {
    events: mpsc::Sender<Event>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Runner {
    /// Starts `session` on a thread of its own. The saved part is loaded
    /// as Continue project loads it, and the first tool call waits for it.
    pub fn start(session: Session) -> Runner {
        Self::start_with(session, Box::new(|_| {}))
    }
    fn start_with(session: Session, prepare: Prepare) -> Runner {
        let (events, inbox) = mpsc::channel();
        let own = events.clone();
        let thread = std::thread::Builder::new()
            .name("mcp-session".into())
            .spawn(move || Host::new(session, inbox, own, prepare).serve())
            .expect("the MCP session's thread starts");
        Runner {
            events,
            thread: Some(thread),
        }
    }
    /// Sends a tool call; `reply` gets its answer on the session's thread,
    /// at once for a read, or when the command's work has settled or its
    /// `wait_s` is up.
    pub fn send(&self, tool: String, args: Value, reply: Reply) {
        let request = Request { tool, args, reply };
        if let Err(mpsc::SendError(Event::Request(r))) = self.events.send(Event::Request(request)) {
            (r.reply)(Err("The headless session has stopped.".into()));
        }
    }
    /// Sends `message` as the user's click in a window would: not the
    /// agent's.
    #[cfg(test)]
    pub fn user(&self, message: Message) {
        let _ = self.events.send(Event::User(Box::new(message)));
    }
    /// A tool call's answer, waited for.
    #[cfg(test)]
    pub fn call(&self, tool: &str, args: Value) -> super::engine::Answer {
        self.call_later(tool, args)
            .recv()
            .unwrap_or_else(|_| Err("no answer".into()))
    }
    /// A tool call whose answer comes on the receiver.
    #[cfg(test)]
    pub fn call_later(&self, tool: &str, args: Value) -> mpsc::Receiver<super::engine::Answer> {
        let (tx, rx) = mpsc::channel();
        self.send(
            tool.into(),
            args,
            Box::new(move |a| {
                let _ = tx.send(a);
            }),
        );
        rx
    }
    /// Answers the calls already sent, saves the session's workspace and
    /// ends its thread.
    pub fn stop(mut self) {
        self.end();
    }
    fn end(&mut self) {
        let _ = self.events.send(Event::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
impl Drop for Runner {
    fn drop(&mut self) {
        self.end();
    }
}

/// Counts a task as running until it is dropped, even by a panic.
struct Running(Arc<AtomicUsize>);
impl Drop for Running {
    fn drop(&mut self) {
        self.0.fetch_sub(1, SeqCst);
    }
}

/// The session's thread: its `App` and the event loop round it.
struct Host {
    app: App,
    session: Session,
    inbox: mpsc::Receiver<Event>,
    /// Events taken off the inbox to see whether any came, not yet handled.
    taken: VecDeque<Event>,
    /// Where tasks send the messages they produce.
    events: mpsc::Sender<Event>,
    runtime: tokio::runtime::Runtime,
    /// The tasks whose streams have not ended.
    tasks: Arc<AtomicUsize>,
    last_tick: Instant,
    /// A `Tick` has run since the last other message: the work a `Tick`
    /// starts after a change has been started.
    ticked: bool,
    stopping: bool,
    /// What `session.json` last held (`SessionFile::key`).
    kept: Option<u64>,
}
impl Host {
    fn new(
        session: Session,
        inbox: mpsc::Receiver<Event>,
        events: mpsc::Sender<Event>,
        prepare: Prepare,
    ) -> Host {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .thread_name("mcp-tasks")
            .enable_all()
            .build()
            .expect("the MCP session's runtime starts");
        let mut app = App::from_workspace(session.workspace(), None);
        // Generation, layout, left stock and meshing run in the background
        // as in the window, so a call is answered while they run.
        app.generate_inline = false;
        // A session resumed: what the workspace does not hold.
        let kept = match SessionFile::read(&session.folder) {
            Some(Ok(kept)) => Some(kept),
            Some(Err(e)) => {
                eprintln!("The session's {}: {e}", super::session::FILE);
                None
            }
            None => None,
        };
        if let Some(kept) = &kept {
            kept.put(&mut app);
        }
        prepare(&mut app);
        // A workspace not yet on disk is written now: until it is, every
        // start makes the starter library afresh, with new ids, and a
        // resume would report each material and holder as the user's
        // change.
        if !session.workspace().exists() {
            app.dirty = true;
            app.persist();
        }
        let mut host = Host {
            app,
            session,
            inbox,
            taken: VecDeque::new(),
            events,
            runtime,
            tasks: Arc::new(AtomicUsize::new(0)),
            last_tick: Instant::now(),
            ticked: false,
            stopping: false,
            kept: None,
        };
        host.update(Message::ResumeProject, false);
        // The selection, page and tab as they were, the part loading.
        if let Some(kept) = &kept {
            kept.put_place(&mut host.app);
            host.kept = Some(SessionFile::of(&host.app).key());
        }
        host
    }
    /// Writes `session.json`, and the workspace with it, when what it holds
    /// changed since it was last written (or always, `force`).
    fn keep_session(&mut self, force: bool) {
        let file = SessionFile::of(&self.app);
        let key = file.key();
        if !force && self.kept == Some(key) && !self.app.dirty {
            return;
        }
        self.app.persist_now();
        match file.write(&self.session.folder) {
            Ok(()) => self.kept = Some(key),
            Err(e) => eprintln!(
                "Could not write the session's {}: {e}",
                super::session::FILE
            ),
        }
    }
    /// The event loop: calls, task messages and ticks, and after each the
    /// agent's windows and answers (`Agent::poll`). On the client's end it
    /// answers what is under way, saves the workspace and ends.
    fn serve(mut self) {
        let mut agent = Agent::new();
        loop {
            while let Some(event) = self.next_event(None) {
                self.take(&mut agent, event);
            }
            self.tick_if_due(agent.waiting());
            crate::fault::catch(|| agent.poll(&mut self)).unwrap_or_else(|fault| {
                eprintln!("The MCP session failed while answering: {fault}");
            });
            if self.stopping && agent.idle() {
                break;
            }
            let until = [Some(self.tick_due(agent.waiting())), agent.deadline()]
                .into_iter()
                .flatten()
                .min()
                .unwrap_or_else(Instant::now);
            if let Some(event) = self.next_event(Some(until)) {
                self.take(&mut agent, event);
            }
        }
        // What the last edits did is kept for the next start.
        if self.app.dirty {
            self.app.persist();
        }
        self.keep_session(true);
        self.runtime.shutdown_background();
    }
    /// The next event: one taken already, one waiting, or (with `until`)
    /// one that comes before then.
    fn next_event(&mut self, until: Option<Instant>) -> Option<Event> {
        if let Some(event) = self.taken.pop_front() {
            return Some(event);
        }
        match until {
            None => self.inbox.try_recv().ok(),
            Some(until) => {
                let wait = until.saturating_duration_since(Instant::now());
                match self.inbox.recv_timeout(wait) {
                    Ok(event) => Some(event),
                    Err(mpsc::RecvTimeoutError::Timeout) => None,
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        self.stopping = true;
                        None
                    }
                }
            }
        }
    }
    fn take(&mut self, agent: &mut Agent, event: Event) {
        match event {
            Event::Request(r) => {
                let (tool, args, reply) = (r.tool, r.args, r.reply);
                // A call that panics is answered with what went wrong.
                let answered = std::sync::Arc::new(std::sync::Mutex::new(Some(reply)));
                let once = answered.clone();
                let respond: Reply = Box::new(move |a| {
                    if let Some(reply) = once.lock().ok().and_then(|mut r| r.take()) {
                        reply(a);
                    }
                });
                if let Err(fault) = crate::fault::catch(|| agent.call(self, &tool, args, respond))
                    && let Some(reply) = answered.lock().ok().and_then(|mut r| r.take())
                {
                    reply(Err(format!("The app failed while doing this: {fault}")));
                }
            }
            Event::Output(message, by_agent) => self.update(*message, by_agent),
            #[cfg(test)]
            Event::User(message) => self.update(*message, false),
            Event::Stop => self.stopping = true,
        }
    }
    /// Sends `message` to the `App`, as the agent's or not, and runs the
    /// work it starts as started by the same.
    fn update(&mut self, message: Message, by_agent: bool) {
        let tick = matches!(message, Message::Tick);
        let task = if by_agent {
            self.app.update_by_agent(message)
        } else {
            self.app.update(message)
        };
        self.pump(task, by_agent);
        self.ticked = tick;
    }
    /// Runs `task` on the runtime, its messages sent back to this thread
    /// with who started it. A task with no work in it, as `Tick` mostly
    /// returns (a batch of `Task::none`), is not run: it would count as
    /// running a moment after every `Tick`.
    fn pump(&self, task: Task<Message>, by_agent: bool) {
        use futures_util::StreamExt;
        if task.units() == 0 {
            return;
        }
        let Some(mut stream) = iced_runtime::task::into_stream(task) else {
            return;
        };
        self.tasks.fetch_add(1, SeqCst);
        let running = Running(self.tasks.clone());
        let events = self.events.clone();
        self.runtime.spawn(async move {
            let _running = running;
            while let Some(action) = stream.next().await {
                if let iced_runtime::Action::Output(message) = action {
                    let _ = events.send(Event::Output(Box::new(message), by_agent));
                }
            }
        });
    }
    /// When the next `Tick` is due: at the window's rate, or fast while a
    /// call waits for work.
    fn tick_due(&self, waiting: bool) -> Instant {
        let fast = waiting || self.app.ticks_fast();
        self.last_tick + if fast { FAST_TICK } else { SLOW_TICK }
    }
    fn tick_if_due(&mut self, waiting: bool) {
        let now = Instant::now();
        if now >= self.tick_due(waiting) {
            self.last_tick = now;
            self.update(Message::Tick, false);
        }
    }
}
impl Backend for Host {
    fn app(&self) -> &App {
        &self.app
    }
    fn app_mut(&mut self) -> &mut App {
        &mut self.app
    }
    fn send(&mut self, message: Message) {
        self.update(message, true)
    }
    fn run(&mut self, task: Task<Message>) {
        self.pump(task, true);
        self.ticked = false;
    }
    fn quiet(&mut self) -> bool {
        // Read before the inbox: a task sends its last message before it
        // ends, so with none running and none waiting, none comes.
        let tasks = self.tasks.load(SeqCst);
        if !self.taken.is_empty() {
            return false;
        }
        if let Ok(event) = self.inbox.try_recv() {
            self.taken.push_back(event);
            return false;
        }
        tasks == 0 && self.ticked
    }
    fn pending(&self) -> Vec<&'static str> {
        let mut pending = self.app.pending();
        if self.tasks.load(SeqCst) > 0 {
            pending.push("background results");
        }
        if !self.ticked {
            pending.push("the next tick's work");
        }
        pending
    }
    fn kind(&self) -> &'static str {
        "headless"
    }
    fn session_folder(&self) -> Option<PathBuf> {
        Some(self.session.folder.clone())
    }
    fn session(&self) -> Value {
        json!({
            "id": self.session.id,
            "folder": self.session.folder.display().to_string(),
        })
    }
    fn keep(&mut self) {
        self.keep_session(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Document;
    use std::time::Duration;

    /// A user's workspace holding the sample job, in `dir`.
    fn user_workspace(dir: &Path) -> PathBuf {
        let path = dir.join("user").join("workspace.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut doc = Document::sample();
        doc.adopt_materials();
        doc.adopt_holders();
        doc.title = "The user's job".into();
        crate::storage::save(&path, &doc).unwrap();
        path
    }
    fn active_ops(inspect: &Value) -> Vec<String> {
        let setups = inspect["setups"].as_array().unwrap();
        let active = setups.iter().find(|s| s["active"] == true).unwrap();
        (active["operations"].as_array().unwrap().iter())
            .map(|o| o["id"].as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn a_session_starts_from_a_copy_and_settles_after_add_operation() {
        let dir = tempfile::tempdir().unwrap();
        let user = user_workspace(dir.path());
        let saved = std::fs::read(&user).unwrap();
        let root = dir.path().join("mcp-sessions");
        let session = Session::open(&root, None, &user).unwrap();
        assert!(session.folder.starts_with(&root));
        assert_eq!(std::fs::read(session.workspace()).unwrap(), saved);
        let runner = Runner::start(session.clone());

        let before = runner.call("inspect", json!({})).unwrap();
        assert_eq!(before["project"]["title"], "The user's job");
        assert_eq!(before["backend"], "headless");
        assert_eq!(before["session"]["id"], session.id.as_str());
        assert_eq!(before["pending"], json!([]), "{before}");
        let ops = active_ops(&before);

        let added = runner
            .call(
                "execute",
                json!({"command": "add_operation", "args": {"strategy": "Pocket"}}),
            )
            .unwrap();
        assert_eq!(added["outcome"], "done", "{added}");
        assert_eq!(added["backend"], "headless");
        assert_eq!(added["pending"], json!([]), "{added}");
        let after = runner.call("inspect", json!({})).unwrap();
        let now = active_ops(&after);
        assert_eq!(now.len(), ops.len() + 1, "{after}");
        let new = now.iter().find(|id| !ops.contains(id)).unwrap();
        assert_eq!(added["selection"]["operation"], new.as_str());
        assert_eq!(added["selection"]["page"], "Work");

        // A title: done, then the same again changes nothing.
        let titled = json!({"command": "set_project_title", "args": {"title": "Bracket"}});
        assert_eq!(
            runner.call("execute", titled.clone()).unwrap()["outcome"],
            "done"
        );
        assert_eq!(
            runner.call("execute", titled).unwrap()["outcome"],
            "no_change"
        );
        // Selecting by index, and a setup that does not exist.
        let first = json!({"command": "select_operation", "args": {"operation": 0}});
        let selected = runner.call("execute", first).unwrap();
        assert_eq!(selected["outcome"], "done", "{selected}");
        assert_eq!(selected["selection"]["operation"], ops[0].as_str());
        let missing = json!({"command": "select_setup", "args": {"setup": "no-such-id"}});
        let refused = runner.call("execute", missing).unwrap();
        assert_eq!(refused["outcome"], "refused");
        assert!(refused["reason"].as_str().unwrap().contains("no-such-id"));
        // An unknown command is an error of the call, not an outcome.
        assert!(runner.call("execute", json!({"command": "fly"})).is_err());
        runner.stop();

        // The session saved its own copy; the user's workspace is as it was.
        assert_eq!(std::fs::read(&user).unwrap(), saved);
        let kept = crate::storage::load(&session.workspace()).unwrap();
        assert_eq!(kept.title, "Bracket");
        assert_eq!(kept.setup().operations.len(), ops.len() + 1);
        // The session can be opened again by its id, without a new copy,
        // once this process lets it go.
        let (id, folder) = (session.id.clone(), session.folder.clone());
        assert!(
            Session::open(&root, Some(&id), &user).is_err(),
            "open in this process still"
        );
        drop(session);
        let again = Session::open(&root, Some(&id), &user).unwrap();
        assert_eq!(again.folder, folder);
        assert!(Session::open(&root, Some("../x"), &user).is_err());
        assert!(Session::open(&root, Some("missing"), &user).is_err());
    }

    /// A session on the fixture block, its material chosen, whose
    /// generations each take `slow` before they work.
    fn slow_session(dir: &Path, slow: std::time::Duration) -> Runner {
        let user = dir.join("user").join("workspace.json");
        std::fs::create_dir_all(user.parent().unwrap()).unwrap();
        let (mut doc, _) = crate::post::fixture::job();
        // Its Face alone: the checks after each edit stay quick.
        doc.setups[0].operations.truncate(1);
        doc.adopt_materials();
        doc.adopt_holders();
        let aluminium = (doc.materials.iter())
            .find(|m| m.name() == "Aluminium 6061-T6")
            .map(|m| m.id.clone())
            .unwrap();
        assert!(doc.select_material(&aluminium));
        crate::storage::save(&user, &doc).unwrap();
        let session = Session::open(&dir.join("mcp-sessions"), None, &user).unwrap();
        Runner::start_with(
            session,
            Box::new(move |app| app.slow_generation = Some(slow)),
        )
    }

    #[test]
    fn slow_work_answers_in_time_and_holds_the_window() {
        let dir = tempfile::tempdir().unwrap();
        let runner = slow_session(dir.path(), std::time::Duration::from_secs(2));
        // The first call waits for the saved part to be restored.
        let start = runner.call("inspect", json!({})).unwrap();
        assert!(start["project"]["model"].is_object(), "{start}");
        assert_eq!(start["pending"], json!([]), "{start}");
        let pick = json!({"command": "select_operation", "args": {"operation": 0}});
        assert_eq!(runner.call("execute", pick).unwrap()["outcome"], "done");

        // A generation longer than wait_s answers still_running, in time.
        let asked = std::time::Instant::now();
        let generate = json!({"command": "generate", "wait_s": 0.3});
        let first = runner.call("execute", generate.clone()).unwrap();
        assert!(
            asked.elapsed() < std::time::Duration::from_secs(2),
            "{first}"
        );
        assert_eq!(first["outcome"], "still_running", "{first}");
        let id = first["command_id"].as_str().unwrap().to_string();
        // Its window is open: reads answer, a mutation is busy.
        let read = runner.call("inspect", json!({})).unwrap();
        assert!(
            read["pending"]
                .as_array()
                .unwrap()
                .contains(&json!("generation")),
            "{read}"
        );
        let title = json!({"command": "set_project_title", "args": {"title": "Busy"}});
        let busy = runner.call("execute", title).unwrap();
        assert_eq!(busy["outcome"], "busy", "{busy}");
        assert!(busy["reason"].as_str().unwrap().contains(&id), "{busy}");
        assert_ne!(
            runner.call("inspect", json!({})).unwrap()["project"]["title"],
            "Busy"
        );
        // wait gives the final result, and keeps it.
        let done = runner
            .call("wait", json!({"command_id": id, "wait_s": 45}))
            .unwrap();
        assert_eq!(done["outcome"], "done", "{done}");
        assert_eq!(done["command_id"], id.as_str());
        let again = runner
            .call("wait", json!({"command_id": id, "wait_s": 0}))
            .unwrap();
        assert_eq!(again, done);

        // A generation cancelled ends cancelled.
        let second = runner.call("execute", generate).unwrap();
        assert_eq!(second["outcome"], "still_running", "{second}");
        let id = second["command_id"].as_str().unwrap();
        let cancelled = runner
            .call("cancel", json!({"command_id": id, "wait_s": 45}))
            .unwrap();
        assert_eq!(cancelled["outcome"], "cancelled", "{cancelled}");

        // A batch stops dispatching once its time is up.
        let step = |t: &str| json!({"command": "set_project_title", "args": {"title": t}});
        let steps = json!({"steps": [step("A"), step("B"), step("C")], "wait_s": 0.2});
        let batch = runner.call("batch", steps).unwrap();
        assert_eq!(batch["outcome"], "still_running", "{batch}");
        let steps = batch["steps"].as_array().unwrap();
        assert_eq!(steps[0]["outcome"], "still_running");
        assert_eq!(
            (steps[1]["outcome"].as_str(), steps[2]["outcome"].as_str()),
            (Some("not_run"), Some("not_run"))
        );
        let id = steps[0]["command_id"].as_str().unwrap();
        let first = runner.call("wait", json!({"command_id": id})).unwrap();
        assert_eq!(first["outcome"], "done", "{first}");
        let title = runner.call("inspect", json!({})).unwrap()["project"]["title"].clone();
        assert_eq!(title, "A");
        // A batch with the time it needs runs every step.
        let steps = json!({"steps": [step("B"), step("B"), step("C")]});
        let batch = runner.call("batch", steps).unwrap();
        assert_eq!(batch["outcome"], "done", "{batch}");
        let outcomes: Vec<&str> = (batch["steps"].as_array().unwrap().iter())
            .map(|s| s["outcome"].as_str().unwrap())
            .collect();
        assert_eq!(outcomes, ["done", "no_change", "done"]);
        runner.stop();
    }

    /// Picks in a session: picking comes on, the part's holes for picking
    /// are worked out off the thread, then the clicks are made; the tab and
    /// picking are as they were, and the same picks again change nothing.
    #[test]
    fn a_session_picks_a_hole_by_two_of_its_walls() {
        let dir = tempfile::tempdir().unwrap();
        // `pocket::chamfered_hole` as an STL file, a Pocket of the sample's
        // setup on it.
        let mut text = String::from("solid hole\n");
        for t in crate::pocket::chamfered_hole() {
            text += "facet normal 0 0 0\nouter loop\n";
            for p in t {
                text += &format!("vertex {} {} {}\n", p[0], p[1], p[2]);
            }
            text += "endloop\nendfacet\n";
        }
        text += "endsolid hole\n";
        let stl = dir.path().join("hole.stl");
        std::fs::write(&stl, &text).unwrap();
        let mut doc = Document::sample();
        doc.adopt_materials();
        doc.adopt_holders();
        doc.sample_model = false;
        doc.file = Some(crate::model::ModelFile {
            name: "hole.stl".into(),
            path: stl.clone(),
            format: "stl".into(),
            bytes: text.len() as u64,
            thickness: None,
        });
        let s = &mut doc.setups[0];
        s.operations.truncate(1);
        s.operations[0] = crate::model::Operation::new(
            crate::model::Strategy::Pocket,
            s.operations[0].tool.clone(),
        );
        let user = dir.path().join("user").join("workspace.json");
        std::fs::create_dir_all(user.parent().unwrap()).unwrap();
        crate::storage::save(&user, &doc).unwrap();
        let session = Session::open(&dir.path().join("mcp-sessions"), None, &user).unwrap();
        let runner = Runner::start(session);
        let start = runner.call("inspect", json!({})).unwrap();
        assert!(start["project"]["model"].is_object(), "{start}");
        // The hole's walls as the session's part has them.
        let mesh = crate::mesh::load(&stl).unwrap();
        let holes = crate::pocket::part_holes(&doc.setups[0], &mesh);
        let hole = holes.iter().find(|h| h.walls.len() == 24).unwrap();
        let call = json!({
            "command": "pick_surfaces",
            "args": {"operation": 0, "ids": [hole.walls[0], hole.walls[9]], "mode": "add"},
        });
        let picked = runner.call("execute", call.clone()).unwrap();
        assert_eq!(picked["outcome"], "done", "{picked}");
        assert_eq!(
            picked["picks"]["ids"].as_array().unwrap().len(),
            24,
            "{picked}"
        );
        assert_eq!(
            picked["picks"]["holes"].as_array().unwrap().len(),
            1,
            "{picked}"
        );
        let clicks: Vec<&str> = (picked["clicks"].as_array().unwrap().iter())
            .map(|c| c["did"].as_str().unwrap())
            .collect();
        assert_eq!(clicks, ["added", "already picked"], "{picked}");
        assert_eq!(picked["pending"], json!([]), "{picked}");
        let read = runner.call("inspect", json!({})).unwrap();
        assert_eq!(read["selection"]["tab"], "Tool", "{read}");
        let again = runner.call("execute", call).unwrap();
        assert_eq!(again["outcome"], "no_change", "{again}");
        assert_eq!(again["revision"], picked["revision"], "{again}");
        runner.stop();
    }

    /// The freshness rule (LIO-470): `generate` is still running, a
    /// `check_job` starts on a snapshot of the job, and the user changes a
    /// clamp position in the window before it is done. The check is refused
    /// as stale, saying what changed; asked again it checks the job as it is.
    #[test]
    fn a_check_whose_job_the_user_changed_meanwhile_is_refused_as_stale() {
        let dir = tempfile::tempdir().unwrap();
        let user = dir.path().join("user").join("workspace.json");
        std::fs::create_dir_all(user.parent().unwrap()).unwrap();
        let (mut doc, _) = crate::post::fixture::job();
        doc.setups[0].operations.truncate(1);
        doc.adopt_materials();
        doc.adopt_holders();
        let aluminium = (doc.materials.iter())
            .find(|m| m.name() == "Aluminium 6061-T6")
            .map(|m| m.id.clone())
            .unwrap();
        assert!(doc.select_material(&aluminium));
        crate::proposal::tests::hold(&mut doc, "Clamp", &[("clamps", "-X:0;+X:0")]);
        doc.active = doc.setups[0].id.clone();
        crate::storage::save(&user, &doc).unwrap();
        let session = Session::open(&dir.path().join("mcp-sessions"), None, &user).unwrap();
        let runner = Runner::start_with(
            session,
            Box::new(|app| {
                app.slow_generation = Some(std::time::Duration::from_secs(3));
                app.mcp.slow_reads = Some(std::time::Duration::from_millis(1500));
            }),
        );
        let start = runner.call("inspect", json!({})).unwrap();
        assert!(start["project"]["model"].is_object(), "{start}");
        let pick = json!({"command": "select_operation", "args": {"operation": 0}});
        assert_eq!(runner.call("execute", pick).unwrap()["outcome"], "done");
        let generate = json!({"command": "generate", "wait_s": 0.2});
        let generating = runner.call("execute", generate).unwrap();
        assert_eq!(generating["outcome"], "still_running", "{generating}");
        let id = generating["command_id"].as_str().unwrap().to_string();
        let check = runner.call_later("check_job", json!({"wait_s": 45}));
        runner.user(Message::SetClamps("-Y:0;+Y:0".into()));
        let checked = check.recv().unwrap().unwrap();
        assert_eq!(checked["outcome"], "refused", "{checked}");
        assert_eq!(checked["stale"], true, "{checked}");
        assert!(
            checked["reason"].as_str().unwrap().contains("changed"),
            "{checked}"
        );
        let changed = checked["changed_meanwhile"].as_array().unwrap();
        assert!(
            changed
                .iter()
                .any(|c| c["kind"] == "setup" && c["what"] == "changed"),
            "{checked}"
        );
        assert!(checked.get("passed").is_none(), "nothing of it reported");
        let done = runner
            .call("wait", json!({"command_id": id, "wait_s": 45}))
            .unwrap();
        assert_ne!(done["outcome"], "still_running", "{done}");
        // Unchanged meanwhile, the check is of the job as it is.
        let again = runner.call("check_job", json!({"wait_s": 45})).unwrap();
        assert_eq!(again["outcome"], "done", "{again}");
        assert_eq!(again["stale"], false, "{again}");
        runner.stop();
    }

    /// The user's edit in the window while the agent's generation runs:
    /// the generation is dropped, and the agent is told the job changed by
    /// both, the edit named. Then an edit with no window open is the user's.
    #[test]
    fn a_user_edit_during_the_agents_window_is_named() {
        use crate::{Target, mcp::state::Entry};
        let dir = tempfile::tempdir().unwrap();
        let runner = slow_session(dir.path(), std::time::Duration::from_secs(20));
        let start = runner.call("inspect", json!({})).unwrap();
        let revision = start["revision"].as_u64().unwrap();
        let pick = json!({"command": "select_operation", "args": {"operation": 0}});
        assert_eq!(runner.call("execute", pick).unwrap()["outcome"], "done");
        let generate = json!({"command": "generate", "wait_s": 0.2});
        let first = runner.call("execute", generate).unwrap();
        assert_eq!(first["outcome"], "still_running", "{first}");
        let id = first["command_id"].as_str().unwrap();
        runner.user(Message::Edit(Target::Operation, "feed", "555".into()));
        let done = runner
            .call("wait", json!({"command_id": id, "wait_s": 15}))
            .unwrap();
        assert_ne!(done["outcome"], "still_running", "{done}");
        let theirs: Vec<Entry> = serde_json::from_value(done["user_changed"].clone()).unwrap();
        let [e] = &theirs[..] else { panic!("{done}") };
        assert_eq!(e.source, crate::mcp::state::Source::Mixed);
        assert_eq!(e.events, ["user: edit_field operation feed"]);
        assert!(e.revision > revision);
        // With no window open, a read finds the user's next edit.
        runner.user(Message::Title("Renamed by hand".into()));
        let read = runner.call("inspect", json!({})).unwrap();
        let theirs: Vec<Entry> = serde_json::from_value(read["user_changed"].clone()).unwrap();
        let [e] = &theirs[..] else { panic!("{read}") };
        assert_eq!(e.source, crate::mcp::state::Source::User);
        assert_eq!(read["project"]["title"], "Renamed by hand");
        runner.stop();
    }

    /// A session on the sample job, its tool draft edited and an operation
    /// added by the agent, the second operation selected; then stopped as
    /// at the client's end. Its id and the user's workspace.
    fn worked_session(root: &Path, dir: &Path) -> (String, PathBuf) {
        let user = user_workspace(dir);
        let session = Session::open(root, None, &user).unwrap();
        let id = session.id.clone();
        let runner = Runner::start(session);
        let edit = json!({"command": "edit_field", "args": {"target": "tool", "key": "name", "value": "Draft kept"}});
        let r = runner.call("execute", edit).unwrap();
        assert_eq!(r["outcome"], "done", "{r}");
        let add = json!({"command": "add_operation", "args": {"strategy": "Pocket"}});
        assert_eq!(runner.call("execute", add).unwrap()["outcome"], "done");
        let pick = json!({"command": "select_operation", "args": {"operation": 1}});
        let r = runner.call("execute", pick).unwrap();
        assert_eq!(r["outcome"], "done", "{r}");
        // Every result names the session and what is saved.
        assert_eq!(r["session"]["id"], id.as_str(), "{r}");
        assert_eq!(r["session"]["saved_to_project_file"], false, "{r}");
        // What it reported was on disk before it was sent.
        let kept = SessionFile::read(&root.join(&id)).unwrap().unwrap();
        assert_eq!(kept.log.revision, r["revision"].as_u64().unwrap());
        assert_eq!(
            kept.place.operation.as_deref(),
            r["selection"]["operation"].as_str()
        );
        let saved = crate::storage::load(&root.join(&id).join("workspace.json")).unwrap();
        assert_eq!(saved.setup().operations.len(), 5);
        runner.stop();
        (id, user)
    }
    fn tool_name(runner: &Runner) -> Value {
        let fields = runner
            .call("list_fields", json!({"target": "tool"}))
            .unwrap();
        let name = (fields["fields"].as_array().unwrap().iter()).find(|f| f["key"] == "name");
        name.unwrap()["value"].clone()
    }

    /// A session resumed after the client ended: the draft, the selection,
    /// the revision and the change log are back, and nothing new is said
    /// to have changed.
    #[test]
    fn a_resumed_session_is_where_it_was_left() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("mcp-sessions");
        let (id, user) = worked_session(&root, dir.path());
        let folder = root.join(&id);
        let kept = SessionFile::read(&folder).unwrap().unwrap();
        assert!(kept.log.revision > 0, "{kept:?}");
        assert!(!folder.join("lock").exists(), "let go at the end");
        let session = Session::open(&root, Some(&id), &user).unwrap();
        let runner = Runner::start(session);
        let read = runner.call("inspect", json!({})).unwrap();
        assert_eq!(read["revision"], kept.log.revision, "{read}");
        assert_eq!(read["user_changed"], json!([]), "{read}");
        assert_eq!(
            read["selection"]["operation"],
            kept.place.operation.as_deref().unwrap()
        );
        assert_eq!(read["selection"]["page"], kept.place.page.as_str());
        assert_eq!(tool_name(&runner), "Draft kept");
        runner.stop();
        let again = SessionFile::read(&folder).unwrap().unwrap();
        assert_eq!(again.log.entries, kept.log.entries);
        assert_eq!(again.log.revision, kept.log.revision);
    }

    /// One process at a time: a second open of a session open in this
    /// process is refused, naming the pid; a lock whose owner is dead is
    /// taken over.
    #[test]
    fn a_session_is_open_in_one_server_at_a_time() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("mcp-sessions");
        let user = user_workspace(dir.path());
        let first = Session::open(&root, None, &user).unwrap();
        let runner = Runner::start(first.clone());
        let second = Session::open(&root, Some(&first.id), &user).unwrap_err();
        assert_eq!(
            second,
            format!(
                "Session {} is open in another MCP server (pid {}).",
                first.id,
                std::process::id()
            )
        );
        runner.stop();
        // A lock left by a process that is gone: an unknown pid, a start
        // time no process has, and not refreshed for an hour.
        let (id, folder) = (first.id.clone(), first.folder.clone());
        drop(first);
        let lock = folder.join("lock");
        let dead = json!({"pid": 999_999, "started": "1.000001", "at": 1});
        std::fs::write(&lock, dead.to_string()).unwrap();
        let old = std::time::SystemTime::now() - Duration::from_secs(3600);
        std::fs::File::options()
            .write(true)
            .open(&lock)
            .unwrap()
            .set_modified(old)
            .unwrap();
        let taken = Session::open(&root, Some(&id), &user).unwrap();
        let owner: Value = serde_json::from_slice(&std::fs::read(&lock).unwrap()).unwrap();
        assert_eq!(owner["pid"], std::process::id());
        drop(taken);
        assert!(!lock.exists());
    }

    /// An unsaved job with an open tool draft is kept through a restart
    /// with the clock 31 days on: nothing deletes a session on its own.
    /// It is listed, and resumed through the session tools.
    #[test]
    fn an_unsaved_session_survives_31_days() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("mcp-sessions");
        let (id, user) = worked_session(&root, dir.path());
        let folder = root.join(&id);
        let month = std::time::SystemTime::now() - Duration::from_secs(31 * 86_400);
        fn age(path: &Path, to: std::time::SystemTime) {
            if path.is_dir() {
                for e in std::fs::read_dir(path).unwrap().flatten() {
                    age(&e.path(), to);
                }
            } else {
                let f = std::fs::File::options().write(true).open(path).unwrap();
                f.set_modified(to).unwrap();
            }
        }
        age(&folder, month);
        // A new server starts, makes a session of its own, and lists both.
        let mut sessions = super::super::session::Sessions::new(root.clone(), user.clone(), None);
        let ask = |sessions: &mut super::super::session::Sessions, tool: &str, args: Value| {
            let (tx, rx) = mpsc::channel();
            sessions.call(tool, args, Box::new(move |a| tx.send(a).unwrap()));
            rx.recv().unwrap().unwrap()
        };
        let fresh = ask(&mut sessions, "inspect", json!({}));
        assert_ne!(fresh["session"]["id"], id.as_str());
        let listed = ask(&mut sessions, "list_sessions", json!({}));
        let all = listed["sessions"].as_array().unwrap();
        assert_eq!(all.len(), 2, "{listed}");
        let old = all.iter().find(|s| s["id"] == id.as_str()).unwrap();
        assert_eq!(old["saved_to_project_file"], false, "{old}");
        assert_eq!(old["open"], false, "{old}");
        assert_eq!(old["title"], "The user's job", "{old}");
        assert!(old["size_bytes"].as_u64().unwrap() > 0);
        let resumed = ask(&mut sessions, "resume_session", json!({"id": id}));
        assert_eq!(resumed["resumed"], id.as_str(), "{resumed}");
        assert_eq!(resumed["session"]["id"], id.as_str(), "{resumed}");
        let ops = active_ops(&resumed);
        assert_eq!(
            resumed["selection"]["operation"],
            ops[1].as_str(),
            "{resumed}"
        );
        let fields = ask(&mut sessions, "list_fields", json!({"target": "tool"}));
        let name = (fields["fields"].as_array().unwrap().iter()).find(|f| f["key"] == "name");
        assert_eq!(name.unwrap()["value"], "Draft kept");
        sessions.stop();
        assert!(folder.join("workspace.json").is_file());
    }

    /// delete_session: refused while the session is open and while its job
    /// is not saved to a project file, unless discarded; a saved one goes.
    #[test]
    fn delete_session_refuses_what_would_lose_work() {
        use super::super::session::delete;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("mcp-sessions");
        let user = user_workspace(dir.path());
        let session = Session::open(&root, None, &user).unwrap();
        let (id, folder) = (session.id.clone(), session.folder.clone());
        let runner = Runner::start(session);
        let open = delete(&root, &id, true).unwrap();
        assert_eq!(open["outcome"], "refused", "{open}");
        assert!(
            open["reason"]
                .as_str()
                .unwrap()
                .contains("open in another MCP server")
        );
        let project = dir.path().join("job.axis.json");
        let save =
            json!({"command": "save_project", "args": {"path": project.display().to_string()}});
        let saved = runner.call("execute", save).unwrap();
        assert_eq!(saved["outcome"], "done", "{saved}");
        assert_eq!(saved["session"]["saved_to_project_file"], true, "{saved}");
        let outputs = &saved["session"]["last_saves_and_exports"];
        assert_eq!(outputs[0]["path"], project.display().to_string(), "{saved}");
        let title = json!({"command": "set_project_title", "args": {"title": "Unsaved"}});
        assert_eq!(runner.call("execute", title).unwrap()["outcome"], "done");
        runner.stop();
        let unsaved = delete(&root, &id, false).unwrap();
        assert_eq!(unsaved["outcome"], "refused", "{unsaved}");
        assert!(
            unsaved["reason"].as_str().unwrap().contains("not saved"),
            "{unsaved}"
        );
        assert!(folder.is_dir());
        assert!(!folder.join("lock").exists(), "the refusal let go of it");
        // Saved: it goes without asking.
        let session = Session::open(&root, Some(&id), &user).unwrap();
        let runner = Runner::start(session);
        let r = runner
            .call("execute", json!({"command": "save_project"}))
            .unwrap();
        assert_eq!(r["outcome"], "done", "{r}");
        runner.stop();
        let gone = delete(&root, &id, false).unwrap();
        assert_eq!(gone["outcome"], "done", "{gone}");
        assert!(!folder.exists());
        // An unsaved one goes when asked to discard it.
        let other = Session::open(&root, None, &user).unwrap();
        let other_id = other.id.clone();
        drop(other);
        let r = delete(&root, &other_id, true).unwrap();
        assert_eq!(r["outcome"], "done", "{r}");
        assert!(delete(&root, "missing", true).is_err());
    }
}
