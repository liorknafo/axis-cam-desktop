//! rfd's file dialogs have no Android backend. The shop tablet build swaps
//! this in under rfd's name: every dialog answers "cancelled".
// ponytail: no file picking on the tablet yet; the Storage Access Framework when it needs to open projects itself.
use std::path::{Path, PathBuf};

#[derive(Default)]
pub struct AsyncFileDialog;

pub struct FileHandle(PathBuf);

impl AsyncFileDialog {
    pub fn new() -> Self {
        Self
    }
    pub fn add_filter(self, _name: impl Into<String>, _extensions: &[impl ToString]) -> Self {
        self
    }
    pub fn set_directory<P: AsRef<Path>>(self, _path: P) -> Self {
        self
    }
    pub fn set_file_name(self, _name: impl Into<String>) -> Self {
        self
    }
    pub fn set_title(self, _title: impl Into<String>) -> Self {
        self
    }
    pub async fn pick_file(self) -> Option<FileHandle> {
        None
    }
    pub async fn pick_folder(self) -> Option<FileHandle> {
        None
    }
    pub async fn save_file(self) -> Option<FileHandle> {
        None
    }
}

impl FileHandle {
    pub fn path(&self) -> &Path {
        &self.0
    }
    pub fn file_name(&self) -> String {
        self.0.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    }
}
