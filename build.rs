use std::{env, path::PathBuf, process::Command};
fn main() {
    println!("cargo:rerun-if-changed=native/step_preview.cpp");
    println!("cargo:rerun-if-env-changed=OCCT_PREFIX");
    println!("cargo:rerun-if-env-changed=AXIS_SKIP_STEP");
    println!("cargo:rerun-if-env-changed=CXX");
    if env::var_os("AXIS_SKIP_STEP").is_some() {
        println!(
            "cargo:warning=STEP helper disabled explicitly; STEP imports will report unavailable."
        );
        return;
    }
    let prefix = env::var_os("OCCT_PREFIX")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            if cfg!(target_os = "macos") {
                PathBuf::from("/opt/homebrew/opt/opencascade")
            } else {
                PathBuf::from("/usr")
            }
        });
    // An upgrade of Open CASCADE renames the versioned libraries the helper
    // links through its rpath, so it must be built again.
    println!(
        "cargo:rerun-if-changed={}",
        prefix
            .join("include/opencascade/Standard_Version.hxx")
            .display()
    );
    println!("cargo:rerun-if-changed={}", prefix.join("lib").display());
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("axis-step");
    let include = prefix.join("include/opencascade");
    assert!(
        include.join("STEPControl_Reader.hxx").exists(),
        "Install Open CASCADE (macOS: brew install opencascade), set OCCT_PREFIX, or explicitly build without STEP using AXIS_SKIP_STEP=1"
    );
    let status = Command::new(env::var("CXX").unwrap_or_else(|_| "c++".into()))
        .args(["-std=c++17", "-O2", "native/step_preview.cpp", "-o"])
        .arg(&out)
        .arg(format!("-I{}", include.display()))
        .arg(format!("-L{}", prefix.join("lib").display()))
        .arg(format!("-Wl,-rpath,{}", prefix.join("lib").display()))
        .args([
            "-lTKDESTEP",
            "-lTKXSBase",
            "-lTKMesh",
            "-lTKBRep",
            "-lTKTopAlgo",
            "-lTKMath",
            "-lTKernel",
        ])
        .status()
        .expect("Run C++ compiler for STEP helper");
    assert!(status.success(), "STEP helper compilation failed");
    println!("cargo:rustc-env=AXIS_BUILT_STEP_HELPER={}", out.display());
}
