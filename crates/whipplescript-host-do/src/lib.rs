//! Cloudflare Durable Object host binding for the sans-IO WhippleScript core
//! (DR-0033 Phase 5).
//!
//! The whip evaluation core is host-agnostic: effects are sans-IO step machines
//! ([`whipplescript_kernel::sansio`]) and the durable store is a set of traits
//! ([`whipplescript_store`]). This crate is the *durable-object* binding for
//! those seams — the Rust side of a Cloudflare Worker/DO. It is built for wasm
//! against the core with `--no-default-features` (no rusqlite), so the DO
//! supplies its own SQLite and its own `fetch`.
//!
//! Two boundaries cross into the JavaScript isolate (wired by the TS Worker shell
//! via `wasm-bindgen`/the `worker` crate in a deployment):
//!   - [`FetchClient`] — the DO's `fetch`, which fulfills a `NeedsIo(Http)` from a
//!     step machine. [`FetchHost`] adapts it into the sans-IO [`HostDriver`].
//!   - [`DoStorage`] — the DO's *synchronous* SQLite. [`DoFileStore`] implements
//!     the file seam over it (small files inline; large files spill to an object
//!     store in Phase 7).
//!
//! What is intentionally NOT here yet (the Cloudflare-runtime greenfield): the
//! full [`RuntimeStore`](whipplescript_store::RuntimeStore) implementation over
//! `DoStorage` (the DO runs the same SQL the native `SqliteStore` does, through
//! the DO SQL API), the alarms/secrets wiring (Phase 6), and the object-store
//! tier (Phase 7). Those need a live DO to build and test against; the seams they
//! plug into are the ones proven here.

use std::io;
use std::path::Path;

use whipplescript_kernel::sansio::{
    HostDriver, HttpRequest, HttpResponse, IoRequest, IoResult, TransportError,
};
use whipplescript_store::files::FileStore;

// -- HTTP: the fetch host driver ------------------------------------------

/// The DO's HTTP `fetch`, surfaced to the synchronous sans-IO core. A deployment
/// implements this over the Worker runtime's `fetch`; the TS shell awaits the
/// promise and re-enters the step machine on resolve (the suspension point the
/// sans-IO design exists for).
pub trait FetchClient {
    fn fetch(&self, request: &HttpRequest) -> Result<HttpResponse, TransportError>;
}

/// Adapts a [`FetchClient`] into the sans-IO [`HostDriver`], so any effect step
/// machine (`coerce`, an agent turn) runs on the DO by having its `NeedsIo(Http)`
/// fulfilled through the isolate's `fetch`.
pub struct FetchHost<F: FetchClient> {
    pub fetch: F,
}

impl<F: FetchClient> HostDriver for FetchHost<F> {
    fn fulfill(&self, request: &IoRequest) -> IoResult {
        match request {
            IoRequest::Http(http) => IoResult::Http(self.fetch.fetch(http)),
        }
    }
}

// -- Files: the DO storage file store -------------------------------------

/// The DO's synchronous SQLite, abstracted to the byte operations a file needs.
/// Keys are flat content paths (the DO has no directory tree). Small files live
/// inline here, transactional with the rest of the store; large files spill to a
/// platform object store behind the same handle (Phase 7).
pub trait DoStorage {
    fn read_file(&self, key: &str) -> io::Result<Option<String>>;
    fn write_file(&self, key: &str, content: &str) -> io::Result<()>;
    fn append_file(&self, key: &str, content: &str) -> io::Result<()>;
    fn file_exists(&self, key: &str) -> bool;
}

/// The file seam ([`FileStore`]) backed by DO storage: the DO binding's answer to
/// the native `NativeFileStore` (`std::fs`). Path resolution and the `file store`
/// policy boundary stay in the effect handler; only the bytes cross here.
pub struct DoFileStore<S: DoStorage> {
    pub storage: S,
}

/// A workspace path becomes a flat storage key (the DO has no filesystem tree).
fn storage_key(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

impl<S: DoStorage> FileStore for DoFileStore<S> {
    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        match self.storage.read_file(&storage_key(path))? {
            Some(content) => Ok(content),
            None => Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("no such file: {}", path.display()),
            )),
        }
    }

    fn exists(&self, path: &Path) -> bool {
        self.storage.file_exists(&storage_key(path))
    }

    fn create_dir_all(&self, _path: &Path) -> io::Result<()> {
        // Flat key space: there are no directories to create.
        Ok(())
    }

    fn write(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        self.storage
            .write_file(&storage_key(path), &String::from_utf8_lossy(bytes))
    }

    fn append(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        self.storage
            .append_file(&storage_key(path), &String::from_utf8_lossy(bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use whipplescript_kernel::sansio::{run_to_completion, Outcome, StepMachine};

    // -- fetch host --------------------------------------------------------

    struct StaticFetch {
        body: serde_json::Value,
        seen: RefCell<Option<HttpRequest>>,
    }

    impl FetchClient for StaticFetch {
        fn fetch(&self, request: &HttpRequest) -> Result<HttpResponse, TransportError> {
            *self.seen.borrow_mut() = Some(request.clone());
            Ok(HttpResponse {
                status: 200,
                body: self.body.clone(),
            })
        }
    }

    /// A trivial one-round step machine that yields one HTTP request then settles
    /// on the response status — enough to prove FetchHost drives the sans-IO core.
    struct OneShot {
        url: String,
    }

    impl StepMachine for OneShot {
        type Output = u16;
        fn step(&mut self, incoming: Option<IoResult>) -> Outcome<u16> {
            match incoming {
                None => Outcome::NeedsIo(IoRequest::Http(HttpRequest {
                    url: self.url.clone(),
                    headers: vec![],
                    body: serde_json::json!({}),
                })),
                Some(IoResult::Http(Ok(response))) => Outcome::Settle(response.status),
                Some(IoResult::Http(Err(_))) => Outcome::Settle(0),
            }
        }
    }

    #[test]
    fn fetch_host_drives_a_step_machine_over_the_do_fetch() {
        let host = FetchHost {
            fetch: StaticFetch {
                body: serde_json::json!({"ok": true}),
                seen: RefCell::new(None),
            },
        };
        let mut machine = OneShot {
            url: "https://api.anthropic.com/v1/messages".to_string(),
        };
        let status = run_to_completion(&mut machine, &host);
        assert_eq!(status, 200);
        assert!(host.fetch.seen.borrow().is_some(), "fetch was invoked");
    }

    // -- DO file store -----------------------------------------------------

    /// In-memory stand-in for the DO's SQLite (what the `worker` crate wires up).
    #[derive(Default)]
    struct MemStorage {
        files: RefCell<HashMap<String, String>>,
    }

    impl DoStorage for MemStorage {
        fn read_file(&self, key: &str) -> io::Result<Option<String>> {
            Ok(self.files.borrow().get(key).cloned())
        }
        fn write_file(&self, key: &str, content: &str) -> io::Result<()> {
            self.files
                .borrow_mut()
                .insert(key.to_string(), content.to_string());
            Ok(())
        }
        fn append_file(&self, key: &str, content: &str) -> io::Result<()> {
            self.files
                .borrow_mut()
                .entry(key.to_string())
                .or_default()
                .push_str(content);
            Ok(())
        }
        fn file_exists(&self, key: &str) -> bool {
            self.files.borrow().contains_key(key)
        }
    }

    #[test]
    fn do_file_store_round_trips_through_the_file_seam() {
        let files: &dyn FileStore = &DoFileStore {
            storage: MemStorage::default(),
        };
        let path = Path::new("notes/todo.txt");

        assert!(!files.exists(path));
        files
            .create_dir_all(Path::new("notes"))
            .expect("mkdir noop");
        files.write(path, b"hello").expect("write");
        assert!(files.exists(path));
        assert_eq!(files.read_to_string(path).expect("read"), "hello");
        files.append(path, b" world").expect("append");
        assert_eq!(files.read_to_string(path).expect("read"), "hello world");
        assert!(files.read_to_string(Path::new("missing")).is_err());
    }
}
