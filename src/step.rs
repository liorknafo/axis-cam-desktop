//! Separate process isolates CAD conversion from the GUI and enforces a timeout.
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub fn load(path: &Path) -> Result<Vec<[[f32; 3]; 3]>, String> {
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let sibling = executable.parent().unwrap().join("axis-step");
    // A shipped .app must never fall back to the developer's build directory.
    let bundled = executable.components().any(|p| p.as_os_str() == "Contents");
    let helper = if sibling.exists() || bundled {
        sibling
    } else {
        option_env!("AXIS_BUILT_STEP_HELPER")
            .map(PathBuf::from)
            .ok_or("STEP support was disabled in this build. Rebuild with Open CASCADE.")?
    };
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
