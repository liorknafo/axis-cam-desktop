use std::{env, path::PathBuf, process::Command};
/// The Open CASCADE libraries the helper links, in link order.
const LIBS: [&str; 7] = [
    "TKDESTEP",
    "TKXSBase",
    "TKMesh",
    "TKBRep",
    "TKTopAlgo",
    "TKMath",
    "TKernel",
];
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
    // links through its rpath, so it must be built again. Watch those files,
    // not the lib directory: cargo walks a whole directory it is given, which
    // is every file of /usr/lib with the default Linux prefix. The versioned
    // file each name points to is watched too: an upgrade removes it, which
    // reruns the script even when the new library's own date is older.
    println!(
        "cargo:rerun-if-changed={}",
        prefix
            .join("include/opencascade/Standard_Version.hxx")
            .display()
    );
    let extension = if cfg!(target_os = "macos") {
        "dylib"
    } else {
        "so"
    };
    for name in LIBS {
        let link = prefix.join("lib").join(format!("lib{name}.{extension}"));
        println!("cargo:rerun-if-changed={}", link.display());
        if let Ok(versioned) = link.canonicalize() {
            println!("cargo:rerun-if-changed={}", versioned.display());
        }
    }
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
        .args(LIBS.map(|name| format!("-l{name}")))
        .status()
        .expect("Run C++ compiler for STEP helper");
    assert!(status.success(), "STEP helper compilation failed");
    println!("cargo:rustc-env=AXIS_BUILT_STEP_HELPER={}", out.display());
}
