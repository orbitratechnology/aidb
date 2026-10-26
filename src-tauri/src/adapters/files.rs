//! Files the user picks in a native dialog. The webview never receives file access: the core
//! shows the dialog, reads or writes the chosen file, and returns only its name.

use std::io::{self, Read};
use std::path::{Path, PathBuf};

use tauri::{Runtime, WebviewWindow};
use tauri_plugin_dialog::DialogExt;
use thiserror::Error;

use crate::errors::{AppError, AppResult, ErrorCode};

/// Title, suggested name and file type for a dialog.
#[derive(Debug, Clone, Copy)]
pub struct DialogRequest<'a> {
    pub title: &'a str,
    pub file_name: &'a str,
    pub filter_name: &'a str,
    pub extensions: &'a [&'a str],
}

#[derive(Debug, Error)]
pub enum FileError {
    #[error("the file is larger than {0} bytes")]
    TooLarge(u64),
    #[error("file access failed")]
    Io(#[from] io::Error),
}

impl From<FileError> for AppError {
    fn from(error: FileError) -> Self {
        match error {
            FileError::TooLarge(_) => Self::invalid_input("The file is too large."),
            FileError::Io(io) => {
                let (code, message) = match io.kind() {
                    io::ErrorKind::PermissionDenied => (
                        ErrorCode::PermissionDenied,
                        "aidb is not allowed to use that file.",
                    ),
                    io::ErrorKind::NotFound => (ErrorCode::NotFound, "The file no longer exists."),
                    _ => (ErrorCode::Storage, "The file could not be read or written."),
                };
                Self::new(code, message).with_cause(io)
            }
        }
    }
}

/// Asks where to save, then writes `bytes` there. Returns the file name, or `None` if the user
/// cancelled.
///
/// # Errors
///
/// When the file cannot be written.
pub async fn save_via_dialog<R: Runtime>(
    window: &WebviewWindow<R>,
    request: DialogRequest<'_>,
    bytes: Vec<u8>,
) -> AppResult<Option<String>> {
    let dialog = window
        .dialog()
        .file()
        .set_parent(window)
        .set_title(request.title)
        .set_file_name(request.file_name)
        .add_filter(request.filter_name, request.extensions);
    blocking(move || {
        let Some(path) = dialog.blocking_save_file() else {
            return Ok(None);
        };
        let path = local_path(path)?;
        std::fs::write(&path, bytes).map_err(FileError::from)?;
        Ok(Some(file_name(&path)))
    })
    .await
}

/// Asks for a file, then reads at most `max_bytes` of it. Returns its name and contents, or
/// `None` if the user cancelled.
///
/// # Errors
///
/// When the file is larger than `max_bytes` or cannot be read.
pub async fn open_via_dialog<R: Runtime>(
    window: &WebviewWindow<R>,
    request: DialogRequest<'_>,
    max_bytes: u64,
) -> AppResult<Option<(String, Vec<u8>)>> {
    let dialog = window
        .dialog()
        .file()
        .set_parent(window)
        .set_title(request.title)
        .add_filter(request.filter_name, request.extensions);
    blocking(move || {
        let Some(path) = dialog.blocking_pick_file() else {
            return Ok(None);
        };
        let path = local_path(path)?;
        let bytes = read_capped(&path, max_bytes)?;
        Ok(Some((file_name(&path), bytes)))
    })
    .await
}

/// Reads a whole file, refusing one larger than `max_bytes` without reading it all.
///
/// # Errors
///
/// [`FileError::TooLarge`] or an I/O error.
pub fn read_capped(path: &Path, max_bytes: u64) -> Result<Vec<u8>, FileError> {
    let file = std::fs::File::open(path)?;
    if file.metadata()?.len() > max_bytes {
        return Err(FileError::TooLarge(max_bytes));
    }
    let mut bytes = Vec::new();
    // The size can change after the check; never read more than one byte past the limit.
    file.take(max_bytes + 1).read_to_end(&mut bytes)?;
    if u64::try_from(bytes.len()).map_or(true, |len| len > max_bytes) {
        return Err(FileError::TooLarge(max_bytes));
    }
    Ok(bytes)
}

fn local_path(path: tauri_plugin_dialog::FilePath) -> AppResult<PathBuf> {
    path.into_path()
        .map_err(|_| AppError::invalid_input("Choose a file on this computer."))
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned())
}

async fn blocking<T, F>(work: F) -> AppResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> AppResult<T> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(AppError::internal)?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_files_are_read_whole() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("connections.json");
        std::fs::write(&path, b"{}").expect("write");

        assert_eq!(read_capped(&path, 10).expect("read"), b"{}");
    }

    #[test]
    fn files_over_the_limit_are_refused() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("big.json");
        std::fs::write(&path, vec![b'x'; 11]).expect("write");

        assert!(matches!(
            read_capped(&path, 10),
            Err(FileError::TooLarge(10))
        ));
    }

    #[test]
    fn io_errors_map_to_safe_messages() {
        let dir = tempfile::tempdir().expect("temp dir");
        let error = read_capped(&dir.path().join("missing.json"), 10).expect_err("missing");

        let app_error = AppError::from(error);

        assert_eq!(app_error.code(), ErrorCode::NotFound);
        assert_eq!(app_error.message(), "The file no longer exists.");
    }
}
