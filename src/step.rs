//! Separate process isolates CAD conversion from the GUI and enforces a timeout.
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
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

pub fn load(path: &Path) -> Result<Vec<[[f32; 3]; 3]>, String> {
    let helper = helper()?;
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
    let mut child = command
        .arg(path)
        .arg(&result)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(fs::File::create(&error).map_err(|e| e.to_string())?)
        .spawn()
        .map_err(|e| format!("Could not start bundled STEP reader: {e}"))?;
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e.to_string());
            }
        }
        if start.elapsed() > Duration::from_secs(120) {
            let _ = child.kill();
            let _ = child.wait();
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
