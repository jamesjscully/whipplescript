//! File-store seam (DR-0033 Phase 4).
//!
//! File effects (`file.read` / `file.write` / `file.import` / `file.export`)
//! route their raw byte I/O through this trait so a second physical tier can back
//! files without the language changing: the durable-object host inlines small
//! files in DO SQLite (transactional with fact-derivation) and spills large ones
//! to a platform object store (Phase 7), while the native CLI backs files with
//! `std::fs` under a workspace root. Path resolution and the `file store` policy
//! boundary stay in the effect handler; only the byte I/O crosses this seam.
//!
//! The trait is intentionally minimal — exactly the operations the file effects
//! perform today. The content-hash-handle / tiering model of DR-0033 Decision 4
//! is layered on later (Phase 7) behind the same seam.

use std::io;
use std::path::Path;

/// The byte-I/O operations a file effect performs, abstracted over the physical
/// backing. Object-safe so a durable-object backend can be used as `&dyn`.
pub trait FileStore {
    /// Read the whole file at `path` as UTF-8 text.
    fn read_to_string(&self, path: &Path) -> io::Result<String>;

    /// Whether a file exists at `path` (the write-mode precondition check).
    fn exists(&self, path: &Path) -> bool;

    /// Ensure the directory at `path` (and its parents) exists.
    fn create_dir_all(&self, path: &Path) -> io::Result<()>;

    /// Write `bytes` to `path`, replacing any existing contents.
    fn write(&self, path: &Path, bytes: &[u8]) -> io::Result<()>;

    /// Append `bytes` to `path`, creating it if absent.
    fn append(&self, path: &Path, bytes: &[u8]) -> io::Result<()>;

    /// Remove the file at `path`. Restorable-context restore (RC-5) uses this to
    /// drop mediated files created after a cut so the file plane equals exactly
    /// the cut manifest. Removing an absent path is a no-op (idempotent).
    fn remove(&self, path: &Path) -> io::Result<()>;
}

/// Native backing: files live on the local filesystem (the workspace root is
/// applied by the caller before the path reaches this store).
pub struct NativeFileStore;

impl FileStore for NativeFileStore {
    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        std::fs::read_to_string(path)
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        std::fs::create_dir_all(path)
    }

    fn write(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        std::fs::write(path, bytes)
    }

    fn append(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        use std::io::Write as _;
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?
            .write_all(bytes)
    }

    fn remove(&self, path: &Path) -> io::Result<()> {
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            // Idempotent: an already-absent file is not an error.
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drive the native store through `&dyn FileStore`: proves object-safety (a
    /// boxed durable-object backend is legal) and that write / read / append /
    /// exists round-trip as the file effects expect.
    #[test]
    fn native_file_store_round_trips_through_the_trait() {
        let dir = std::env::temp_dir().join(format!(
            "whipplescript-filestore-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos(),
        ));
        let path = dir.join("nested/note.txt");
        let files: &dyn FileStore = &NativeFileStore;

        assert!(!files.exists(&path));
        files
            .create_dir_all(path.parent().expect("parent"))
            .expect("mkdir");
        files.write(&path, b"hello").expect("write");
        assert!(files.exists(&path));
        assert_eq!(files.read_to_string(&path).expect("read"), "hello");
        files.append(&path, b" world").expect("append");
        assert_eq!(files.read_to_string(&path).expect("read"), "hello world");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
