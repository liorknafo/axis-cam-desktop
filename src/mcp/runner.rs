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
use super::engine::{self, Backend};
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

/// What a tool call gives back: its result object, or why the call could
/// not be carried out at all (an MCP tool error).
pub type Answer = Result<Value, String>;
/// Where a tool call's answer goes, called once on the session's thread.
pub type Reply = Box<dyn FnOnce(Answer) + Send>;

/// The `Tick` rates of the window's subscription (`App::subscription`).
const FAST_TICK: Duration = Duration::from_millis(33);
const SLOW_TICK: Duration = Duration::from_millis(500);

/// A headless session: its id and the folder it is kept in,
/// `AXIS CAM Studio/mcp-sessions/<id>/`, which holds its own workspace.
#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub folder: PathBuf,
}
impl Session {
    /// The folder the sessions are kept in, beside the user's workspace.
    pub fn root() -> PathBuf {
        crate::storage::state_path().with_file_name("mcp-sessions")
    }
    /// Session `id` in `root`, whose folder must be there, or with no id a
    /// new session, which starts from a copy of the user's `workspace`
    /// (when there is one) so it has their libraries. The user's workspace
    /// is only read: the session's `App` saves its own copy.
    pub fn open(root: &Path, id: Option<&str>, workspace: &Path) -> Result<Session, String> {
        if let Some(id) = id {
            let named = !id.is_empty()
                && id.len() <= 64
                && id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
            if !named {
                return Err(format!("\"{id}\" is not a session id."));
            }
            let folder = root.join(id);
            if !folder.is_dir() {
                return Err(format!(
                    "There is no MCP session {id} in {}.",
                    root.display()
                ));
            }
            return Ok(Session {
                id: id.into(),
                folder,
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
        let session = Session { id, folder };
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
    /// A message a task produced, boxed: a `Message` is large.
    Output(Box<Message>),
    /// The client has gone: the session is saved and the thread ends.
    Stop,
}

/// The handle to a running headless session.
pub struct Runner {
    events: mpsc::Sender<Event>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Runner {
    /// Starts `session` on a thread of its own. The saved part is loaded
    /// as Continue project loads it, and the first tool call waits for it.
    pub fn start(session: Session) -> Runner {
        let (events, inbox) = mpsc::channel();
        let own = events.clone();
        let thread = std::thread::Builder::new()
            .name("mcp-session".into())
            .spawn(move || Host::new(session, inbox, own).serve())
            .expect("the MCP session's thread starts");
        Runner {
            events,
            thread: Some(thread),
        }
    }
    /// Sends a tool call; `reply` gets its answer on the session's thread.
    /// Calls are answered one at a time, in the order they came.
    pub fn send(&self, tool: String, args: Value, reply: Reply) {
        let request = Request { tool, args, reply };
        if let Err(mpsc::SendError(Event::Request(r))) = self.events.send(Event::Request(request)) {
            (r.reply)(Err("The headless session has stopped.".into()));
        }
    }
    /// A tool call's answer, waited for.
    #[cfg(test)]
    pub fn call(&self, tool: &str, args: Value) -> Answer {
        let (tx, rx) = mpsc::channel();
        self.send(
            tool.into(),
            args,
            Box::new(move |a| {
                let _ = tx.send(a);
            }),
        );
        rx.recv().unwrap_or_else(|_| Err("no answer".into()))
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
    /// Where tasks send the messages they produce.
    events: mpsc::Sender<Event>,
    runtime: tokio::runtime::Runtime,
    /// The tasks whose streams have not ended.
    tasks: Arc<AtomicUsize>,
    last_tick: Instant,
    /// A `Tick` has run since the last other message: the work a `Tick`
    /// starts after a change has been started.
    ticked: bool,
    /// The first tool call has waited for the session's start to settle.
    started: bool,
    /// Tool calls that came while another was waiting for its work.
    backlog: VecDeque<Request>,
    stopping: bool,
}
impl Host {
    fn new(session: Session, inbox: mpsc::Receiver<Event>, events: mpsc::Sender<Event>) -> Host {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .thread_name("mcp-tasks")
            .enable_all()
            .build()
            .expect("the MCP session's runtime starts");
        let mut app = App::from_workspace(session.workspace(), None);
        // Generation, layout, left stock and meshing run in the background
        // as in the window, so a call is answered while they run.
        app.generate_inline = false;
        let mut host = Host {
            app,
            session,
            inbox,
            events,
            runtime,
            tasks: Arc::new(AtomicUsize::new(0)),
            last_tick: Instant::now(),
            ticked: false,
            started: false,
            backlog: VecDeque::new(),
            stopping: false,
        };
        host.update(Message::ResumeProject);
        host
    }
    fn serve(mut self) {
        while !self.stopping || !self.backlog.is_empty() {
            if let Some(request) = self.backlog.pop_front() {
                self.answer(request);
                continue;
            }
            let wait = self
                .tick_due(false)
                .saturating_duration_since(Instant::now());
            match self.inbox.recv_timeout(wait) {
                Ok(event) => self.take(event),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
            self.tick_if_due(false);
        }
        // What the last edits did is kept for the next start.
        if self.app.dirty {
            self.app.persist();
        }
        self.runtime.shutdown_background();
    }
    fn take(&mut self, event: Event) {
        match event {
            Event::Request(r) => self.backlog.push_back(r),
            Event::Output(message) => self.update(*message),
            Event::Stop => self.stopping = true,
        }
    }
    /// Answers a tool call within its `wait_s`. The first waits for the
    /// session's start to settle, its saved part restored.
    fn answer(&mut self, request: Request) {
        let deadline = Instant::now() + engine::wait_of(&request.args);
        if !self.started {
            self.started = self.settle(deadline);
        }
        let (tool, args) = (request.tool, request.args);
        let answer = crate::fault::catch(|| engine::call(self, &tool, &args, deadline))
            .unwrap_or_else(|fault| Err(format!("The app failed while doing this: {fault}")));
        (request.reply)(answer);
    }
    fn update(&mut self, message: Message) {
        let tick = matches!(message, Message::Tick);
        let task = self.app.update(message);
        self.pump(task);
        self.ticked = tick;
    }
    /// Runs `task` on the runtime, its messages sent back to this thread.
    /// A task with no work in it, as `Tick` mostly returns (a batch of
    /// `Task::none`), is not run: it would count as running a moment
    /// after every `Tick`.
    fn pump(&self, task: Task<Message>) {
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
                    let _ = events.send(Event::Output(Box::new(message)));
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
            self.update(Message::Tick);
        }
    }
}
impl Backend for Host {
    fn app(&self) -> &App {
        &self.app
    }
    fn send(&mut self, message: Message) {
        self.update(message)
    }
    /// Takes events until the work is done or `deadline` passes: no task
    /// running, no message waiting, a `Tick` since the last message, and
    /// `App::pending` empty. Tool calls that come meanwhile wait their turn.
    fn settle(&mut self, deadline: Instant) -> bool {
        loop {
            self.tick_if_due(true);
            // Read before the inbox: a task sends its last message before
            // it ends, so with none running and none waiting, none comes.
            let tasks = self.tasks.load(SeqCst);
            match self.inbox.try_recv() {
                Ok(event) => {
                    self.take(event);
                    continue;
                }
                Err(mpsc::TryRecvError::Disconnected) => return false,
                Err(mpsc::TryRecvError::Empty) => {}
            }
            if tasks == 0 && self.ticked && self.app.pending().is_empty() {
                return true;
            }
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            let until = self.tick_due(true).min(deadline);
            match self
                .inbox
                .recv_timeout(until.saturating_duration_since(now))
            {
                Ok(event) => self.take(event),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return false,
            }
        }
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Document;

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
        // The session can be opened again by its id, without a new copy.
        let again = Session::open(&root, Some(&session.id), &user).unwrap();
        assert_eq!(again.folder, session.folder);
        assert!(Session::open(&root, Some("../x"), &user).is_err());
        assert!(Session::open(&root, Some("missing"), &user).is_err());
    }
}
