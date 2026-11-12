//! Separate process isolates CAD conversion from the GUI and enforces a timeout.
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{Mutex, MutexGuard},
    time::{Duration, Instant},
};

/// The converter to run: the one shipped beside the executable, or the one
/// this build compiled.
fn helper() -> Result<PathBuf, String> {
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let sibling = executable.parent().unwrap().join("axis-step");
    // A shipped .app must never fall back to the developer's build directory.
    let bundled = executable.components().any(|p| p.as_os_str() == "Contents");
    if sibling.exists() || bundled {
        Ok(sibling)
    } else {
        option_env!("AXIS_BUILT_STEP_HELPER")
            .map(PathBuf::from)
            .ok_or("STEP support was disabled in this build. Rebuild with Open CASCADE.".into())
    }
}

/// Whether `load` has a converter to run; false in a build made with
/// `AXIS_SKIP_STEP=1`.
pub fn available() -> bool {
    helper().is_ok()
}

/// For a test that needs the STEP reader: says so and returns true when this
/// build has none, and the test then skips its STEP cases (`return`, or
/// `continue` in a loop over formats). Written straight to standard error,
/// which the test harness does not capture, so a normal `cargo test` shows it.
#[cfg(test)]
pub fn skip_without_helper() -> bool {
    use std::io::Write;
    let skip = !available();
    if skip {
        let _ = writeln!(
            std::io::stderr(),
            "skipped: built without the STEP helper (AXIS_SKIP_STEP)"
        );
    }
    skip
}

const TIMEOUT: Duration = Duration::from_secs(120);
/// How long reading the first bytes of the file may take before the import
/// stops and says why. A local file answers in milliseconds; a read that
/// takes longer is most likely waiting on macOS asking whether the app may
/// open files in that folder (LIO-444), which the converter would wait on
/// for the whole `TIMEOUT`.
const READ_TIMEOUT: Duration = Duration::from_secs(10);

/// The converters being waited for, which `kill_running` ends. A converter is
/// registered here from its start until it has exited, so that a process that
/// ends mid-import does not leave it running at full CPU with no parent.
type Running = Mutex<Vec<Child>>;
static RUNNING: Running = Mutex::new(Vec::new());

fn locked(running: &Running) -> MutexGuard<'_, Vec<Child>> {
    running.lock().unwrap_or_else(|e| e.into_inner())
}

/// Kills and reaps every converter that is running; the waiting `load`s
/// report that the import was cancelled. For the close path, before it ends
/// the process.
pub fn kill_running() {
    kill_all(&RUNNING);
}

fn kill_all(running: &Running) {
    for mut child in locked(running).drain(..) {
        let _ = child.kill();
        let _ = child.wait();
    }
}

enum Poll {
    Exited(ExitStatus),
    Running,
    /// Killed by `kill_all`.
    Gone,
    Failed(String),
}

fn poll(running: &Running, pid: u32) -> Poll {
    let mut running = locked(running);
    let Some(i) = running.iter().position(|c| c.id() == pid) else {
        return Poll::Gone;
    };
    match running[i].try_wait() {
        Ok(Some(status)) => {
            // Already reaped by `try_wait`; this returns the same status.
            let _ = running.swap_remove(i).wait();
            Poll::Exited(status)
        }
        Ok(None) => Poll::Running,
        Err(e) => {
            let mut child = running.swap_remove(i);
            let _ = child.kill();
            let _ = child.wait();
            Poll::Failed(e.to_string())
        }
    }
}

/// The part's triangles, and the CAD face of each where the reader wrote
/// them (`mesh::stl_faces`).
pub type Faces = (Vec<[[f32; 3]; 3]>, Option<Vec<u32>>);
pub fn load(path: &Path) -> Result<Faces, String> {
    let bytes = convert(&RUNNING, &helper()?, path, READ_TIMEOUT, TIMEOUT)?;
    Ok((super::mesh::stl(&bytes)?, super::mesh::stl_faces(&bytes)))
}

/// Whether this process can read `path`, found by reading its first bytes on
/// a thread of its own and waiting at most `timeout` for them. A read macOS
/// holds until someone answers its folder-permission prompt is left to finish
/// on that thread: nothing is remembered, so the next import tries afresh.
fn readable(path: &Path, timeout: Duration) -> Result<(), String> {
    let name = path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    let (send, receive) = std::sync::mpsc::channel();
    let owned = path.to_path_buf();
    std::thread::Builder::new()
        .name("step-read-check".into())
        .spawn(move || {
            let read = fs::File::open(&owned).and_then(|mut f| f.read(&mut [0; 64]));
            let _ = send.send(read);
        })
        .map_err(|e| format!("{name} could not be read: {e}."))?;
    match receive.recv_timeout(timeout) {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(e)) => Err(unreadable(&name, &e)),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Err(waiting(&name, timeout)),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            Err(format!("{name} could not be read."))
        }
    }
}

/// The settings pane that gives an app a protected folder (Documents,
/// Desktop, Downloads, iCloud Drive, a removable or network volume).
#[cfg(target_os = "macos")]
const FILES_AND_FOLDERS: &str = "System Settings › Privacy & Security › Files and Folders";

/// Why a read that has not answered within `timeout` waits.
fn waiting(name: &str, timeout: Duration) -> String {
    let secs = timeout.as_secs_f32();
    #[cfg(target_os = "macos")]
    return format!(
        "{name} could not be read within {secs} s. macOS is probably asking whether AXIS CAM Studio may open files in its folder: click Allow in that prompt, or turn the folder on for AXIS CAM Studio in {FILES_AND_FOLDERS}. Then reimport the model."
    );
    #[cfg(not(target_os = "macos"))]
    format!(
        "{name} could not be read within {secs} s: the drive or network folder that holds it is not answering. Once it does, reimport the model."
    )
}

/// Why a read failed; on macOS a refused folder permission (EPERM) names
/// where to give it.
fn unreadable(name: &str, e: &std::io::Error) -> String {
    #[cfg(target_os = "macos")]
    if e.raw_os_error() == Some(libc::EPERM) {
        return format!(
            "macOS did not let AXIS CAM Studio read {name}. Turn its folder on for AXIS CAM Studio in {FILES_AND_FOLDERS}, then reimport the model."
        );
    }
    format!("{name} could not be read: {e}.")
}

// Split from `load` so a test can run a stand-in converter under short
// timeouts, registered in a slot of its own.
fn convert(
    running: &Running,
    helper: &Path,
    path: &Path,
    read_timeout: Duration,
    timeout: Duration,
) -> Result<Vec<u8>, String> {
    // Before the converter starts: it would wait on the same read for the
    // whole `timeout` and be reported as a slow conversion.
    readable(path, read_timeout)?;
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let bundled = executable.components().any(|p| p.as_os_str() == "Contents");
    let temp = tempfile::tempdir().map_err(|e| e.to_string())?;
    let result = temp.path().join("preview.stl");
    let error = temp.path().join("error.txt");
    let mut command = Command::new(helper);
    if bundled {
        let resources = executable.parent().unwrap().join("../Resources/OCCT");
        command.env("CSF_STEPDefaults", resources.join("XSTEPResource"));
        command.env("CSF_XSMessage", resources.join("XSMessage"));
        command.env("CSF_UnitsDefinition", resources.join("UnitsAPI/Units.dat"));
    }
    let child = command
        .arg(path)
        .arg(&result)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(fs::File::create(&error).map_err(|e| e.to_string())?)
        .spawn()
        .map_err(|e| format!("Could not start bundled STEP reader: {e}"))?;
    let pid = child.id();
    locked(running).push(child);
    let start = Instant::now();
    let status = loop {
        match poll(running, pid) {
            Poll::Exited(status) => break status,
            Poll::Running => {}
            Poll::Gone => return Err("STEP import cancelled: the application is closing.".into()),
            Poll::Failed(e) => return Err(e),
        }
        if start.elapsed() > timeout {
            let mut running = locked(running);
            if let Some(i) = running.iter().position(|c| c.id() == pid) {
                let mut child = running.swap_remove(i);
                let _ = child.kill();
                let _ = child.wait();
            }
            return Err("STEP conversion exceeded two minutes; try a smaller model.".into());
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    if !status.success() {
        let error = fs::read_to_string(error).unwrap_or_default();
        return Err(format!(
            "STEP import failed: {}",
            if error.trim().is_empty() {
                format!("converter exited with {status}")
            } else {
                error.chars().take(500).collect()
            }
        ));
    }
    fs::read(result).map_err(|e| format!("STEP preview missing: {e}"))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    type Tri = [[f32; 3]; 3];

    // The shell is the stand-in converter: it is run as `sh <path> <result>`, so the
    // "STEP file" is the script. No executable has to be written, which can fail with
    // "text file busy" while other tests fork.
    fn run(script: &str, timeout: Duration) -> (Result<Vec<Tri>, String>, Duration) {
        run_in(&Mutex::new(vec![]), script, timeout)
    }
    fn run_in(
        running: &Running,
        script: &str,
        timeout: Duration,
    ) -> (Result<Vec<Tri>, String>, Duration) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("converter.sh");
        fs::write(&path, script).unwrap();
        let start = Instant::now();
        let result = convert(running, Path::new("/bin/sh"), &path, READ_TIMEOUT, timeout)
            .and_then(|bytes| super::super::mesh::stl(&bytes));
        (result, start.elapsed())
    }

    #[test]
    fn killing_the_running_converters_ends_the_child_and_cancels_the_import() {
        let running = Running::default();
        let (result, took, pid) = std::thread::scope(|scope| {
            let import = scope.spawn(|| run_in(&running, "exec sleep 60", Duration::from_secs(60)));
            let start = Instant::now();
            let pid = loop {
                if let Some(child) = locked(&running).first() {
                    break child.id();
                }
                assert!(start.elapsed() < Duration::from_secs(20), "never started");
                std::thread::sleep(Duration::from_millis(10));
            };
            kill_all(&running);
            let (result, took) = import.join().unwrap();
            (result, took, pid)
        });
        assert_eq!(
            result.unwrap_err(),
            "STEP import cancelled: the application is closing."
        );
        assert!(took < Duration::from_secs(20));
        assert!(locked(&running).is_empty());
        // `kill_all` reaped the child, so the process is gone, not a zombie.
        let alive = Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success();
        assert!(!alive, "converter {pid} still exists");
    }

    #[test]
    fn a_converter_that_outlives_the_timeout_is_killed() {
        let (result, took) = run("exec sleep 60", Duration::from_millis(300));
        assert_eq!(
            result.unwrap_err(),
            "STEP conversion exceeded two minutes; try a smaller model."
        );
        // Returned when the timeout passed, not when the 60 s sleep ended.
        assert!(took >= Duration::from_millis(300) && took < Duration::from_secs(20));
    }

    /// A file whose read macOS holds while its folder-permission prompt is
    /// open, stood in for by a FIFO, whose open waits for a writer the same
    /// way: the import stops after the short read timeout, without starting
    /// the converter, and says why. Once the read answers (permission
    /// given), the next import of the same path converts it.
    #[test]
    fn a_file_that_cannot_be_read_yet_is_reported_soon_and_converts_once_it_can() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("adapter-plate.step");
        assert!(
            Command::new("mkfifo")
                .arg(&path)
                .status()
                .unwrap()
                .success()
        );
        let running = Running::default();
        let start = Instant::now();
        // A converter that does not exist: had it been started, the error
        // would say it could not be.
        let result = convert(
            &running,
            &dir.path().join("no-converter"),
            &path,
            Duration::from_millis(300),
            Duration::from_secs(60),
        );
        let took = start.elapsed();
        let message = result.unwrap_err();
        assert_eq!(
            message,
            waiting("adapter-plate.step", Duration::from_millis(300))
        );
        assert!(message.starts_with("adapter-plate.step could not be read within 0.3 s"));
        assert!(message.ends_with("reimport the model."), "{message}");
        if cfg!(target_os = "macos") {
            assert!(
                message.contains(
                    "click Allow in that prompt, or turn the folder on for AXIS CAM Studio in System Settings › Privacy & Security › Files and Folders"
                ),
                "{message}"
            );
        }
        assert!(took >= Duration::from_millis(300) && took < Duration::from_secs(20));
        assert!(locked(&running).is_empty());

        // Permission given: the held read answers (the check's thread ends)
        // and the file can be read.
        drop(fs::OpenOptions::new().write(true).open(&path).unwrap());
        fs::remove_file(&path).unwrap();
        let triangle = dir.path().join("triangle.stl");
        let mut stl = vec![0u8; 80];
        stl.extend(1u32.to_le_bytes());
        stl.extend([0u8; 12]);
        for x in [0f32, 0., 0., 1., 0., 0., 0., 1., 0.] {
            stl.extend(x.to_le_bytes());
        }
        stl.extend([0u8; 2]);
        fs::write(&triangle, stl).unwrap();
        fs::write(&path, format!("cat '{}' > \"$1\"", triangle.display())).unwrap();
        let bytes = convert(
            &running,
            Path::new("/bin/sh"),
            &path,
            Duration::from_millis(300),
            Duration::from_secs(60),
        )
        .unwrap();
        assert_eq!(
            super::super::mesh::stl(&bytes).unwrap(),
            vec![[[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]]]
        );
    }

    /// A read macOS refused (the prompt answered Don't Allow, or the folder
    /// turned off) names where to allow it; any other error is passed on.
    #[test]
    fn a_refused_read_names_where_to_allow_it() {
        #[cfg(target_os = "macos")]
        assert_eq!(
            unreadable("part.step", &std::io::Error::from_raw_os_error(libc::EPERM)),
            "macOS did not let AXIS CAM Studio read part.step. Turn its folder on for AXIS CAM Studio in System Settings › Privacy & Security › Files and Folders, then reimport the model."
        );
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("gone.step");
        let message = convert(
            &Running::default(),
            Path::new("/bin/sh"),
            &missing,
            READ_TIMEOUT,
            Duration::from_secs(60),
        )
        .unwrap_err();
        assert!(
            message.starts_with("gone.step could not be read: No such file or directory"),
            "{message}"
        );
    }

    #[test]
    fn a_converter_that_fails_without_a_message_is_reported_by_its_exit_status() {
        let (result, _) = run("exit 3", Duration::from_secs(60));
        assert_eq!(
            result.unwrap_err(),
            "STEP import failed: converter exited with exit status: 3"
        );
    }

    #[test]
    fn a_converter_that_fails_with_a_message_is_reported_by_it() {
        let (result, _) = run("echo 'no solids' >&2; exit 1", Duration::from_secs(60));
        assert_eq!(result.unwrap_err(), "STEP import failed: no solids\n");
    }
}
