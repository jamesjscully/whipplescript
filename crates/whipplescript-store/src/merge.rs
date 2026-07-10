//! Merge engine v1, blob plane: path-level three-way merge over file
//! manifests (spec/versioned-workspace-research-note.md §6.1 "Blob files",
//! §7.3 "The conflict surface"; untie-substrate readiness tracker Phase 1).
//!
//! Inputs are the content-addressed manifests the restorable-context build
//! already folds (path → content hash): the branch-point BASE and the two
//! divergent heads. Per path, an unchanged side yields to the changed one;
//! byte-identical outcomes (same hash) merge even when both sides moved;
//! anything else is a REAL conflict, escalated as a structured object
//! carrying base + both sides + both sides' provenance — never `<<<<<<<`
//! markers, never a fake auto-merge, and a delete is an honest outcome
//! (`None`), not an absence. Conflict objects are content-addressed pairs
//! upstream (resolution memory); this module only builds them.
//!
//! Pure and host-agnostic by construction — no store handle, no feature
//! gate — so the DO host and future wasm consumers call it unchanged. The
//! declaration-granularity whip-source merge (slice certificates,
//! merge-slice.maude) is the other half of engine v1 and lands against the
//! slicer seam, not here.

use std::collections::{BTreeMap, BTreeSet};

/// Which branch/head a merge side is, for provenance on conflicts. The
/// label is the branch id (or another human-meaningful head ref); the
/// cut id pins the exact state merged.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MergeSide {
    pub label: String,
    pub cut_id: Option<String>,
}

/// One path's honest conflict: base and both sides as content hashes
/// (`None` = absent/deleted on that timeline), plus both sides'
/// provenance. `base` is `None` for an add/add collision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PathConflict {
    pub path: String,
    pub base: Option<String>,
    pub ours: Option<String>,
    pub theirs: Option<String>,
    pub ours_side: MergeSide,
    pub theirs_side: MergeSide,
}

/// The three-way outcome. `Clean` carries the merged manifest; `Conflicted`
/// carries the merged remainder (every non-conflicting path already folded)
/// plus the structured conflicts — a conflict-bearing cut is a legal,
/// tagged state to build upon, never silently adoptable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MergeOutcome {
    Clean {
        manifest: BTreeMap<String, String>,
    },
    Conflicted {
        merged_remainder: BTreeMap<String, String>,
        conflicts: Vec<PathConflict>,
    },
}

/// Path-level three-way merge over content-addressed manifests.
///
/// Per path with base `b`, ours `o`, theirs `t` (each `Option<hash>`):
/// identical sides (`o == t`) take that outcome — including both-deleted
/// and byte-identical independent writes; an unchanged side (`o == b` or
/// `t == b`) yields to the changed one — including a clean delete; any
/// remaining divergence (modify/modify, modify/delete, add/add with
/// different bytes) escalates.
pub fn merge_manifests(
    base: &BTreeMap<String, String>,
    ours: &BTreeMap<String, String>,
    theirs: &BTreeMap<String, String>,
    ours_side: &MergeSide,
    theirs_side: &MergeSide,
) -> MergeOutcome {
    let mut paths: BTreeSet<&str> = BTreeSet::new();
    paths.extend(base.keys().map(String::as_str));
    paths.extend(ours.keys().map(String::as_str));
    paths.extend(theirs.keys().map(String::as_str));

    let mut merged: BTreeMap<String, String> = BTreeMap::new();
    let mut conflicts: Vec<PathConflict> = Vec::new();
    for path in paths {
        let b = base.get(path);
        let o = ours.get(path);
        let t = theirs.get(path);
        let taken = if o == t {
            o
        } else if o == b {
            t
        } else if t == b {
            o
        } else {
            conflicts.push(PathConflict {
                path: path.to_owned(),
                base: b.cloned(),
                ours: o.cloned(),
                theirs: t.cloned(),
                ours_side: ours_side.clone(),
                theirs_side: theirs_side.clone(),
            });
            continue;
        };
        if let Some(hash) = taken {
            merged.insert(path.to_owned(), hash.clone());
        }
    }
    if conflicts.is_empty() {
        MergeOutcome::Clean { manifest: merged }
    } else {
        MergeOutcome::Conflicted {
            merged_remainder: merged,
            conflicts,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
        entries
            .iter()
            .map(|(path, hash)| ((*path).to_owned(), (*hash).to_owned()))
            .collect()
    }

    fn side(label: &str, cut: &str) -> MergeSide {
        MergeSide {
            label: label.to_owned(),
            cut_id: Some(cut.to_owned()),
        }
    }

    /// Disjoint edits — including two edits landing NEW paths — fold
    /// cleanly and preserve both sides (merge-slice.maude coverage,
    /// restated over blobs).
    #[test]
    fn disjoint_edits_merge_clean_preserving_both() {
        let base = manifest(&[("a.md", "h_a0"), ("b.md", "h_b0")]);
        let ours = manifest(&[("a.md", "h_a1"), ("b.md", "h_b0"), ("new_ours.md", "h_n1")]);
        let theirs = manifest(&[
            ("a.md", "h_a0"),
            ("b.md", "h_b1"),
            ("new_theirs.md", "h_n2"),
        ]);
        let outcome = merge_manifests(
            &base,
            &ours,
            &theirs,
            &side("draft_a", "c1"),
            &side("main", "c2"),
        );
        assert_eq!(
            outcome,
            MergeOutcome::Clean {
                manifest: manifest(&[
                    ("a.md", "h_a1"),
                    ("b.md", "h_b1"),
                    ("new_ours.md", "h_n1"),
                    ("new_theirs.md", "h_n2"),
                ])
            }
        );
    }

    /// Byte-identical independent outcomes merge without conflict — content
    /// addressing recognizes the same change on both sides (transport
    /// reunification's mechanism).
    #[test]
    fn identical_outcomes_are_one_change() {
        let base = manifest(&[("a.md", "h_a0")]);
        let ours = manifest(&[("a.md", "h_a1")]);
        let theirs = manifest(&[("a.md", "h_a1")]);
        let outcome = merge_manifests(
            &base,
            &ours,
            &theirs,
            &side("draft_a", "c1"),
            &side("main", "c2"),
        );
        assert_eq!(
            outcome,
            MergeOutcome::Clean {
                manifest: manifest(&[("a.md", "h_a1")])
            }
        );
    }

    /// A clean one-sided delete wins over an unchanged side; both-deleted
    /// agrees. Deletes are outcomes, not absences.
    #[test]
    fn clean_deletes_fold() {
        let base = manifest(&[
            ("gone_ours.md", "h_1"),
            ("gone_both.md", "h_2"),
            ("kept.md", "h_3"),
        ]);
        let ours = manifest(&[("kept.md", "h_3")]);
        let theirs = manifest(&[("gone_ours.md", "h_1"), ("kept.md", "h_3")]);
        let outcome = merge_manifests(
            &base,
            &ours,
            &theirs,
            &side("draft_a", "c1"),
            &side("main", "c2"),
        );
        assert_eq!(
            outcome,
            MergeOutcome::Clean {
                manifest: manifest(&[("kept.md", "h_3")])
            }
        );
    }

    /// Modify/modify on one path escalates as a structured conflict
    /// carrying base + both sides + provenance, while every other path
    /// still folds into the remainder (conflicts are per-item, never
    /// merge-global).
    #[test]
    fn divergent_writes_escalate_with_provenance() {
        let base = manifest(&[("a.md", "h_a0"), ("b.md", "h_b0")]);
        let ours = manifest(&[("a.md", "h_a1"), ("b.md", "h_b1")]);
        let theirs = manifest(&[("a.md", "h_a2"), ("b.md", "h_b0")]);
        let outcome = merge_manifests(
            &base,
            &ours,
            &theirs,
            &side("draft_a", "c1"),
            &side("main", "c2"),
        );
        assert_eq!(
            outcome,
            MergeOutcome::Conflicted {
                merged_remainder: manifest(&[("b.md", "h_b1")]),
                conflicts: vec![PathConflict {
                    path: "a.md".to_owned(),
                    base: Some("h_a0".to_owned()),
                    ours: Some("h_a1".to_owned()),
                    theirs: Some("h_a2".to_owned()),
                    ours_side: side("draft_a", "c1"),
                    theirs_side: side("main", "c2"),
                }]
            }
        );
    }

    /// Modify/delete is a real conflict — a deletion never silently wins
    /// over concurrent work (never fake auto-merge).
    #[test]
    fn modify_vs_delete_escalates() {
        let base = manifest(&[("a.md", "h_a0")]);
        let ours = manifest(&[]);
        let theirs = manifest(&[("a.md", "h_a1")]);
        let outcome = merge_manifests(
            &base,
            &ours,
            &theirs,
            &side("draft_a", "c1"),
            &side("main", "c2"),
        );
        let MergeOutcome::Conflicted { conflicts, .. } = outcome else {
            panic!("expected escalation");
        };
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].ours, None);
        assert_eq!(conflicts[0].theirs.as_deref(), Some("h_a1"));
    }

    /// Add/add with different bytes escalates with an absent base.
    #[test]
    fn divergent_adds_escalate_with_absent_base() {
        let base = manifest(&[]);
        let ours = manifest(&[("new.md", "h_1")]);
        let theirs = manifest(&[("new.md", "h_2")]);
        let outcome = merge_manifests(
            &base,
            &ours,
            &theirs,
            &side("draft_a", "c1"),
            &side("main", "c2"),
        );
        let MergeOutcome::Conflicted { conflicts, .. } = outcome else {
            panic!("expected escalation");
        };
        assert_eq!(conflicts[0].base, None);
    }
}
