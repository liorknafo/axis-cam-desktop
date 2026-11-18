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
    collections::{HashSet, VecDeque},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst},
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
    /// Session `id` in `root`, whose folder must be there with its job, and
    /// which no other server has open (`session::Lock`), or with no id a new
    /// session, which starts from a copy of the user's `workspace` (when
    /// there is one) so it has their libraries. The user's workspace is
    /// only read: the session's `App` saves its own copy.
    pub fn open(root: &Path, id: Option<&str>, workspace: &Path) -> Result<Session, String> {
        if let Some(id) = id {
            if !super::session::valid_id(id) {
                return Err(format!("\"{id}\" is not a session id."));
            }
            let folder = root.join(id);
            let missing = || format!("There is no MCP session {id} in {}.", root.display());
            if !folder.is_dir() {
                return Err(missing());
            }
            let lock = super::session::Lock::take(&folder, id)
                .map_err(|why| if folder.is_dir() { why } else { missing() })?;
            // Locked before anything in it is read. `delete` removes the
            // workspace and the record first, and holds the lock through
            // the rest: a folder without either is being deleted or gone,
            // and is never started, so never written again.
            if !super::session::has_job(&folder) {
                return Err(format!(
                    "Session {id} is being deleted, or is gone: it holds no job."
                ));
            }
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
    /// The session is committed to staying headless though a window opens
    /// (`session::Sessions`): it has dispatched work that may change its
    /// job (`Backend::issued`), or its revision is above 0. Set at the
    /// dispatch, on the session's thread, before any answer about it goes;
    /// never cleared.
    committed: Arc<AtomicBool>,
    /// The `command_id`s the session has given out: `wait` and `cancel` for
    /// one of them go to it whatever window is open.
    issued: Arc<Mutex<HashSet<String>>>,
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
        // A session resumed with changes has them from the start.
        let before = SessionFile::read(&session.folder)
            .and_then(Result::ok)
            .is_some_and(|k| k.log.revision > 0);
        let committed = Arc::new(AtomicBool::new(before));
        let issued = Arc::new(Mutex::new(HashSet::new()));
        let (flag, ids) = (committed.clone(), issued.clone());
        let thread = std::thread::Builder::new()
            .name("mcp-session".into())
            .spawn(move || Host::new(session, inbox, own, flag, ids, prepare).serve())
            .expect("the MCP session's thread starts");
        Runner {
            events,
            thread: Some(thread),
            committed,
            issued,
        }
    }
    /// Whether the session is committed to staying headless: it has
    /// dispatched a change, or has one from before it was resumed.
    pub fn committed(&self) -> bool {
        self.committed.load(SeqCst)
    }
    /// Whether the session gave out `command_id` `id`.
    pub fn knows(&self, id: &str) -> bool {
        self.issued.lock().is_ok_and(|ids| ids.contains(id))
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
    /// `Runner::committed`: set at a dispatch that may change the job, and
    /// once the revision is above 0.
    committed: Arc<AtomicBool>,
    /// `Runner::issued`.
    issued: Arc<Mutex<HashSet<String>>>,
}
impl Host {
    fn new(
        session: Session,
        inbox: mpsc::Receiver<Event>,
        events: mpsc::Sender<Event>,
        committed: Arc<AtomicBool>,
        issued: Arc<Mutex<HashSet<String>>>,
        prepare: Prepare,
    ) -> Host {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .thread_name("mcp-tasks")
            .enable_all()
            .build()
            .expect("the MCP session's runtime starts");
        // Whether the workspace is the one `session.json` was written with.
        let on_disk = std::fs::read(session.workspace())
            .ok()
            .map(|b| super::session::fingerprint(&b));
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
        // Whether the record goes with the workspace on disk. Not when the
        // workspace was written after it (its own write failed, or the
        // process ended first): the difference is the agent's, and what the
        // record holds of the job it was written with (its project file,
        // drafts, selection, checkpoints' project) may be another job's.
        let paired = kept
            .as_ref()
            .is_some_and(|k| k.workspace.is_none() || k.workspace == on_disk);
        if let Some(kept) = &kept {
            if paired {
                kept.put(&mut app);
            } else {
                kept.put_unpaired(&mut app);
            }
            app.mcp.unrecorded = !paired;
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
            committed,
            issued,
        };
        host.update(Message::ResumeProject, false);
        // The selection, page and tab as they were, the part loading.
        if let Some(kept) = kept.as_ref().filter(|_| paired) {
            kept.put_place(&mut host.app);
            host.kept = Some(SessionFile::of(&host.app).key());
        }
        host
    }
    /// Writes the workspace and then `session.json`, when what they hold
    /// changed since they were last written (or always, `force`). The
    /// `Document` goes first and `session.json` only after it is on disk,
    /// with the workspace's fingerprint: a failed `Document` leaves the pair
    /// on disk as it was, and a failed `session.json` is told on resume by
    /// the fingerprint. What failed, if anything; it is tried again at the
    /// next answer.
    fn keep_session(&mut self, force: bool) -> Result<(), String> {
        if self.app.mcp.log.revision > 0 {
            self.committed.store(true, SeqCst);
        }
        let mut file = SessionFile::of(&self.app);
        let key = file.key();
        if !force && self.kept == Some(key) && !self.app.unsaved() {
            return Ok(());
        }
        if !self.app.persist_now() {
            let why = self.app.save_error.clone().unwrap_or_default();
            return Err(format!("its workspace.json was not written ({why})"));
        }
        // The workspace as it is on disk now, which this record goes with.
        let doc = std::fs::read(self.session.workspace())
            .map_err(|e| format!("its workspace.json could not be read back ({e})"))?;
        file.workspace = Some(super::session::fingerprint(&doc));
        file.write(&self.session.folder)
            .map_err(|e| format!("its {} was not written ({e})", super::session::FILE))?;
        self.kept = Some(key);
        Ok(())
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
        if let Err(why) = self.keep_session(true) {
            eprintln!("The session could not be saved at its end: {why}.");
        }
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
    fn keep(&mut self) -> Result<(), String> {
        self.keep_session(false)
    }
    fn issued(&mut self, id: &str, changes: bool) {
        if changes {
            self.committed.store(true, SeqCst);
        }
        if let Ok(mut ids) = self.issued.lock() {
            ids.insert(id.to_string());
        }
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
        assert_let_go(&folder, "let go at the end");
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
        // A lock file left by a process that is gone, as the old lock files
        // were: nothing holds it, so it is taken at once, never deleted,
        // and signed by its new holder.
        let (id, folder) = (first.id.clone(), first.folder.clone());
        drop(first);
        let lock = folder.join("lock");
        assert!(lock.is_file(), "the lock file stays with its session");
        assert_let_go(&folder, "let go with the session");
        let dead = json!({"pid": 999_999, "started": "1.000001", "at": 1});
        std::fs::write(&lock, dead.to_string()).unwrap();
        let taken = Session::open(&root, Some(&id), &user).unwrap();
        assert!(held(&folder));
        let owner: Value = serde_json::from_slice(&std::fs::read(&lock).unwrap()).unwrap();
        assert_eq!(owner["pid"], std::process::id());
        drop(taken);
        assert!(lock.is_file(), "not deleted");
        assert_let_go(&folder, "let go");
    }

    /// Whether some open of the session in `folder` holds its lock: its
    /// lock file cannot be held shared now.
    fn held(folder: &Path) -> bool {
        let file = std::fs::File::open(folder.join("lock")).unwrap();
        matches!(
            file.try_lock_shared(),
            Err(std::fs::TryLockError::WouldBlock)
        )
    }

    /// Asserts that this process has let go of the lock of the session in
    /// `folder` (`why` says when), then waits until no process holds it.
    ///
    /// A process another test starts (the STEP converter, `open`, `kill`)
    /// gets a copy of every open file of this one, the lock file's too,
    /// until its program starts and the copies close (they are
    /// close-on-exec). The `flock` is let go only when the last copy
    /// closes, so the lock can stay held a moment after a dropped
    /// `Session` closed its file (LIO-566: 1 round in 5 with 4 threads
    /// starting `true`). So what this process holds is told by its own
    /// open files, right after the drop: none of them is the lock file.
    /// The wait then is for a starting program only, so the next open
    /// does not meet the copy.
    #[track_caller]
    fn assert_let_go(folder: &Path, why: &str) {
        #[cfg(unix)]
        assert!(
            !open_here(&folder.join("lock")),
            "{why}: this process still has the lock file open"
        );
        // Windows passes no open file to a program it starts: held is held.
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        while held(folder) {
            assert!(
                cfg!(unix) && std::time::Instant::now() < deadline,
                "{why}: the lock is still held"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// `assert_let_go` is no weaker than the check it replaced: a session
    /// still open is caught at once, not waited for.
    #[test]
    #[should_panic(expected = "kept: this process still has the lock file open")]
    #[cfg(unix)]
    fn a_session_still_open_is_not_let_go() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("mcp-sessions");
        let user = user_workspace(dir.path());
        let session = Session::open(&root, None, &user).unwrap();
        let kept = session.clone();
        drop(session);
        assert_let_go(&kept.folder, "kept");
    }

    /// Whether one of this process's open files is the file at `path`:
    /// each number in `/dev/fd` asked with `fstat`, never duplicated or
    /// closed (a duplicate would hold its lock too). A number closed since
    /// it was listed (the listing's own, or another thread's) fails
    /// `fstat`; one opened again since is some other file.
    #[cfg(unix)]
    fn open_here(path: &Path) -> bool {
        use std::os::unix::fs::MetadataExt;
        let want = std::fs::metadata(path).unwrap();
        let fds: Vec<i32> = (std::fs::read_dir("/dev/fd").unwrap().flatten())
            .filter_map(|e| e.file_name().to_str()?.parse().ok())
            .collect();
        fds.into_iter().any(|fd| {
            // SAFETY: `fstat` only writes the `stat` it is given, all
            // zeroes a valid value; a closed number is EBADF, not UB.
            let mut stat: libc::stat = unsafe { std::mem::zeroed() };
            let got = unsafe { libc::fstat(fd, &raw mut stat) };
            got == 0 && (stat.st_dev as u64, stat.st_ino as u64) == (want.dev(), want.ino())
        })
    }

    /// The reviewer's race (LIO-470): many opens of one session at once, its
    /// lock file left by a dead process. The lock is the OS's, held for the
    /// session's life: while one holds it every other open is refused and
    /// writes nothing, the lock file included; racing, exactly one wins;
    /// once it lets go, the next takes it. (`flock` locks an open of the
    /// file, not a process: threads race here as processes do; the
    /// processes' race is in `tests/mcp_stdio.rs`.) Programs are started
    /// all through, as other tests start them in a parallel run: each gets
    /// a moment's copy of the lock file while a session has it open
    /// (LIO-566, `assert_let_go`).
    #[test]
    fn of_many_opens_racing_for_a_session_one_wins() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("mcp-sessions");
        let user = user_workspace(dir.path());
        let first = Session::open(&root, None, &user).unwrap();
        let runner = Runner::start(first.clone());
        let title = json!({"command": "set_project_title", "args": {"title": "Raced"}});
        assert_eq!(runner.call("execute", title).unwrap()["outcome"], "done");
        runner.stop();
        let (id, folder) = (first.id.clone(), first.folder.clone());
        drop(first);
        let dead = json!({"pid": 2_147_483_000u32, "started": "dead", "at": 0});
        std::fs::write(folder.join("lock"), dead.to_string()).unwrap();
        let race = |n: usize| -> Vec<Result<Session, String>> {
            let gate = std::sync::Arc::new(std::sync::Barrier::new(n));
            let racers: Vec<_> = (0..n)
                .map(|_| {
                    let (gate, root, user, id) =
                        (gate.clone(), root.clone(), user.clone(), id.clone());
                    std::thread::spawn(move || {
                        gate.wait();
                        Session::open(&root, Some(&id), &user)
                    })
                })
                .collect();
            racers.into_iter().map(|r| r.join().unwrap()).collect()
        };
        let files = || {
            ["workspace.json", "session.json", "lock"].map(|name| {
                let path = folder.join(name);
                let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
                (std::fs::read(&path).unwrap(), modified)
            })
        };
        // Until `starting` is dropped, by the end or a failed assert.
        let (starting, stop) = mpsc::channel::<()>();
        #[cfg(unix)]
        let starter = std::thread::spawn(move || {
            let mut started = 0u32;
            while stop.try_recv() == Err(mpsc::TryRecvError::Empty) {
                let _ = std::process::Command::new("true").status();
                started += 1;
            }
            started
        });
        #[cfg(not(unix))]
        let _ = stop;
        for round in 0..20 {
            let opened = race(8);
            let won: Vec<&Session> = opened.iter().filter_map(|r| r.as_ref().ok()).collect();
            assert_eq!(won.len(), 1, "round {round}: {opened:?}");
            for lost in opened.iter().filter_map(|r| r.as_ref().err()) {
                assert!(
                    lost.starts_with(&format!("Session {id} is open in another MCP server")),
                    "{lost}"
                );
            }
            // While it is held, every other open is refused and writes
            // nothing.
            let before = files();
            let again = race(6);
            assert!(again.iter().all(Result::is_err), "{again:?}");
            assert_eq!(files(), before, "a refused open wrote");
            drop(again);
            drop(opened);
            assert_let_go(&folder, &format!("round {round}: let go with the session"));
        }
        drop(starting);
        #[cfg(unix)]
        assert!(starter.join().unwrap() > 0, "no program was started");
        let resumed = Runner::start(Session::open(&root, Some(&id), &user).unwrap());
        assert_eq!(
            resumed.call("inspect", json!({})).unwrap()["project"]["title"],
            "Raced"
        );
        resumed.stop();
    }

    /// The reviewer's race of delete and resume (LIO-470): a resume while a
    /// session is being deleted, at each step of the deletion, is refused
    /// and writes nothing, and the deletion ends with the folder gone; a
    /// session resumed first is not deleted, its checkpoints all there.
    #[test]
    fn a_session_being_deleted_is_never_resumed() {
        use super::super::session::{DELETE_STEP, delete};
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("mcp-sessions");
        let user = user_workspace(dir.path());
        let worked = |title: &str| {
            let session = Session::open(&root, None, &user).unwrap();
            let id = session.id.clone();
            let runner = Runner::start(session);
            let edit = json!({"command": "set_project_title", "args": {"title": title}});
            assert_eq!(runner.call("execute", edit).unwrap()["outcome"], "done");
            for name in ["a", "b"] {
                let kept = runner.call("checkpoint", json!({"name": name})).unwrap();
                assert_eq!(kept["outcome"], "done", "{kept}");
            }
            runner.stop();
            id
        };
        let checkpoints =
            |folder: &Path| std::fs::read_dir(folder.join("checkpoints")).map_or(0, |d| d.count());

        // Resumed first: the deletion is refused, and nothing of it goes.
        let id = worked("Kept");
        let folder = root.join(&id);
        let listed = SessionFile::read(&folder)
            .unwrap()
            .unwrap()
            .checkpoints
            .len();
        assert_eq!(listed, 2);
        assert_eq!(checkpoints(&folder), listed);
        let resumed = Runner::start(Session::open(&root, Some(&id), &user).unwrap());
        let refused = delete(&root, &id, true).unwrap();
        assert_eq!(refused["outcome"], "refused", "{refused}");
        assert!(
            refused["reason"]
                .as_str()
                .unwrap()
                .contains("open in another MCP server"),
            "{refused}"
        );
        assert_eq!(checkpoints(&folder), listed);
        let back = resumed.call("list_checkpoints", json!({})).unwrap();
        assert_eq!(back["checkpoints"].as_array().unwrap().len(), 2, "{back}");
        resumed.stop();

        // Deleted first: a resume at each step is refused.
        let tried: std::rc::Rc<std::cell::RefCell<Vec<(String, String)>>> = Default::default();
        let log = tried.clone();
        let (r, u, i) = (root.clone(), user.clone(), id.clone());
        DELETE_STEP.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |step: &str| {
                let (r, u, i) = (r.clone(), u.clone(), i.clone());
                // Another open of the file, as another server's would be.
                let opened = std::thread::spawn(move || Session::open(&r, Some(&i), &u).map(drop))
                    .join()
                    .unwrap();
                let why = opened.expect_err("a session being deleted was opened");
                log.borrow_mut().push((step.to_string(), why));
            }));
        });
        let gone = delete(&root, &id, true);
        DELETE_STEP.with(|hook| hook.borrow_mut().take());
        assert_eq!(gone.unwrap()["outcome"], "done");
        assert!(!folder.exists(), "the folder is gone");
        let tried = tried.borrow();
        let steps: Vec<&str> = tried.iter().map(|(s, _)| s.as_str()).collect();
        assert_eq!(
            steps,
            [
                "session.json",
                "workspace.json",
                "entries",
                "lock",
                "folder"
            ]
        );
        for (step, why) in tried.iter() {
            let expected = match step.as_str() {
                // The lock still held.
                "session.json" | "workspace.json" | "entries" => "is open in another MCP server",
                // Its file removed: a resume makes a new one, and finds no job.
                "lock" => "is being deleted, or is gone",
                _ => "There is no MCP session",
            };
            assert!(why.contains(expected), "{step}: {why}");
        }
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
        assert_let_go(&folder, "the refusal let go of it");
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

    /// A tool call to `sessions`, answered.
    fn ask(
        sessions: &mut super::super::session::Sessions,
        tool: &str,
        args: Value,
    ) -> super::super::engine::Answer {
        let (tx, rx) = mpsc::channel();
        sessions.call(tool, args, Box::new(move |a| tx.send(a).unwrap()));
        rx.recv().unwrap()
    }
    fn listed(sessions: &mut super::super::session::Sessions, id: &str) -> Value {
        let all = ask(sessions, "list_sessions", json!({})).unwrap();
        (all["sessions"].as_array().unwrap().iter())
            .find(|s| s["id"] == id)
            .cloned()
            .unwrap()
    }

    /// The reviewer's probe (LIO-470): a session saved to a project file,
    /// ended, its project file removed; a new server still listed it as
    /// saved, and delete_session without discard_unsaved deleted the only
    /// copy. Saved is the file as it is now, holding the job by value.
    #[test]
    fn a_session_whose_project_file_went_is_not_saved_and_is_not_deleted() {
        use super::super::session::{Sessions, delete};
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("mcp-sessions");
        let user = user_workspace(dir.path());
        let session = Session::open(&root, None, &user).unwrap();
        let (id, folder) = (session.id.clone(), session.folder.clone());
        let runner = Runner::start(session);
        let project = dir.path().join("job.axis.json");
        let save =
            json!({"command": "save_project", "args": {"path": project.display().to_string()}});
        let saved = runner.call("execute", save).unwrap();
        assert_eq!(saved["outcome"], "done", "{saved}");
        runner.stop();
        let mut sessions = Sessions::new(root.clone(), user.clone(), None);
        assert_eq!(listed(&mut sessions, &id)["saved_to_project_file"], true);
        let bytes = std::fs::read(&project).unwrap();

        // Removed: not saved, and kept.
        std::fs::remove_file(&project).unwrap();
        let it = listed(&mut sessions, &id);
        assert_eq!(it["saved_to_project_file"], false, "{it}");
        let r = delete(&root, &id, false).unwrap();
        assert_eq!(r["outcome"], "refused", "{r}");
        assert!(r["reason"].as_str().unwrap().contains("not saved"), "{r}");
        assert!(folder.join("workspace.json").is_file());
        // Unreadable, or another job in it: the same.
        std::fs::write(&project, "not a project").unwrap();
        assert_eq!(listed(&mut sessions, &id)["saved_to_project_file"], false);
        assert_eq!(delete(&root, &id, false).unwrap()["outcome"], "refused");
        let mut other = crate::storage::load(&folder.join("workspace.json")).unwrap();
        other.title = "Changed by hand".into();
        crate::storage::save(&project, &other).unwrap();
        assert_eq!(listed(&mut sessions, &id)["saved_to_project_file"], false);
        assert_eq!(delete(&root, &id, false).unwrap()["outcome"], "refused");
        // Resumed, it is not saved either.
        let resumed = ask(&mut sessions, "resume_session", json!({"id": id})).unwrap();
        assert_eq!(
            resumed["session"]["saved_to_project_file"], false,
            "{resumed}"
        );
        assert_eq!(resumed["project"]["saved"], false, "{resumed}");
        sessions.stop();
        // The job back in its file: saved, and it goes without asking.
        std::fs::write(&project, &bytes).unwrap();
        let mut sessions = Sessions::new(root.clone(), user, None);
        assert_eq!(listed(&mut sessions, &id)["saved_to_project_file"], true);
        sessions.stop();
        assert_eq!(delete(&root, &id, false).unwrap()["outcome"], "done");
        assert!(!folder.exists());
    }

    /// The reviewer's probe (LIO-470): the session's folder made read-only,
    /// a title set was answered done though nothing reached the disk, and
    /// the resumed session had lost it. The result now says the change
    /// failed and is in memory only, every result after says the session
    /// is not saved, and once a write succeeds it is saved again.
    #[cfg(unix)]
    #[test]
    fn a_change_that_cannot_be_saved_is_not_reported_done() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("mcp-sessions");
        let user = user_workspace(dir.path());
        let session = Session::open(&root, None, &user).unwrap();
        let (id, folder) = (session.id.clone(), session.folder.clone());
        let runner = Runner::start(session);
        let start = runner.call("inspect", json!({})).unwrap();
        assert_eq!(start["persisted"], true, "{start}");
        let revision = start["revision"].as_u64().unwrap();
        let mode =
            |m: u32| std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(m)).unwrap();
        mode(0o555);
        let title = "ACKNOWLEDGED BUT NOT SAVED";
        let set = json!({"command": "set_project_title", "args": {"title": title}});
        let r = runner.call("execute", set).unwrap();
        let read = runner.call("inspect", json!({})).unwrap();
        let on_disk = SessionFile::read(&folder).unwrap().unwrap();
        mode(0o755);
        assert_eq!(r["outcome"], "failed", "{r}");
        assert_eq!(r["persisted"], false, "{r}");
        let why = r["reason"].as_str().unwrap();
        assert!(
            why.starts_with("The session could not be saved: its workspace.json was not written (")
                && why.ends_with("). Your change is in memory only."),
            "{r}"
        );
        assert_eq!(r["revision"], revision + 1, "the revision in memory: {r}");
        // Still not saved: a read says so too, and the change is there.
        assert_eq!(read["persisted"], false, "{read}");
        assert_eq!(read["project"]["title"], title, "{read}");
        let notes = read["notes"].as_array().unwrap();
        assert!(notes[0].as_str().unwrap().contains("not saved"), "{read}");
        assert_eq!(on_disk.log.revision, revision, "nothing new on disk");
        // Writable again: the next answer saves it.
        let again = runner.call("inspect", json!({})).unwrap();
        assert_eq!(again["persisted"], true, "{again}");
        assert!(again.get("notes").is_none(), "{again}");
        runner.stop();
        let session = Session::open(&root, Some(&id), &user).unwrap();
        let runner = Runner::start(session);
        let resumed = runner.call("inspect", json!({})).unwrap();
        assert_eq!(resumed["project"]["title"], title, "{resumed}");
        assert_eq!(resumed["revision"], revision + 1, "{resumed}");
        assert_eq!(resumed["user_changed"], json!([]), "{resumed}");
        runner.stop();
    }

    /// The reviewer's other probe: `session.json` alone cannot be written
    /// (a folder in its place) while the workspace can. The draft change
    /// is reported failed; the change the workspace kept is the agent's on
    /// resume, not taken for the user's.
    #[test]
    fn a_session_record_that_cannot_be_saved_is_reported_and_paired_on_resume() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("mcp-sessions");
        let user = user_workspace(dir.path());
        let session = Session::open(&root, None, &user).unwrap();
        let (id, folder) = (session.id.clone(), session.folder.clone());
        let runner = Runner::start(session);
        let start = runner.call("inspect", json!({})).unwrap();
        let revision = start["revision"].as_u64().unwrap();
        let record = folder.join(super::super::session::FILE);
        let aside = folder.join("session-before.json");
        std::fs::rename(&record, &aside).unwrap();
        std::fs::create_dir(&record).unwrap();
        let draft = json!({"command": "new_tool"});
        let r = runner.call("execute", draft).unwrap();
        assert_eq!(r["outcome"], "failed", "{r}");
        assert_eq!(r["persisted"], false, "{r}");
        assert!(
            r["reason"]
                .as_str()
                .unwrap()
                .contains("its session.json was not written"),
            "{r}"
        );
        let title = json!({"command": "set_project_title", "args": {"title": "In the workspace"}});
        let r = runner.call("execute", title).unwrap();
        assert_eq!(r["outcome"], "failed", "{r}");
        runner.stop();
        std::fs::remove_dir(&record).unwrap();
        std::fs::rename(&aside, &record).unwrap();
        // The workspace holds the title, the record does not: the change
        // is the agent's, one step of the revision.
        let session = Session::open(&root, Some(&id), &user).unwrap();
        let runner = Runner::start(session);
        let resumed = runner.call("inspect", json!({})).unwrap();
        assert_eq!(resumed["project"]["title"], "In the workspace", "{resumed}");
        assert_eq!(resumed["user_changed"], json!([]), "{resumed}");
        assert_eq!(resumed["revision"], revision + 1, "{resumed}");
        runner.stop();
        let kept = SessionFile::read(&folder).unwrap().unwrap();
        let last = kept.log.entries.last().unwrap();
        assert_eq!(last.source, crate::mcp::state::Source::Agent, "{last:?}");
        assert_eq!(last.events, [crate::mcp::state::UNRECORDED]);
    }

    /// Instance discovery (LIO-470): a call goes to a headless session only
    /// when no AXIS CAM window is open, until that session has changed its
    /// job. A window with access off, one with access on (driven in the
    /// next version), several, and a dead window's file.
    #[test]
    fn a_call_goes_headless_only_while_no_window_is_open() {
        use super::super::instance::{ACCESS_OFF, Access, Instance, folder};
        use super::super::{instance::started, session::Sessions};
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("mcp-sessions");
        let user = user_workspace(dir.path());
        let windows = folder(&user);
        let window = |state: Access, pid: u32| {
            let mut i = Instance::of_this_process(state);
            i.pid = pid;
            i.started = started(pid);
            i.project_title = "Open job".into();
            i.write(&windows).unwrap()
        };
        let me = std::process::id();
        #[cfg(unix)]
        let other = std::os::unix::process::parent_id();
        #[cfg(not(unix))]
        let other = me + 1;
        let sessions_made = || std::fs::read_dir(&root).map_or(0, |d| d.count());
        let mut sessions = Sessions::new(root.clone(), user.clone(), None);

        // One window, access off: the spec's error; no session made.
        let off = window(Access::Off, me);
        let r = ask(&mut sessions, "inspect", json!({})).unwrap_err();
        assert_eq!(r, format!("{ACCESS_OFF} (pid {me})"));
        assert_eq!(sessions_made(), 0);
        // Access on: driving a window comes next; never headless.
        std::fs::remove_file(&off).unwrap();
        let on = window(Access::On, me);
        let r = ask(&mut sessions, "inspect", json!({})).unwrap_err();
        assert!(
            r.contains(&format!("An AXIS CAM window is open (pid {me}")),
            "{r}"
        );
        assert!(
            r.contains("Driving the open window comes in the next version"),
            "{r}"
        );
        assert_eq!(sessions_made(), 0);
        let r = ask(&mut sessions, "resume_session", json!({"id": "any"})).unwrap_err();
        assert!(r.contains("next version"), "{r}");
        // A dead window's file is deleted, and the call goes headless.
        std::fs::remove_file(&on).unwrap();
        let dead = super::super::instance::tests::dead(&windows);
        let r = ask(&mut sessions, "inspect", json!({})).unwrap();
        assert_eq!(r["backend"], "headless", "{r}");
        assert!(!dead.exists());
        assert_eq!(sessions_made(), 1);
        // Two windows: listed, until one is chosen; the session has no
        // change yet, so it does not carry on headless.
        window(Access::On, me);
        window(Access::Off, other);
        let r = ask(&mut sessions, "inspect", json!({})).unwrap_err();
        assert!(r.starts_with("Several AXIS CAM windows are open"), "{r}");
        assert!(
            r.contains(&format!("pid {me}")) && r.contains(&format!("pid {other}")),
            "{r}"
        );
        assert!(r.contains("select_window"), "{r}");
        let r = ask(&mut sessions, "select_window", json!({"pid": 1})).unwrap_err();
        assert_eq!(r, "No AXIS CAM window with pid 1 is open.");
        let r = ask(&mut sessions, "select_window", json!({"pid": other})).unwrap_err();
        assert_eq!(r, format!("{ACCESS_OFF} (pid {other})"));
        let r = ask(&mut sessions, "inspect", json!({})).unwrap_err();
        assert_eq!(
            r,
            format!("{ACCESS_OFF} (pid {other})"),
            "the window chosen"
        );
        let r = ask(&mut sessions, "select_window", json!({"pid": me})).unwrap_err();
        assert!(r.contains("next version"), "{r}");
        // No window: the session changes its job, and then stays headless
        // though a window opens.
        for e in std::fs::read_dir(&windows).unwrap().flatten() {
            std::fs::remove_file(e.path()).unwrap();
        }
        let set = json!({"command": "set_project_title", "args": {"title": "Headless"}});
        assert_eq!(
            ask(&mut sessions, "execute", set).unwrap()["outcome"],
            "done"
        );
        window(Access::On, me);
        let r = ask(&mut sessions, "inspect", json!({})).unwrap();
        assert_eq!(r["project"]["title"], "Headless", "{r}");
        sessions.stop();
    }

    /// A command run in a session, waited for.
    fn exec(runner: &Runner, command: &str, args: Value) -> Value {
        let call = json!({"command": command, "args": args});
        runner.call("execute", call).unwrap()
    }

    /// The reviewer's probe (LIO-470, round 2): project A saved; then, with
    /// `session.json` alone unwritable, project B opened (B written to the
    /// workspace, the record still naming A's file); the server ended and
    /// the old record put back. Resumed, the session held B with A's path,
    /// and `save_project {}` wrote B over A. The record older than its job
    /// now restores no project file, says so, and A is never written.
    #[test]
    fn a_record_older_than_its_job_never_saves_over_another_project() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("mcp-sessions");
        let user = user_workspace(dir.path());
        let session = Session::open(&root, None, &user).unwrap();
        let (id, folder) = (session.id.clone(), session.folder.clone());
        let runner = Runner::start(session);
        let r = exec(&runner, "set_project_title", json!({"title": "Original A"}));
        assert_eq!(r["outcome"], "done", "{r}");
        let a = dir.path().join("A.axis.json");
        let b = dir.path().join("B.axis.json");
        let r = exec(
            &runner,
            "save_project",
            json!({"path": a.display().to_string()}),
        );
        assert_eq!(r["outcome"], "done", "{r}");
        let old_a = std::fs::read(&a).unwrap();
        let mut other: Value = serde_json::from_slice(&old_a).unwrap();
        other["title"] = json!("Different B");
        other["project_id"] = json!(uuid::Uuid::new_v4().to_string());
        std::fs::write(&b, serde_json::to_vec(&other).unwrap()).unwrap();
        // session.json alone cannot be replaced; the workspace can.
        let record = folder.join(super::super::session::FILE);
        let aside = folder.join("old-record.json");
        std::fs::rename(&record, &aside).unwrap();
        std::fs::create_dir(&record).unwrap();
        let r = exec(
            &runner,
            "open_project",
            json!({"path": b.display().to_string()}),
        );
        assert_eq!(r["persisted"], false, "{r}");
        let read = runner.call("inspect", json!({})).unwrap();
        assert_eq!(read["project"]["title"], "Different B", "{read}");
        // The server ends (its last write of the record fails too), and
        // the record of A is back beside B's workspace.
        runner.stop();
        std::fs::remove_dir(&record).unwrap();
        std::fs::rename(&aside, &record).unwrap();
        let kept = SessionFile::read(&folder).unwrap().unwrap();
        assert_eq!(kept.project_path.as_deref(), Some(a.as_path()));

        let runner = Runner::start(Session::open(&root, Some(&id), &user).unwrap());
        let read = runner.call("inspect", json!({})).unwrap();
        assert_eq!(read["project"]["title"], "Different B", "{read}");
        assert_eq!(read["project"]["file"], Value::Null, "{read}");
        assert_eq!(read["project"]["saved"], false, "{read}");
        assert_eq!(read["project"]["dirty"], true, "{read}");
        assert_eq!(read["session"]["project_file"], Value::Null, "{read}");
        assert_eq!(read["session"]["saved_to_project_file"], false, "{read}");
        assert_eq!(
            read["notes"],
            json!([super::super::session::OLDER_RECORD]),
            "{read}"
        );
        // Said once.
        let again = runner.call("inspect", json!({})).unwrap();
        assert!(again.get("notes").is_none(), "{again}");
        let r = exec(&runner, "save_project", json!({}));
        assert_eq!(r["outcome"], "refused", "{r}");
        assert_eq!(std::fs::read(&a).unwrap(), old_a, "A is as it was");
        // Saved with a path, it is saved there.
        let c = dir.path().join("C.axis.json");
        let r = exec(
            &runner,
            "save_project",
            json!({"path": c.display().to_string()}),
        );
        assert_eq!(r["outcome"], "done", "{r}");
        assert_eq!(r["session"]["saved_to_project_file"], true, "{r}");
        runner.stop();
        assert_eq!(std::fs::read(&a).unwrap(), old_a, "A is as it was");
        assert_eq!(crate::storage::load(&c).unwrap().title, "Different B");
    }

    /// The reviewer's probe (LIO-470, round 2): the instance folder made
    /// unreadable with a live window's file in it; the call went headless
    /// and made a session. Whether a window is open cannot be told then:
    /// the call is an error, and no session is made.
    #[cfg(unix)]
    #[test]
    fn an_unreadable_instance_folder_is_an_error_not_headless() {
        use super::super::instance::{ACCESS_OFF, Access, Found, Instance, discover, folder};
        use super::super::session::Sessions;
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("mcp-sessions");
        let user = user_workspace(dir.path());
        let windows = folder(&user);
        Instance::of_this_process(Access::Off)
            .write(&windows)
            .unwrap();
        let mode = |m: u32| {
            std::fs::set_permissions(&windows, std::fs::Permissions::from_mode(m)).unwrap()
        };
        let mut sessions = Sessions::new(root.clone(), user.clone(), None);
        mode(0o000);
        let found = discover(&windows);
        let read = ask(&mut sessions, "inspect", json!({}));
        let resume = ask(&mut sessions, "resume_session", json!({"id": "any"}));
        let made = root.exists();
        mode(0o755);
        assert!(matches!(found, Found::Unknown(_)), "{found:?}");
        let why = read.unwrap_err();
        assert!(
            why.starts_with("Whether an AXIS CAM window is open cannot be told")
                && why.contains(&windows.display().to_string()),
            "{why}"
        );
        assert!(resume.unwrap_err().contains("cannot be told"));
        assert!(!made, "no session made");
        // Readable again: the window is found.
        let r = ask(&mut sessions, "inspect", json!({})).unwrap_err();
        assert_eq!(r, format!("{ACCESS_OFF} (pid {})", std::process::id()));
        assert!(!root.exists());
        sessions.stop();
    }

    /// The reviewer's probe (LIO-470, round 2): a session's first change
    /// sent with `wait_s: 0` (still running at revision 0), then a window
    /// appears; `wait` and `inspect` were refused for it even after the
    /// change had settled. A session that has dispatched a change is
    /// committed to staying headless from the dispatch. Before that, a
    /// window refuses the session's calls but its stops, and a `wait` for
    /// a command it gave out.
    #[test]
    fn a_change_dispatched_before_a_window_opens_stays_headless() {
        use super::super::instance::{ACCESS_OFF, Access, Instance, folder};
        use super::super::session::Sessions;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("mcp-sessions");
        let user = user_workspace(dir.path());
        let windows = folder(&user);
        let off = format!("{ACCESS_OFF} (pid {})", std::process::id());
        let mut sessions = Sessions::new(root.clone(), user.clone(), None);
        let start = ask(&mut sessions, "inspect", json!({})).unwrap();
        assert_eq!(start["revision"], 0, "{start}");

        // No change dispatched yet: the window refuses a call, but not a
        // stop, nor the wait for a read the session gave out.
        let read = ask(&mut sessions, "check_job", json!({"wait_s": 50})).unwrap();
        let read_id = read["command_id"].as_str().unwrap().to_string();
        let window = Instance::of_this_process(Access::Off)
            .write(&windows)
            .unwrap();
        assert_eq!(ask(&mut sessions, "inspect", json!({})).unwrap_err(), off);
        let stop = json!({"command": "hold"});
        let r = ask(&mut sessions, "execute", stop).unwrap();
        assert_ne!(r["outcome"], "busy", "{r}");
        let r = ask(&mut sessions, "wait", json!({"command_id": read_id})).unwrap();
        assert_eq!(r["command_id"], read_id.as_str(), "{r}");
        assert_ne!(r["outcome"], "still_running", "{r}");
        let r = ask(&mut sessions, "wait", json!({"command_id": "nothing-9"}));
        assert_eq!(r.unwrap_err(), off, "not a command it gave out");
        std::fs::remove_file(&window).unwrap();

        // A change sent, still running at revision 0.
        let set = json!({"command": "set_project_title", "args": {"title": "Before the window"}, "wait_s": 0});
        let r = ask(&mut sessions, "execute", set).unwrap();
        assert_eq!(r["outcome"], "still_running", "{r}");
        assert_eq!(r["revision"], 0, "{r}");
        let id = r["command_id"].as_str().unwrap().to_string();
        // A window opens; the change settles with no one waiting.
        Instance::of_this_process(Access::Off)
            .write(&windows)
            .unwrap();
        std::thread::sleep(Duration::from_secs(1));
        let r = ask(
            &mut sessions,
            "wait",
            json!({"command_id": id, "wait_s": 10}),
        )
        .unwrap();
        assert_eq!(r["outcome"], "done", "{r}");
        assert_eq!(r["revision"], 1, "{r}");
        assert_eq!(r["backend"], "headless", "{r}");
        let read = ask(&mut sessions, "inspect", json!({})).unwrap();
        assert_eq!(read["project"]["title"], "Before the window", "{read}");
        assert_eq!(read["backend"], "headless", "{read}");
        sessions.stop();
    }

    /// The reviewer's probe (LIO-470, round 2): the project file changed by
    /// another program while the server ran; `inspect` still said saved,
    /// and `list_sessions` not. Every saved flag is the file as it is now,
    /// by value: changed, removed, unreadable and put back, no restart.
    #[test]
    fn inspect_says_saved_by_the_project_file_as_it_is_now() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("mcp-sessions");
        let user = user_workspace(dir.path());
        let session = Session::open(&root, None, &user).unwrap();
        let id = session.id.clone();
        let runner = Runner::start(session);
        let project = dir.path().join("job.axis.json");
        let r = exec(
            &runner,
            "save_project",
            json!({"path": project.display().to_string()}),
        );
        assert_eq!(r["outcome"], "done", "{r}");
        let original = std::fs::read(&project).unwrap();
        let saved = |runner: &Runner, want: bool| {
            let read = runner.call("inspect", json!({})).unwrap();
            assert_eq!(read["project"]["saved"], want, "{read}");
            assert_eq!(read["session"]["saved_to_project_file"], want, "{read}");
            let listed = super::super::session::list(&root);
            let it = listed.iter().find(|s| s["id"] == id.as_str()).unwrap();
            assert_eq!(it["saved_to_project_file"], want, "{it}");
        };
        saved(&runner, true);
        let mut changed = crate::storage::load(&project).unwrap();
        changed.title = "Changed elsewhere".into();
        crate::storage::save(&project, &changed).unwrap();
        saved(&runner, false);
        std::fs::write(&project, &original).unwrap();
        saved(&runner, true);
        std::fs::write(&project, "not a project").unwrap();
        saved(&runner, false);
        std::fs::remove_file(&project).unwrap();
        saved(&runner, false);
        std::fs::write(&project, &original).unwrap();
        saved(&runner, true);
        // The job changed here: not saved, until saved again.
        let r = exec(
            &runner,
            "set_project_title",
            json!({"title": "Changed here"}),
        );
        assert_eq!(r["session"]["saved_to_project_file"], false, "{r}");
        saved(&runner, false);
        let r = exec(&runner, "save_project", json!({}));
        assert_eq!(r["outcome"], "done", "{r}");
        saved(&runner, true);
        runner.stop();
    }

    /// The reviewer's probes (LIO-470, round 3): a chmod of the project
    /// file, and a same-length replacement with its mtime put back, left
    /// `inspect` saying saved while `list_sessions` did not.
    #[cfg(unix)]
    #[test]
    fn saved_flags_agree_after_a_chmod_and_a_same_mtime_replacement() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("mcp-sessions");
        let user = user_workspace(dir.path());
        let session = Session::open(&root, None, &user).unwrap();
        let id = session.id.clone();
        let runner = Runner::start(session);
        let project = dir.path().join("job.axis.json");
        let r = exec(
            &runner,
            "save_project",
            json!({"path": project.display().to_string()}),
        );
        assert_eq!(r["outcome"], "done", "{r}");
        let original = std::fs::read(&project).unwrap();
        let saved = |runner: &Runner, want: bool| {
            let read = runner.call("inspect", json!({})).unwrap();
            assert_eq!(read["project"]["saved"], want, "{read}");
            assert_eq!(read["session"]["saved_to_project_file"], want, "{read}");
            let listed = super::super::session::list(&root);
            let it = listed.iter().find(|s| s["id"] == id.as_str()).unwrap();
            assert_eq!(it["saved_to_project_file"], want, "{it}");
        };
        saved(&runner, true);
        // Unreadable: never saved.
        std::fs::set_permissions(&project, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::read(&project).is_err() {
            saved(&runner, false);
        }
        std::fs::set_permissions(&project, std::fs::Permissions::from_mode(0o644)).unwrap();
        saved(&runner, true);
        // Same length, same mtime, other content, by an atomic replace.
        let mtime = std::fs::metadata(&project).unwrap().modified().unwrap();
        let mut other = String::from_utf8(original.clone()).unwrap();
        let at = other.find("\"title\"").expect("a title");
        let digit = other[at..]
            .find(|c: char| c.is_alphanumeric() && c != 't')
            .unwrap()
            + at;
        let c = other.as_bytes()[digit];
        let swapped = if c == b'x' { 'y' } else { 'x' };
        other.replace_range(digit..=digit, &swapped.to_string());
        assert_eq!(other.len(), original.len());
        let temp = dir.path().join("replacement.tmp");
        std::fs::write(&temp, &other).unwrap();
        std::fs::File::options()
            .write(true)
            .open(&temp)
            .unwrap()
            .set_modified(mtime)
            .unwrap();
        std::fs::rename(&temp, &project).unwrap();
        saved(&runner, false);
        // Restored the same way.
        std::fs::write(&temp, &original).unwrap();
        std::fs::File::options()
            .write(true)
            .open(&temp)
            .unwrap()
            .set_modified(mtime)
            .unwrap();
        std::fs::rename(&temp, &project).unwrap();
        saved(&runner, true);
        runner.stop();
    }
}
