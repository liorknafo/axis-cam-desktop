//! Separate process isolates CAD conversion from the GUI and enforces a timeout.
use std::{
    fs,
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

pub fn load(path: &Path) -> Result<Vec<[[f32; 3]; 3]>, String> {
    convert(&RUNNING, &helper()?, path, TIMEOUT)
}

// Split from `load` so a test can run a stand-in converter under a short timeout,
// registered in a slot of its own.
fn convert(
    running: &Running,
    helper: &Path,
    path: &Path,
    timeout: Duration,
) -> Result<Vec<[[f32; 3]; 3]>, String> {
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
    let bytes = fs::read(result).map_err(|e| format!("STEP preview missing: {e}"))?;
    super::mesh::stl(&bytes)
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
        let result = convert(running, Path::new("/bin/sh"), &path, timeout);
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
