//! Workspace export/import bundles — the handoff carrier
//! (untie-substrate readiness tracker Phase 2; un-tie's
//! `STATE_BEFORE_HOME` ships manifest + reachable blobs, cleaner than
//! `git bundle`).
//!
//! A bundle is one self-contained JSON document: the branch row
//! (lineage snapshot), its head manifest, every reachable blob, and the
//! branch's recorded cuts so CHANGE IDENTITY travels (dual identity: a
//! bundle-imported change reunifies at a later merge instead of
//! re-conflicting as an ancestry-less duplicate). It is
//! **erasure-respecting by construction**: an erased blob travels as a
//! tombstone entry — hash + size, NO body — so the bytes that were
//! erased never escape through the handoff path again (the leak un-tie
//! documented in ADR 0008/0018 and could not fix over git).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::branches::{BranchRow, CutRow};
use crate::content::{BlobStatus, ContentBlobs};
use crate::StoreResult;

pub const BUNDLE_FORMAT: &str = "whipplescript.bundle.v1";

/// One blob in the bundle. `body: None` + `erased: true` is the
/// tombstone shape — identity travels, payload does not.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BundleBlob {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    pub byte_len: u64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub erased: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceBundle {
    pub format: String,
    /// The exported branch's row at export time (lineage snapshot).
    pub branch: BranchRow,
    /// The head change id, so intent identity survives the handoff.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_change_id: Option<String>,
    pub manifest: BTreeMap<String, String>,
    pub blobs: Vec<BundleBlob>,
    /// The branch's recorded cuts (identity + provenance travel).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cuts: Vec<CutRow>,
}

/// Collect the manifest's reachable blobs through the content seam,
/// erasure-respecting: tombstoned payloads never travel.
pub fn collect_blobs(
    manifest: &BTreeMap<String, String>,
    content: &dyn ContentBlobs,
) -> StoreResult<Vec<BundleBlob>> {
    let mut ids: Vec<&String> = manifest.values().collect();
    ids.sort();
    ids.dedup();
    let mut blobs = Vec::new();
    for id in ids {
        match content.status(id)? {
            BlobStatus::Live { byte_len } => blobs.push(BundleBlob {
                id: id.clone(),
                body: content.get(id)?,
                byte_len,
                erased: false,
            }),
            BlobStatus::Erased { byte_len } => blobs.push(BundleBlob {
                id: id.clone(),
                body: None,
                byte_len,
                erased: true,
            }),
            // A hash the store has never seen: carried as an honest
            // zero-knowledge tombstone rather than silently dropped —
            // the receiving side sees the manifest references it.
            BlobStatus::Unknown => blobs.push(BundleBlob {
                id: id.clone(),
                body: None,
                byte_len: 0,
                erased: false,
            }),
        }
    }
    Ok(blobs)
}

/// Outcome of importing a bundle.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BundleImportOutcome {
    Imported {
        branch_id: String,
        cut_id: String,
        manifest_hash: String,
    },
    /// Idempotent re-materialization: the branch already carries the
    /// bundle's state.
    AlreadyPresent { branch_id: String },
    /// The branch exists locally with DIFFERENT content: refused
    /// honestly (import never clobbers local work).
    DivergentBranch {
        branch_id: String,
        local_head_manifest_hash: Option<String>,
    },
}

#[cfg(all(test, feature = "native"))]
mod tests {
    use super::*;
    use crate::content::EraseOutcome;
    use crate::vcs::NativeWorkspaceVcs;

    fn vcs(tag: &str) -> NativeWorkspaceVcs {
        let dir = std::env::temp_dir().join(format!(
            "whipplescript-bundle-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos(),
        ));
        NativeWorkspaceVcs::open(dir.join("branches.sqlite"), dir.join("content.sqlite"))
            .expect("open vcs")
    }

    /// The handoff loop: export a branch, import it into a FRESH
    /// workspace, get an identical working state — idempotently (the
    /// second import is `AlreadyPresent`), with change identity carried
    /// (dual identity: the imported head resolves to its original
    /// change id).
    #[test]
    fn export_import_rematerializes_idempotently_with_identity() {
        let mut source = vcs("src");
        source.init("t0").expect("init");
        source
            .create_branch("draft_a", Some("handoff"), "main", "t1")
            .expect("create");
        source
            .write("draft_a", "a.md", Some("state before home"), "cut_a1", "t2")
            .expect("write");
        source
            .write("draft_a", "b.md", Some("second file"), "cut_a2", "t3")
            .expect("write");
        let bundle = source
            .export_bundle("draft_a")
            .expect("export")
            .expect("branch exists");
        assert_eq!(bundle.format, BUNDLE_FORMAT);
        assert_eq!(bundle.manifest.len(), 2);
        assert!(bundle.blobs.iter().all(|blob| blob.body.is_some()));
        assert_eq!(bundle.head_change_id.as_deref(), Some("cut_a2"));

        // The bundle is a plain JSON document (the wire shape).
        let wire = serde_json::to_string(&bundle).expect("encode");
        let decoded: WorkspaceBundle = serde_json::from_str(&wire).expect("decode");

        let mut target = vcs("dst");
        target.init("t4").expect("init");
        let outcome = target.import_bundle(&decoded, "t5").expect("import");
        assert!(matches!(outcome, BundleImportOutcome::Imported { .. }));
        assert_eq!(
            target.read("draft_a", "a.md").expect("read").as_deref(),
            Some("state before home")
        );
        assert_eq!(
            target.read("draft_a", "b.md").expect("read").as_deref(),
            Some("second file")
        );
        // Change identity traveled: the imported head carries the
        // ORIGINAL change id, not a fresh one.
        let report = target
            .status_report("draft_a")
            .expect("status")
            .expect("branch");
        assert_eq!(report.head_change_id.as_deref(), Some("cut_a2"));
        // Idempotent re-materialization.
        assert_eq!(
            target.import_bundle(&decoded, "t6").expect("re-import"),
            BundleImportOutcome::AlreadyPresent {
                branch_id: "draft_a".to_owned()
            }
        );
        // Local divergence is refused, never clobbered.
        target
            .write("draft_a", "a.md", Some("local edit"), "cut_local", "t7")
            .expect("write");
        assert!(matches!(
            target.import_bundle(&decoded, "t8").expect("import"),
            BundleImportOutcome::DivergentBranch { .. }
        ));
        assert_eq!(
            target.read("draft_a", "a.md").expect("read").as_deref(),
            Some("local edit")
        );
    }

    /// Un-tie's content-erasure invariants discharged over the
    /// substrate:
    /// - `HISTORY_PRESERVED` — after per-blob erasure, every manifest,
    ///   cut, op, and lineage record still reads; only the payload is
    ///   gone, and reads degrade honestly.
    /// - `EXPORTED_COPY_NOT_RECALLED` — a bundle exported BEFORE erasure
    ///   keeps its payload (erasure is local, honestly so); a bundle
    ///   exported AFTER carries a tombstone — the erased bytes never
    ///   travel again.
    #[test]
    fn erasure_preserves_history_and_respects_exports() {
        let mut vcs = vcs("erase");
        vcs.init("t0").expect("init");
        vcs.create_branch("draft_a", None, "main", "t1")
            .expect("create");
        vcs.write("draft_a", "secret.md", Some("the payload"), "cut_a1", "t2")
            .expect("write");
        vcs.write("draft_a", "kept.md", Some("stays"), "cut_a2", "t3")
            .expect("write");
        let before = vcs
            .export_bundle("draft_a")
            .expect("export")
            .expect("branch");
        // Erase the secret's payload.
        let (hash, outcome) = vcs
            .erase_path("draft_a", "secret.md", "t4")
            .expect("erase")
            .expect("path exists");
        assert!(matches!(outcome, EraseOutcome::Erased { byte_len: 11 }));
        // The retry is honest.
        let (_, retry) = vcs
            .erase_path("draft_a", "secret.md", "t5")
            .expect("erase")
            .expect("path exists");
        assert_eq!(retry, EraseOutcome::AlreadyErased);

        // HISTORY_PRESERVED: the manifest still references the hash, the
        // cuts and ops still read, the branch still works.
        let manifest = vcs.manifest("draft_a").expect("manifest").expect("branch");
        assert_eq!(manifest.get("secret.md"), Some(&hash));
        assert_eq!(
            vcs.read("draft_a", "secret.md").expect("read"),
            None,
            "the payload is honestly gone"
        );
        assert_eq!(
            vcs.read("draft_a", "kept.md").expect("read").as_deref(),
            Some("stays")
        );
        assert!(vcs.get_cut("cut_a1").expect("cut").is_some());
        assert!(!vcs.list_ops(20).expect("ops").is_empty());
        let diff = vcs
            .diff_against("draft_a", None, 3)
            .expect("diff")
            .expect("branch");
        let secret_entry = diff
            .iter()
            .find(|entry| entry.path == "secret.md")
            .expect("entry");
        assert!(
            secret_entry.payload_unavailable,
            "the diff degrades honestly, never fabricates"
        );
        vcs.write("draft_a", "secret.md", Some("rewritten"), "cut_a3", "t6")
            .expect("the branch keeps working after erasure");

        // EXPORTED_COPY_NOT_RECALLED: the pre-erasure bundle still
        // carries the payload — erasure is local and the system does not
        // pretend otherwise...
        let exported_secret = before
            .blobs
            .iter()
            .find(|blob| blob.id == hash)
            .expect("exported blob");
        assert_eq!(exported_secret.body.as_deref(), Some("the payload"));
        // ...and it re-materializes elsewhere untouched by the erasure.
        let mut elsewhere = vcs2();
        elsewhere.init("t0").expect("init");
        elsewhere.import_bundle(&before, "t1").expect("import");
        assert_eq!(
            elsewhere
                .read("draft_a", "secret.md")
                .expect("read")
                .as_deref(),
            Some("the payload"),
            "the exported copy is not recalled"
        );

        // ...but a NEW export after erasure carries the tombstone: hash
        // and size travel, the bytes do not. (The head moved past the
        // erased state above, so export the recorded pre-rewrite cut's
        // manifest via a fresh branch pinned at it.)
        vcs.fork_with_lineage("audit", None, "draft_a", Some("cut_a2"), "t7")
            .expect("fork at the erased-payload cut");
        let after = vcs.export_bundle("audit").expect("export").expect("branch");
        let tombstone = after
            .blobs
            .iter()
            .find(|blob| blob.id == hash)
            .expect("tombstone entry");
        assert_eq!(tombstone.body, None, "tombstoned bytes never travel");
        assert!(tombstone.erased);
        assert_eq!(tombstone.byte_len, 11, "identity and size retained");
    }

    fn vcs2() -> NativeWorkspaceVcs {
        vcs("elsewhere")
    }
}
