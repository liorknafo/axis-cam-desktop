//! AXIS CAM's core (LIO-729): the model, geometry, toolpath strategies,
//! simulation, posting, analysis, proposal, plan check, storage and importers,
//! with no user interface, machine control or network code. The app
//! (`axis-cam`) builds on it, so an edit in the app does not recompile it and
//! its tests build and run on their own (`cargo test -p axis-core`).

pub mod adaptive;
pub mod air;
pub mod analysis;
pub mod cam;
pub mod centre;
pub mod chamfer;
pub mod chuck;
pub mod corner;
pub mod dxf;
pub mod envelope;
pub mod facing;
pub mod finishing;
pub mod forces;
pub mod gpu_mesh;
pub mod groove;
pub mod guide;
pub mod holding;
pub mod hole_stages;
pub mod indexed;
pub mod lathe;
pub mod machine_profile;
pub mod manual;
pub mod material;
pub mod mesh;
pub mod model;
pub mod names;
pub mod ngc;
pub mod orient;
pub mod parallels;
pub mod picks;
pub mod plan_check;
pub mod pocket;
pub mod post;
pub mod post_check;
pub mod profile_draft;
pub mod progress;
pub mod proposal;
pub mod rest;
pub mod rotary_check;
pub mod round_hold;
pub mod section;
pub mod sheet;
pub mod simulation;
pub mod slot;
pub mod step;
pub mod stock;
pub mod stock_surface;
pub mod storage;
pub mod surface;
pub mod thread;
pub mod threading;
pub mod three_mf;
pub mod tooth_load;
pub mod turn;
pub mod uncut;
pub mod volume;

pub use model::*;
/// The repository's root: where the tests find `examples/` and `assets/`
/// (this crate lives in `crates/axis-core`).
#[cfg(any(test, feature = "test-support"))]
pub fn repo_root() -> &'static std::path::Path {
    std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}
