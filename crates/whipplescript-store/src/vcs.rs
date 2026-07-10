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

use std::collections::BTreeMap;
use std::path::Path;

#[cfg(feature = "native")]
use crate::branches::BranchStore;
use crate::branches::{
    AdvanceOutcome, BranchRow, BranchStatus, Branches, CreateBranch, CreateBranchOutcome,
    StatusOutcome, MAINLINE_BRANCH_ID,
};
use crate::content::ContentBlobs;
#[cfg(feature = "native")]
use crate::content::ContentStore;
use crate::files::FileStore;
use crate::merge::MergeSide;
use crate::merge::PathConflict;
use crate::reconcile::{plan_merge_up, plan_rebase_down, MergeUpPlan, RebaseDownPlan};
use crate::working_set::VirtualWorkingSet;
use crate::{StoreError, StoreResult};

/// One VCS write/remove outcome: the new cut, or the refusal.
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

/// Verdict of a source-aware merge attempt on one conflicted path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SourceMergeVerdict {
    /// The edits' slices are provably disjoint: this is the certified
    /// merged source (the composition theorem's crossover).
    Certified { merged: String },
    /// No certificate — stays an honest conflict.
    Conflict,
}

/// The declaration-granularity source-merge seam (vw note §6.1): the
/// store crate owns WHERE source-aware refinement applies (conflicted
/// `.whip` paths); the host installs HOW (the kernel's parser-backed
/// merger). Absent merger = every source conflict escalates (fail
/// closed).
pub trait SourceMerger {
    fn merge_source(&self, base: Option<&str>, ours: &str, theirs: &str) -> SourceMergeVerdict;
}

/// One reconciliation-daemon tick over one branch (the executor of
/// reconcile.rs's plans; lifecycle in ReconciliationDaemonLifecycle.tla).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReconcileOutcome {
    UpToDate,
    /// The parent's delta folded in (silently when blob-disjoint; via
    /// certified source merges at quiescence).
    Rebased {
        rebase_cut_id: String,
    },
    /// Mid-run and the delta intersects: nothing moves until quiescence.
    DeferredMidRun,
    /// At quiescence the residual conflicts are the notification-and-ask.
    Conflicts {
        conflicts: Vec<PathConflict>,
    },
    BranchMissing,
    BranchNotActive,
    NoParent,
}

pub struct WorkspaceVcs<B: Branches, C: ContentBlobs> {
    branches: B,
    content: C,
    source_merger: Option<Box<dyn SourceMerger>>,
}

/// The native workspace VCS: rusqlite-backed branch + content stores.
#[cfg(feature = "native")]
pub type NativeWorkspaceVcs = WorkspaceVcs<BranchStore, ContentStore>;

#[cfg(feature = "native")]
impl NativeWorkspaceVcs {
    pub fn open(
        branches_path: impl AsRef<Path>,
        content_path: impl AsRef<Path>,
    ) -> StoreResult<Self> {
        Ok(Self {
            branches: BranchStore::open(branches_path)?,
            content: ContentStore::open(content_path)?,
            source_merger: None,
        })
    }
}

impl<B: Branches, C: ContentBlobs> WorkspaceVcs<B, C> {
    /// Compose a workspace VCS from any host's branch + content seams
    /// (the DO host passes its `DoSql`-backed implementations).
    pub fn from_parts(branches: B, content: C) -> Self {
        Self {
            branches,
            content,
            source_merger: None,
        }
    }

    /// Install the host's declaration-granularity source merger (the
    /// kernel's parser-backed implementation). Fail-closed default: no
    /// merger, no source-aware refinement.
    pub fn set_source_merger(&mut self, merger: Box<dyn SourceMerger>) {
        self.source_merger = Some(merger);
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

    /// Source-aware conflict refinement: for each conflicted `.whip` path
    /// with both sides present, ask the installed merger for a certified
    /// declaration-granularity merge; certified content is stored and
    /// folded, everything else stays an honest conflict (fail closed:
    /// no merger, delete-vs-modify, or non-source paths never refine).
    fn refine_source_conflicts(
        &self,
        conflicts: Vec<PathConflict>,
    ) -> StoreResult<(BTreeMap<String, String>, Vec<PathConflict>)> {
        let Some(merger) = self.source_merger.as_deref() else {
            return Ok((BTreeMap::new(), conflicts));
        };
        let mut resolved = BTreeMap::new();
        let mut remaining = Vec::new();
        for conflict in conflicts {
            let refinable = conflict.path.ends_with(".whip")
                && conflict.ours.is_some()
                && conflict.theirs.is_some();
            if !refinable {
                remaining.push(conflict);
                continue;
            }
            let base_body = match conflict.base.as_deref() {
                Some(hash) => self.content.get(hash)?,
                None => None,
            };
            let (Some(ours_body), Some(theirs_body)) = (
                self.content
                    .get(conflict.ours.as_deref().expect("present"))?,
                self.content
                    .get(conflict.theirs.as_deref().expect("present"))?,
            ) else {
                remaining.push(conflict);
                continue;
            };
            match merger.merge_source(base_body.as_deref(), &ours_body, &theirs_body) {
                SourceMergeVerdict::Certified { merged } => {
                    let hash = self.content.put(&merged)?;
                    resolved.insert(conflict.path.clone(), hash);
                }
                SourceMergeVerdict::Conflict => remaining.push(conflict),
            }
        }
        Ok((resolved, remaining))
    }

    /// One reconciliation-daemon tick for one branch: fold the parent's
    /// delta down. Blob-disjoint deltas fold in ANY phase (silent
    /// continuous rebase); everything that touches contested paths waits
    /// for quiescence, where certified source merges refine and the
    /// residue escalates as the ask. Executes reconcile.rs's plans
    /// against the branch store; lifecycle per
    /// ReconciliationDaemonLifecycle.tla.
    pub fn reconcile_branch(
        &mut self,
        branch_id: &str,
        quiescent: bool,
        rebase_cut_id: &str,
        at: &str,
    ) -> StoreResult<ReconcileOutcome> {
        let Some(branch) = self.branches.get_branch(branch_id)? else {
            return Ok(ReconcileOutcome::BranchMissing);
        };
        if branch.status != BranchStatus::Active {
            return Ok(ReconcileOutcome::BranchNotActive);
        }
        let Some(parent_id) = branch.parent_branch_id.clone() else {
            return Ok(ReconcileOutcome::NoParent);
        };
        let Some(parent) = self.branches.get_branch(&parent_id)? else {
            return Ok(ReconcileOutcome::NoParent);
        };
        if branch.branch_point_cut_id == parent.head_cut_id {
            return Ok(ReconcileOutcome::UpToDate);
        }
        let branch_side = MergeSide {
            label: branch_id.to_owned(),
            cut_id: branch.head_cut_id.clone(),
        };
        let parent_side = MergeSide {
            label: parent_id.clone(),
            cut_id: parent.head_cut_id.clone(),
        };
        let point = self.load_manifest(branch.branch_point_manifest_hash.as_deref())?;
        let head = self.load_manifest(branch.head_manifest_hash.as_deref())?;
        let target = self.load_manifest(parent.head_manifest_hash.as_deref())?;
        let new_head = match plan_rebase_down(
            &point,
            &head,
            &target,
            &branch_side,
            &parent_side,
            quiescent,
        ) {
            RebaseDownPlan::UpToDate => return Ok(ReconcileOutcome::UpToDate),
            RebaseDownPlan::Silent { new_head_manifest } => new_head_manifest,
            RebaseDownPlan::DeferredMidRun => return Ok(ReconcileOutcome::DeferredMidRun),
            RebaseDownPlan::AskAtQuiescence { conflicts } => {
                // At quiescence, certified source merges refine the
                // contested paths; any residue is the honest ask. The
                // refined manifest = clean remainder + certified content,
                // recomputed here from the same three-way.
                let crate::merge::MergeOutcome::Conflicted {
                    merged_remainder,
                    conflicts: raw,
                } = crate::merge::merge_manifests(
                    &point,
                    &head,
                    &target,
                    &branch_side,
                    &parent_side,
                )
                else {
                    unreachable!("plan reported conflicts");
                };
                debug_assert_eq!(raw.len(), conflicts.len());
                let (resolved, remaining) = self.refine_source_conflicts(raw)?;
                if !remaining.is_empty() {
                    return Ok(ReconcileOutcome::Conflicts {
                        conflicts: remaining,
                    });
                }
                let mut manifest = merged_remainder;
                manifest.extend(resolved);
                manifest
            }
        };
        let rebased_hash = self.store_manifest(&new_head)?;
        let parent_head_cut = parent.head_cut_id.clone().unwrap_or_default();
        let parent_head_hash = parent
            .head_manifest_hash
            .clone()
            .unwrap_or_else(|| self.store_manifest(&BTreeMap::new()).unwrap_or_default());
        match self.branches.rebase_branch(
            branch_id,
            branch.head_cut_id.as_deref(),
            &parent_head_cut,
            &parent_head_hash,
            rebase_cut_id,
            &rebased_hash,
            at,
        )? {
            AdvanceOutcome::Advanced(_) => Ok(ReconcileOutcome::Rebased {
                rebase_cut_id: rebase_cut_id.to_owned(),
            }),
            AdvanceOutcome::Stale { .. } => Err(StoreError::Conflict(
                "branch head moved during the rebase; retry".to_owned(),
            )),
            AdvanceOutcome::NotActive { .. } => Ok(ReconcileOutcome::BranchNotActive),
            AdvanceOutcome::NotFound => Ok(ReconcileOutcome::BranchMissing),
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
        // Rebase-down when the parent moved past our branch point. The
        // CLI merge runs at a quiescence point by definition (no run in
        // flight inside this verb), so a conflicting delta escalates as
        // the ask instead of deferring; source-certified refinement
        // applies (quiescent).
        match self.reconcile_branch(branch_id, true, &format!("{merge_cut_id}-rebase"), at)? {
            ReconcileOutcome::UpToDate => {}
            ReconcileOutcome::Rebased { .. } => {
                branch = self
                    .branches
                    .get_branch(branch_id)?
                    .ok_or_else(|| StoreError::Conflict("branch vanished mid-merge".to_owned()))?;
            }
            ReconcileOutcome::DeferredMidRun => unreachable!("reconciled at quiescence"),
            ReconcileOutcome::Conflicts { conflicts } => {
                return Ok(VcsMergeOutcome::Conflicted { conflicts });
            }
            ReconcileOutcome::BranchMissing => return Ok(VcsMergeOutcome::BranchMissing),
            ReconcileOutcome::BranchNotActive => return Ok(VcsMergeOutcome::BranchNotActive),
            ReconcileOutcome::NoParent => return Ok(VcsMergeOutcome::NoParent),
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

    /// Commit an imported scratch diff (materialize.rs) as ONE cut on the
    /// branch: atomic (a single head advance carries the whole diff),
    /// recorded, complete (the caller stored every blob first), keyed by
    /// the effect-derived `cut_id`, and IDEMPOTENT — a crash-retry that
    /// finds the head already at this cut is a no-op success, so
    /// re-driving the effect never double-applies.
    pub fn import_diff(
        &mut self,
        branch_id: &str,
        changed: &BTreeMap<String, String>,
        removed: &[String],
        cut_id: &str,
        at: &str,
    ) -> StoreResult<VcsWriteOutcome> {
        let Some(row) = self.branches.get_branch(branch_id)? else {
            return Ok(VcsWriteOutcome::BranchMissing);
        };
        if row.head_cut_id.as_deref() == Some(cut_id) {
            // The idempotent retry: this effect's import already landed.
            return Ok(VcsWriteOutcome::Written {
                cut_id: cut_id.to_owned(),
                manifest_hash: row.head_manifest_hash.unwrap_or_default(),
            });
        }
        if row.status != BranchStatus::Active {
            return Ok(VcsWriteOutcome::BranchNotActive);
        }
        let mut manifest = self.load_manifest(row.head_manifest_hash.as_deref())?;
        for (path, hash) in changed {
            manifest.insert(path.clone(), hash.clone());
        }
        for path in removed {
            manifest.remove(path);
        }
        let manifest_hash = self.store_manifest(&manifest)?;
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
            AdvanceOutcome::Stale {
                current_head_cut_id,
            } => {
                if current_head_cut_id.as_deref() == Some(cut_id) {
                    // Raced with our own retry: the import landed.
                    return Ok(VcsWriteOutcome::Written {
                        cut_id: cut_id.to_owned(),
                        manifest_hash,
                    });
                }
                Err(StoreError::Conflict(
                    "branch head moved during the import; retry".to_owned(),
                ))
            }
            AdvanceOutcome::NotActive { .. } => Ok(VcsWriteOutcome::BranchNotActive),
            AdvanceOutcome::NotFound => Ok(VcsWriteOutcome::BranchMissing),
        }
    }

    /// Materialize-on-exec: project the branch's head manifest into a
    /// real scratch directory (materialize.rs). `None` = no such branch.
    #[cfg(feature = "native")]
    pub fn materialize_branch(
        &self,
        branch_id: &str,
        root: &Path,
        now_unix_nanos: i128,
    ) -> StoreResult<Option<crate::materialize::MaterializedScratch>> {
        let Some(manifest) = self.manifest(branch_id)? else {
            return Ok(None);
        };
        crate::materialize::materialize_manifest(&manifest, &self.content, root, now_unix_nanos)
            .map(Some)
    }

    /// Import-back: scan the scratch against its seeded cache, store every
    /// changed blob, and commit the whole diff as ONE effect-keyed,
    /// idempotent cut on the branch.
    #[cfg(feature = "native")]
    #[allow(clippy::too_many_arguments)]
    pub fn import_scratch(
        &mut self,
        branch_id: &str,
        root: &Path,
        scratch: &crate::materialize::MaterializedScratch,
        cut_id: &str,
        at: &str,
        now_unix_nanos: i128,
    ) -> StoreResult<(crate::materialize::ScratchImport, VcsWriteOutcome)> {
        let import =
            crate::materialize::import_scratch(root, scratch, &self.content, now_unix_nanos)?;
        let outcome = self.import_diff(branch_id, &import.changed, &import.removed, cut_id, at)?;
        Ok((import, outcome))
    }

    /// Bind an instance to the branch it is born on (write-once; see
    /// `Branches::bind_instance`).
    pub fn bind_instance(
        &mut self,
        instance_id: &str,
        branch_id: &str,
        at: &str,
    ) -> StoreResult<crate::branches::BindOutcome> {
        self.branches.ensure_mainline(at)?;
        self.branches.bind_instance(instance_id, branch_id, at)
    }

    pub fn instance_branch(&self, instance_id: &str) -> StoreResult<Option<String>> {
        self.branches.instance_branch(instance_id)
    }

    pub fn list_bound_instances_of(&self, branch_id: &str) -> StoreResult<Vec<String>> {
        self.branches.list_bound_instances(branch_id)
    }

    /// Mainline's id, for callers that don't hardcode it.
    pub fn mainline() -> &'static str {
        MAINLINE_BRANCH_ID
    }
}

/// A branch-bound instance's file surface: the `FileStore` the effect
/// handlers dispatch onto when the instance was born on a branch. Every
/// mutation write-throughs a cut on the branch (COW — nothing outside the
/// branch changes); reads resolve the branch's current head. Cut ids
/// derive from the effect id (`<effect>-f<n>`), so one effect's file
/// operations are attributable cuts. Interior mutability because the
/// `FileStore` seam takes `&self`.
pub struct BranchFileStore<B: Branches, C: ContentBlobs> {
    vcs: std::cell::RefCell<WorkspaceVcs<B, C>>,
    branch_id: String,
    cut_seed: String,
    at: String,
    counter: std::cell::Cell<u64>,
}

impl<B: Branches, C: ContentBlobs> BranchFileStore<B, C> {
    pub fn new(vcs: WorkspaceVcs<B, C>, branch_id: &str, cut_seed: &str, at: &str) -> Self {
        Self {
            vcs: std::cell::RefCell::new(vcs),
            branch_id: branch_id.to_owned(),
            cut_seed: cut_seed.to_owned(),
            at: at.to_owned(),
            counter: std::cell::Cell::new(0),
        }
    }

    fn next_cut_id(&self) -> String {
        let index = self.counter.get();
        self.counter.set(index + 1);
        format!("{}-f{index}", self.cut_seed)
    }

    fn io_error(error: StoreError) -> std::io::Error {
        std::io::Error::other(format!("branch file store: {error:?}"))
    }

    fn apply(&self, path: &Path, body: Option<&str>) -> std::io::Result<()> {
        let cut_id = self.next_cut_id();
        let path_key = path.to_string_lossy();
        match self
            .vcs
            .borrow_mut()
            .write(&self.branch_id, &path_key, body, &cut_id, &self.at)
            .map_err(Self::io_error)?
        {
            VcsWriteOutcome::Written { .. } => Ok(()),
            VcsWriteOutcome::BranchMissing => Err(std::io::Error::other(format!(
                "instance is bound to unknown branch `{}`",
                self.branch_id
            ))),
            VcsWriteOutcome::BranchNotActive => Err(std::io::Error::other(format!(
                "instance is bound to closed branch `{}`",
                self.branch_id
            ))),
        }
    }
}

impl<B: Branches, C: ContentBlobs> FileStore for BranchFileStore<B, C> {
    fn read_to_string(&self, path: &Path) -> std::io::Result<String> {
        let path_key = path.to_string_lossy();
        match self
            .vcs
            .borrow()
            .read(&self.branch_id, &path_key)
            .map_err(Self::io_error)?
        {
            Some(body) => Ok(body),
            None => Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("no file at {path_key} on branch `{}`", self.branch_id),
            )),
        }
    }

    fn exists(&self, path: &Path) -> bool {
        let path_key = path.to_string_lossy();
        self.vcs
            .borrow()
            .read(&self.branch_id, &path_key)
            .map(|body| body.is_some())
            .unwrap_or(false)
    }

    fn create_dir_all(&self, _path: &Path) -> std::io::Result<()> {
        Ok(())
    }

    fn write(&self, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
        let body = std::str::from_utf8(bytes).map_err(|error| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("non-UTF-8 write to {}: {error}", path.display()),
            )
        })?;
        self.apply(path, Some(body))
    }

    fn append(&self, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
        let existing = match self.read_to_string(path) {
            Ok(body) => body,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => return Err(error),
        };
        let suffix = std::str::from_utf8(bytes).map_err(|error| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("non-UTF-8 append to {}: {error}", path.display()),
            )
        })?;
        let mut body = existing;
        body.push_str(suffix);
        self.apply(path, Some(&body))
    }

    fn remove(&self, path: &Path) -> std::io::Result<()> {
        self.apply(path, None)
    }
}

#[cfg(all(test, feature = "native"))]
mod tests {
    use super::*;

    fn vcs() -> NativeWorkspaceVcs {
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
                MAINLINE_BRANCH_ID,
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
            vcs.create_branch("draft_a", Some("triage"), MAINLINE_BRANCH_ID, "t2")
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
            vcs.read(MAINLINE_BRANCH_ID, "notes/a.md").expect("read"),
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
                into_branch_id: MAINLINE_BRANCH_ID.to_owned(),
            }
        );
        assert_eq!(
            vcs.read(MAINLINE_BRANCH_ID, "notes/a.md").expect("read"),
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
        vcs.write(MAINLINE_BRANCH_ID, "a.md", Some("A0"), "cut_m1", "t1")
            .expect("write");
        vcs.create_branch("draft_a", None, MAINLINE_BRANCH_ID, "t2")
            .expect("create");
        vcs.write("draft_a", "a.md", Some("A1"), "cut_d1", "t3")
            .expect("write");
        // Mainline advances on a DIFFERENT path after the branch point.
        vcs.write(MAINLINE_BRANCH_ID, "b.md", Some("B1"), "cut_m2", "t4")
            .expect("write");
        assert!(matches!(
            vcs.merge("draft_a", "cut_merge_1", "t5").expect("merge"),
            VcsMergeOutcome::Adopted { .. }
        ));
        assert_eq!(
            vcs.read(MAINLINE_BRANCH_ID, "a.md").expect("read"),
            Some("A1".to_owned())
        );
        assert_eq!(
            vcs.read(MAINLINE_BRANCH_ID, "b.md").expect("read"),
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
        vcs.write(MAINLINE_BRANCH_ID, "a.md", Some("A0"), "cut_m1", "t1")
            .expect("write");
        vcs.create_branch("draft_a", None, MAINLINE_BRANCH_ID, "t2")
            .expect("create");
        vcs.write("draft_a", "a.md", Some("draft A"), "cut_d1", "t3")
            .expect("write");
        // Mainline advances on the SAME path: a real conflict.
        vcs.write(MAINLINE_BRANCH_ID, "a.md", Some("main A"), "cut_m2", "t4")
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
            vcs.read(MAINLINE_BRANCH_ID, "a.md").expect("read"),
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
            MAINLINE_BRANCH_ID,
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

    /// import_diff commits a whole scratch diff as ONE effect-keyed cut,
    /// and the crash-retry (same cut id) is a no-op success — never a
    /// double-apply, never a spurious conflict.
    #[test]
    fn import_diff_is_atomic_and_idempotent() {
        let mut vcs = vcs();
        vcs.init("t0").expect("init");
        vcs.write(MAINLINE_BRANCH_ID, "a.md", Some("A0"), "cut_m1", "t1")
            .expect("write");
        vcs.create_branch("draft_a", None, MAINLINE_BRANCH_ID, "t2")
            .expect("create");
        let mut changed = BTreeMap::new();
        changed.insert(
            "out.md".to_owned(),
            vcs.content.put("produced").expect("put"),
        );
        changed.insert(
            "a.md".to_owned(),
            vcs.content.put("A modified").expect("put"),
        );
        let removed = Vec::new();
        let first = vcs
            .import_diff("draft_a", &changed, &removed, "cut_effect_1", "t3")
            .expect("import");
        let VcsWriteOutcome::Written { manifest_hash, .. } = first else {
            panic!("expected the import to land");
        };
        assert_eq!(
            vcs.read("draft_a", "out.md").expect("read").as_deref(),
            Some("produced")
        );
        assert_eq!(
            vcs.read("draft_a", "a.md").expect("read").as_deref(),
            Some("A modified"),
            "the whole diff landed in one cut"
        );
        // The idempotent retry: same effect-keyed cut id, no double-apply.
        let retry = vcs
            .import_diff("draft_a", &changed, &removed, "cut_effect_1", "t4")
            .expect("retry");
        assert_eq!(
            retry,
            VcsWriteOutcome::Written {
                cut_id: "cut_effect_1".to_owned(),
                manifest_hash,
            }
        );
        // Removals fold in the same atomic step.
        let mut second_changed = BTreeMap::new();
        second_changed.insert("b.md".to_owned(), vcs.content.put("B new").expect("put"));
        let outcome = vcs
            .import_diff(
                "draft_a",
                &second_changed,
                &["out.md".to_owned()],
                "cut_effect_2",
                "t5",
            )
            .expect("second import");
        assert!(matches!(outcome, VcsWriteOutcome::Written { .. }));
        assert_eq!(vcs.read("draft_a", "out.md").expect("read"), None);
        assert_eq!(
            vcs.read("draft_a", "b.md").expect("read").as_deref(),
            Some("B new")
        );
    }

    #[test]
    fn discard_closes_a_head_without_deleting_history() {
        let mut vcs = vcs();
        vcs.init("t0").expect("init");
        vcs.create_branch("draft_a", None, MAINLINE_BRANCH_ID, "t1")
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
