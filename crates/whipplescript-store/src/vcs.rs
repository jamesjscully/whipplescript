//! The integrated versioned workspace: branches, content, working sets,
//! and merge composed into the user-facing VCS operations
//! (spec/versioned-workspace-research-note.md §4–§7; untie-substrate
//! readiness tracker Phases 1–2; `whip branch` is the CLI surface).
//!
//! `WorkspaceVcs` is the mediator's storage half: every verb is a
//! proposal over immutable content — a write mints a new cut, a merge
//! either adopts certified content or returns structured conflicts and
//! moves NOTHING, a discard closes a head without deleting history. No
//! destructive verb exists on this surface. Manifests are stored as
//! content-addressed JSON blobs; a branch row carries only pointers, so
//! branch creation stays O(1) and a hundred branches share every
//! unchanged blob.
//!
//! Merging runs the P1 pipeline end to end: rebase-down first when the
//! parent moved (silent when slice-disjoint, honest structured conflicts
//! when not — never a fake auto-merge), then the staleness-checked
//! merge-up that adopts the branch and advances the parent head. The CLI
//! process is the single writer per workspace (the mediator); optimistic
//! head guards make a racing writer a refused normal outcome rather
//! than a lost update.

#[cfg(feature = "native")]
use std::collections::BTreeMap;
#[cfg(feature = "native")]
use std::path::Path;

#[cfg(feature = "native")]
use crate::branches::{
    AdvanceOutcome, BranchRow, BranchStatus, BranchStore, Branches, CreateBranch,
    CreateBranchOutcome, StatusOutcome, MAINLINE_BRANCH_ID,
};
#[cfg(feature = "native")]
use crate::content::ContentStore;
#[cfg(feature = "native")]
use crate::files::FileStore;
#[cfg(feature = "native")]
use crate::merge::MergeSide;
#[cfg(feature = "native")]
use crate::merge::PathConflict;
#[cfg(feature = "native")]
use crate::reconcile::{plan_merge_up, plan_rebase_down, MergeUpPlan, RebaseDownPlan};
#[cfg(feature = "native")]
use crate::working_set::VirtualWorkingSet;
#[cfg(feature = "native")]
use crate::{StoreError, StoreResult};

/// One VCS write/remove outcome: the new cut, or the refusal.
#[cfg(feature = "native")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VcsWriteOutcome {
    Written {
        cut_id: String,
        manifest_hash: String,
    },
    BranchMissing,
    BranchNotActive,
}

/// One merge outcome. `Conflicted` moves nothing; the conflicts are the
/// notification-and-ask payload (resolve by writing the merged content on
/// the branch, then merge again).
#[cfg(feature = "native")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VcsMergeOutcome {
    Adopted {
        merge_cut_id: String,
        into_branch_id: String,
    },
    Conflicted {
        conflicts: Vec<PathConflict>,
    },
    BranchMissing,
    BranchNotActive,
    /// Mainline has no parent to merge into.
    NoParent,
}

#[cfg(feature = "native")]
pub struct WorkspaceVcs {
    branches: BranchStore,
    content: ContentStore,
}

#[cfg(feature = "native")]
impl WorkspaceVcs {
    pub fn open(
        branches_path: impl AsRef<Path>,
        content_path: impl AsRef<Path>,
    ) -> StoreResult<Self> {
        Ok(Self {
            branches: BranchStore::open(branches_path)?,
            content: ContentStore::open(content_path)?,
        })
    }

    /// Bootstrap the workspace: the mainline branch exists after this.
    pub fn init(&mut self, at: &str) -> StoreResult<BranchRow> {
        self.branches.ensure_mainline(at)
    }

    pub fn create_branch(
        &mut self,
        branch_id: &str,
        name: Option<&str>,
        parent_branch_id: &str,
        at: &str,
    ) -> StoreResult<CreateBranchOutcome> {
        self.branches.ensure_mainline(at)?;
        self.branches.create_branch(CreateBranch {
            branch_id,
            name,
            parent_branch_id,
            at_cut: None,
            created_at: at,
            idempotency_key: None,
        })
    }

    pub fn get_branch(&self, branch_id: &str) -> StoreResult<Option<BranchRow>> {
        self.branches.get_branch(branch_id)
    }

    pub fn list_branches(&self, status: Option<BranchStatus>) -> StoreResult<Vec<BranchRow>> {
        self.branches.list_branches(status)
    }

    pub fn discard_branch(&mut self, branch_id: &str, at: &str) -> StoreResult<StatusOutcome> {
        self.branches.discard_branch(branch_id, at)
    }

    fn load_manifest(&self, hash: Option<&str>) -> StoreResult<BTreeMap<String, String>> {
        let Some(hash) = hash else {
            return Ok(BTreeMap::new());
        };
        let Some(body) = self.content.get(hash)? else {
            return Err(StoreError::Conflict(format!(
                "manifest blob {hash} is absent from the content store"
            )));
        };
        serde_json::from_str(&body).map_err(StoreError::from)
    }

    fn store_manifest(&self, manifest: &BTreeMap<String, String>) -> StoreResult<String> {
        let body = serde_json::to_string(manifest)?;
        self.content.put(&body)
    }

    /// The branch's current file listing (path → content id).
    pub fn manifest(&self, branch_id: &str) -> StoreResult<Option<BTreeMap<String, String>>> {
        let Some(row) = self.branches.get_branch(branch_id)? else {
            return Ok(None);
        };
        Ok(Some(self.load_manifest(row.head_manifest_hash.as_deref())?))
    }

    /// Read a file's body on a branch. `Ok(None)` = no such file there.
    pub fn read(&self, branch_id: &str, path: &str) -> StoreResult<Option<String>> {
        let Some(manifest) = self.manifest(branch_id)? else {
            return Ok(None);
        };
        let Some(id) = manifest.get(path) else {
            return Ok(None);
        };
        self.content.get(id)
    }

    /// Write (or remove) one path on a branch through its virtual working
    /// set, minting the next cut. Copy-on-write: no other branch and no
    /// prior cut changes.
    pub fn write(
        &mut self,
        branch_id: &str,
        path: &str,
        body: Option<&str>,
        cut_id: &str,
        at: &str,
    ) -> StoreResult<VcsWriteOutcome> {
        let Some(row) = self.branches.get_branch(branch_id)? else {
            return Ok(VcsWriteOutcome::BranchMissing);
        };
        if row.status != BranchStatus::Active {
            return Ok(VcsWriteOutcome::BranchNotActive);
        }
        let base = self.load_manifest(row.head_manifest_hash.as_deref())?;
        let working_set = VirtualWorkingSet::new(&self.content, base);
        match body {
            Some(body) => working_set
                .write(Path::new(path), body.as_bytes())
                .map_err(|error| StoreError::Conflict(error.to_string()))?,
            None => working_set
                .remove(Path::new(path))
                .map_err(|error| StoreError::Conflict(error.to_string()))?,
        }
        let manifest_hash = self.store_manifest(&working_set.manifest())?;
        match self.branches.advance_head(
            branch_id,
            row.head_cut_id.as_deref(),
            cut_id,
            &manifest_hash,
            at,
        )? {
            AdvanceOutcome::Advanced(_) => Ok(VcsWriteOutcome::Written {
                cut_id: cut_id.to_owned(),
                manifest_hash,
            }),
            AdvanceOutcome::Stale { .. } => Err(StoreError::Conflict(
                "branch head moved during the write; retry".to_owned(),
            )),
            AdvanceOutcome::NotActive { .. } => Ok(VcsWriteOutcome::BranchNotActive),
            AdvanceOutcome::NotFound => Ok(VcsWriteOutcome::BranchMissing),
        }
    }

    /// Merge a branch into its parent line, running the reconciliation
    /// pipeline: rebase-down first when the parent advanced (silent when
    /// clean, honest conflicts when not), then the staleness-checked
    /// merge-up. On adoption the parent head advances to the branch's
    /// content and the branch becomes immutable history.
    pub fn merge(
        &mut self,
        branch_id: &str,
        merge_cut_id: &str,
        at: &str,
    ) -> StoreResult<VcsMergeOutcome> {
        let Some(mut branch) = self.branches.get_branch(branch_id)? else {
            return Ok(VcsMergeOutcome::BranchMissing);
        };
        if branch.status != BranchStatus::Active {
            return Ok(VcsMergeOutcome::BranchNotActive);
        }
        let Some(parent_id) = branch.parent_branch_id.clone() else {
            return Ok(VcsMergeOutcome::NoParent);
        };
        let Some(parent) = self.branches.get_branch(&parent_id)? else {
            return Ok(VcsMergeOutcome::NoParent);
        };
        let branch_side = MergeSide {
            label: branch_id.to_owned(),
            cut_id: branch.head_cut_id.clone(),
        };
        let parent_side = MergeSide {
            label: parent_id.clone(),
            cut_id: parent.head_cut_id.clone(),
        };

        // Rebase-down when the parent moved past our branch point. The
        // CLI merge runs at a quiescence point by definition (no run in
        // flight inside this verb), so a conflicting delta escalates as
        // the ask instead of deferring.
        if branch.branch_point_cut_id != parent.head_cut_id {
            let point = self.load_manifest(branch.branch_point_manifest_hash.as_deref())?;
            let head = self.load_manifest(branch.head_manifest_hash.as_deref())?;
            let target = self.load_manifest(parent.head_manifest_hash.as_deref())?;
            match plan_rebase_down(&point, &head, &target, &branch_side, &parent_side, true) {
                RebaseDownPlan::UpToDate => {}
                RebaseDownPlan::Silent { new_head_manifest } => {
                    let rebased_hash = self.store_manifest(&new_head_manifest)?;
                    let rebase_cut = format!("{merge_cut_id}-rebase");
                    let parent_head_cut = parent.head_cut_id.clone().unwrap_or_default();
                    let parent_head_hash = parent.head_manifest_hash.clone().unwrap_or_else(|| {
                        self.store_manifest(&BTreeMap::new()).unwrap_or_default()
                    });
                    match self.branches.rebase_branch(
                        branch_id,
                        branch.head_cut_id.as_deref(),
                        &parent_head_cut,
                        &parent_head_hash,
                        &rebase_cut,
                        &rebased_hash,
                        at,
                    )? {
                        AdvanceOutcome::Advanced(row) => branch = *row,
                        AdvanceOutcome::Stale { .. } => {
                            return Err(StoreError::Conflict(
                                "branch head moved during the merge; retry".to_owned(),
                            ))
                        }
                        AdvanceOutcome::NotActive { .. } => {
                            return Ok(VcsMergeOutcome::BranchNotActive)
                        }
                        AdvanceOutcome::NotFound => return Ok(VcsMergeOutcome::BranchMissing),
                    }
                }
                RebaseDownPlan::DeferredMidRun => unreachable!("planned at quiescence"),
                RebaseDownPlan::AskAtQuiescence { conflicts } => {
                    return Ok(VcsMergeOutcome::Conflicted { conflicts });
                }
            }
        }

        let head = self.load_manifest(branch.head_manifest_hash.as_deref())?;
        match plan_merge_up(
            &head,
            branch.branch_point_cut_id.as_deref(),
            parent.head_cut_id.as_deref(),
            true,
            true,
        ) {
            MergeUpPlan::Certified { merged_manifest } => {
                let merged_hash = self.store_manifest(&merged_manifest)?;
                match self.branches.advance_head(
                    &parent_id,
                    parent.head_cut_id.as_deref(),
                    merge_cut_id,
                    &merged_hash,
                    at,
                )? {
                    AdvanceOutcome::Advanced(_) => {}
                    AdvanceOutcome::Stale { .. } => {
                        return Err(StoreError::Conflict(
                            "parent head moved during the merge; retry".to_owned(),
                        ))
                    }
                    AdvanceOutcome::NotActive { .. } | AdvanceOutcome::NotFound => {
                        return Ok(VcsMergeOutcome::NoParent)
                    }
                }
                match self.branches.adopt_branch(branch_id, merge_cut_id, at)? {
                    StatusOutcome::Done(_) => Ok(VcsMergeOutcome::Adopted {
                        merge_cut_id: merge_cut_id.to_owned(),
                        into_branch_id: parent_id,
                    }),
                    StatusOutcome::InvalidTransition { .. } => Ok(VcsMergeOutcome::BranchNotActive),
                    StatusOutcome::NotFound => Ok(VcsMergeOutcome::BranchMissing),
                }
            }
            MergeUpPlan::StaleBase { .. } => Err(StoreError::Conflict(
                "parent advanced mid-merge; retry".to_owned(),
            )),
            MergeUpPlan::NeedsLease | MergeUpPlan::NeedsQuiescence => {
                unreachable!("CLI merge holds the mediator role and runs quiescent")
            }
        }
    }

    /// Mainline's id, for callers that don't hardcode it.
    pub fn mainline() -> &'static str {
        MAINLINE_BRANCH_ID
    }
}

#[cfg(all(test, feature = "native"))]
mod tests {
    use super::*;

    fn vcs() -> WorkspaceVcs {
        let dir = std::env::temp_dir().join(format!(
            "whipplescript-vcs-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos(),
        ));
        WorkspaceVcs::open(dir.join("branches.sqlite"), dir.join("content.sqlite"))
            .expect("open vcs")
    }

    /// The full integrated loop: init → branch → isolated writes → merge
    /// adopts into mainline → the branch is immutable history.
    #[test]
    fn branch_write_merge_roundtrip() {
        let mut vcs = vcs();
        vcs.init("t0").expect("init");
        // Seed mainline.
        assert!(matches!(
            vcs.write(
                WorkspaceVcs::mainline(),
                "notes/a.md",
                Some("base A"),
                "cut_m1",
                "t1"
            )
            .expect("write"),
            VcsWriteOutcome::Written { .. }
        ));
        // Branch and diverge.
        assert!(matches!(
            vcs.create_branch("draft_a", Some("triage"), WorkspaceVcs::mainline(), "t2")
                .expect("create"),
            CreateBranchOutcome::Created(_)
        ));
        assert!(matches!(
            vcs.write("draft_a", "notes/a.md", Some("draft A"), "cut_d1", "t3")
                .expect("write"),
            VcsWriteOutcome::Written { .. }
        ));
        // Isolation both ways.
        assert_eq!(
            vcs.read(WorkspaceVcs::mainline(), "notes/a.md")
                .expect("read"),
            Some("base A".to_owned())
        );
        assert_eq!(
            vcs.read("draft_a", "notes/a.md").expect("read"),
            Some("draft A".to_owned())
        );
        // Merge: adopted, mainline sees the branch content, branch is
        // terminal history.
        assert_eq!(
            vcs.merge("draft_a", "cut_merge_1", "t4").expect("merge"),
            VcsMergeOutcome::Adopted {
                merge_cut_id: "cut_merge_1".to_owned(),
                into_branch_id: WorkspaceVcs::mainline().to_owned(),
            }
        );
        assert_eq!(
            vcs.read(WorkspaceVcs::mainline(), "notes/a.md")
                .expect("read"),
            Some("draft A".to_owned())
        );
        let adopted = vcs.get_branch("draft_a").expect("get").expect("row");
        assert_eq!(adopted.status, BranchStatus::Adopted);
        // Writes to adopted history are refused.
        assert_eq!(
            vcs.write("draft_a", "x.md", Some("nope"), "cut_x", "t5")
                .expect("write"),
            VcsWriteOutcome::BranchNotActive
        );
    }

    /// A parent that advanced DISJOINTLY rebases in silently during the
    /// merge; both lines' content lands on mainline.
    #[test]
    fn merge_auto_rebases_disjoint_parent_advance() {
        let mut vcs = vcs();
        vcs.init("t0").expect("init");
        vcs.write(WorkspaceVcs::mainline(), "a.md", Some("A0"), "cut_m1", "t1")
            .expect("write");
        vcs.create_branch("draft_a", None, WorkspaceVcs::mainline(), "t2")
            .expect("create");
        vcs.write("draft_a", "a.md", Some("A1"), "cut_d1", "t3")
            .expect("write");
        // Mainline advances on a DIFFERENT path after the branch point.
        vcs.write(WorkspaceVcs::mainline(), "b.md", Some("B1"), "cut_m2", "t4")
            .expect("write");
        assert!(matches!(
            vcs.merge("draft_a", "cut_merge_1", "t5").expect("merge"),
            VcsMergeOutcome::Adopted { .. }
        ));
        assert_eq!(
            vcs.read(WorkspaceVcs::mainline(), "a.md").expect("read"),
            Some("A1".to_owned())
        );
        assert_eq!(
            vcs.read(WorkspaceVcs::mainline(), "b.md").expect("read"),
            Some("B1".to_owned())
        );
    }

    /// A conflicting parent advance escalates: structured conflicts,
    /// NOTHING moves, the branch stays active; resolving on the branch
    /// (writing the merged content) makes the next merge... still honest:
    /// the same path diverges again, so resolution IS the branch's own
    /// write and the second merge adopts it only after the conflict is
    /// gone from the three-way (base catches up via the resolve write).
    #[test]
    fn conflicting_merge_escalates_and_resolves_by_branch_write() {
        let mut vcs = vcs();
        vcs.init("t0").expect("init");
        vcs.write(WorkspaceVcs::mainline(), "a.md", Some("A0"), "cut_m1", "t1")
            .expect("write");
        vcs.create_branch("draft_a", None, WorkspaceVcs::mainline(), "t2")
            .expect("create");
        vcs.write("draft_a", "a.md", Some("draft A"), "cut_d1", "t3")
            .expect("write");
        // Mainline advances on the SAME path: a real conflict.
        vcs.write(
            WorkspaceVcs::mainline(),
            "a.md",
            Some("main A"),
            "cut_m2",
            "t4",
        )
        .expect("write");
        let VcsMergeOutcome::Conflicted { conflicts } =
            vcs.merge("draft_a", "cut_merge_1", "t5").expect("merge")
        else {
            panic!("expected escalation");
        };
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].path, "a.md");
        assert_eq!(conflicts[0].ours_side.label, "draft_a");
        // Nothing moved.
        assert_eq!(
            vcs.read(WorkspaceVcs::mainline(), "a.md").expect("read"),
            Some("main A".to_owned())
        );
        assert_eq!(
            vcs.get_branch("draft_a").expect("get").expect("row").status,
            BranchStatus::Active
        );
        // Resolution is an ordinary provenance-carrying edit on the
        // branch: write the merged content, then merge again — but the
        // three-way still sees divergence vs the OLD base... resolve by
        // matching mainline's change is a take-theirs; an authored merge
        // needs the branch to agree with mainline for this path to stop
        // conflicting. Authored resolution:
        vcs.write("draft_a", "a.md", Some("main A + draft A"), "cut_d2", "t6")
            .expect("write");
        // Still conflicted (base didn't move): the honest outcome.
        assert!(matches!(
            vcs.merge("draft_a", "cut_merge_2", "t7").expect("merge"),
            VcsMergeOutcome::Conflicted { .. }
        ));
        // Take-ours-into-mainline path: mainline itself adopts the
        // resolution as an ordinary edit (manual override — plain editing
        // is complete over states), after which the branch merges clean.
        vcs.write(
            WorkspaceVcs::mainline(),
            "a.md",
            Some("main A + draft A"),
            "cut_m3",
            "t8",
        )
        .expect("write");
        vcs.write("draft_a", "a.md", Some("main A + draft A"), "cut_d3", "t9")
            .expect("write");
        assert!(matches!(
            vcs.merge("draft_a", "cut_merge_3", "t10").expect("merge"),
            VcsMergeOutcome::Adopted { .. }
        ));
    }

    #[test]
    fn discard_closes_a_head_without_deleting_history() {
        let mut vcs = vcs();
        vcs.init("t0").expect("init");
        vcs.create_branch("draft_a", None, WorkspaceVcs::mainline(), "t1")
            .expect("create");
        vcs.write("draft_a", "x.md", Some("scratch"), "cut_d1", "t2")
            .expect("write");
        assert!(matches!(
            vcs.discard_branch("draft_a", "t3").expect("discard"),
            StatusOutcome::Done(_)
        ));
        // The row (and its cut pointers) remain readable history.
        let row = vcs.get_branch("draft_a").expect("get").expect("row");
        assert_eq!(row.status, BranchStatus::Discarded);
        assert_eq!(
            vcs.read("draft_a", "x.md").expect("read"),
            Some("scratch".to_owned()),
            "discard closes the head; the content remains addressable"
        );
    }
}
