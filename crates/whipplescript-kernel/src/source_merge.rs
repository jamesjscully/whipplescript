//! Declaration-granularity whip-source merge with slice certificates
//! (spec/versioned-workspace-research-note.md §6.1 "Whip source — the real
//! engine"; untie-substrate readiness tracker Phase 1; semantics modeled
//! in merge-slice.maude).
//!
//! Source is a set of declarations, not a text file. The three-way runs
//! at whole-declaration granularity over the branch-point base:
//! both-modified-same-declaration is trivially a conflict; disjoint
//! changed declarations earn a certificate ONLY when the composition
//! theorem's anti-dependence discipline holds — one side's
//! write-or-consume fact footprint must not intersect the other's
//! read-or-write-or-consume footprint, computed from the parser's rule
//! metadata across every version of each changed rule (the blast radius
//! covers before and after). This is what grants same-file merges git
//! could never certify AND refuses cross-declaration interference git
//! silently mis-merges.
//!
//! v1 is fail-closed at every uncertainty, per the tracker: any side that
//! does not compile, a lossy declaration split, a changed NON-rule
//! declaration (classes, stores, headers — no footprint model yet), a
//! changed rule with projection reads (query heads are not yet folded
//! into the read footprint), duplicate identities, or a merged result
//! that does not re-compile — every one escalates to an honest conflict
//! rather than guessing.

use std::collections::{BTreeMap, BTreeSet};

use whipplescript_parser::{compile_program, IrProgram};
use whipplescript_store::vcs::{SourceMergeVerdict, SourceMerger};

/// The parser-backed source merger the hosts install into `WorkspaceVcs`.
pub struct WhipSourceMerger;

/// One top-level declaration block: the identity (its normalized first
/// line) and the exact source text (reassembly is lossless by check).
#[derive(Clone, Debug, Eq, PartialEq)]
struct DeclBlock {
    identity: String,
    text: String,
}

/// The keywords that may open a top-level declaration. A line at brace
/// depth zero starting with one of these begins a new block; everything
/// else (bodies, `=>` lines, comments, blanks) extends the current one.
const DECL_KEYWORDS: &[&str] = &[
    "workflow", "use", "output", "failure", "class", "rule", "signal", "source", "file", "channel",
    "agent", "action", "pattern", "gauge", "campaign",
];

fn starts_declaration(line: &str) -> bool {
    let trimmed = line.trim_start();
    if trimmed.len() != line.len() {
        // Indented lines are always body continuations.
        return false;
    }
    DECL_KEYWORDS.iter().any(|keyword| {
        trimmed
            .strip_prefix(keyword)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\t'))
    })
}

/// Advance the brace depth across a line, skipping string literals.
fn depth_after(line: &str, mut depth: i64) -> i64 {
    let mut in_string = false;
    let mut escaped = false;
    for character in line.chars() {
        if in_string {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
            continue;
        }
        match character {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => depth -= 1,
            _ => {}
        }
    }
    depth
}

/// Split source into declaration blocks. `None` = the split cannot be
/// trusted (duplicate identities, or reassembly is not byte-identical) —
/// the caller fails closed.
fn split_declarations(source: &str) -> Option<Vec<DeclBlock>> {
    let mut blocks: Vec<DeclBlock> = Vec::new();
    let mut current: Option<DeclBlock> = None;
    let mut depth = 0i64;
    for line in source.split_inclusive('\n') {
        let logical = line.strip_suffix('\n').unwrap_or(line);
        if depth == 0 && starts_declaration(logical) {
            if let Some(block) = current.take() {
                blocks.push(block);
            }
            current = Some(DeclBlock {
                identity: logical
                    .trim_end()
                    .trim_end_matches('{')
                    .trim_end()
                    .to_owned(),
                text: String::new(),
            });
        }
        match current.as_mut() {
            Some(block) => block.text.push_str(line),
            None => {
                // Leading prose before the first declaration: attach to a
                // synthetic preamble block keyed by its own text.
                current = Some(DeclBlock {
                    identity: format!("preamble:{}", logical.trim()),
                    text: line.to_owned(),
                });
            }
        }
        depth = depth_after(logical, depth);
    }
    if let Some(block) = current.take() {
        blocks.push(block);
    }
    // Lossless + unambiguous, or nothing.
    let reassembled: String = blocks.iter().map(|block| block.text.as_str()).collect();
    if reassembled != source {
        return None;
    }
    let mut seen = BTreeSet::new();
    for block in &blocks {
        if !seen.insert(block.identity.clone()) {
            return None;
        }
    }
    Some(blocks)
}

/// A changed rule's fact footprint, unioned across every version of the
/// declaration (base and edited) — the whole blast radius.
#[derive(Clone, Debug, Default)]
struct Footprint {
    reads: BTreeSet<String>,
    writes: BTreeSet<String>,
    consumes: BTreeSet<String>,
    /// Projection reads present anywhere in the rule: no certificate (the
    /// query head is not yet folded into the read footprint).
    opaque: bool,
}

impl Footprint {
    fn absorb_rule(&mut self, program: &IrProgram, rule_name: &str) -> bool {
        let Some(rule) = program.rules.iter().find(|rule| rule.name == rule_name) else {
            return false;
        };
        self.reads.extend(rule.metadata.fact_reads.iter().cloned());
        self.writes
            .extend(rule.metadata.fact_writes.iter().cloned());
        self.consumes
            .extend(rule.metadata.fact_consumes.iter().cloned());
        if !rule.metadata.projection_reads.is_empty() {
            self.opaque = true;
        }
        true
    }

    fn write_or_consume(&self) -> BTreeSet<String> {
        self.writes.union(&self.consumes).cloned().collect()
    }

    fn touched_or_read(&self) -> BTreeSet<String> {
        let mut all = self.write_or_consume();
        all.extend(self.reads.iter().cloned());
        all
    }
}

/// The anti-dependence certificate over two changed-rule footprints,
/// exactly merge-slice.maude's guard: neither side's write-or-consume
/// may meet the other's read-or-write-or-consume.
fn slices_disjoint(a: &Footprint, b: &Footprint) -> bool {
    if a.opaque || b.opaque {
        return false;
    }
    a.write_or_consume().is_disjoint(&b.touched_or_read())
        && b.write_or_consume().is_disjoint(&a.touched_or_read())
}

fn compile_ir(source: &str) -> Option<IrProgram> {
    compile_program(source).ir
}

/// Trailing whitespace is block PACKAGING (a following addition donates a
/// separator newline to the previous block), not content — comparisons
/// normalize it away; assembly keeps the raw text.
fn norm(text: &str) -> &str {
    text.trim_end()
}

/// The identities changed between base and a side (modified, added, or
/// deleted), with the per-identity verdict deferred to the caller.
fn changed_identities(
    base: &BTreeMap<String, String>,
    side: &BTreeMap<String, String>,
) -> BTreeSet<String> {
    let mut changed = BTreeSet::new();
    for (identity, text) in side {
        if base.get(identity).map(|base_text| norm(base_text)) != Some(norm(text)) {
            changed.insert(identity.clone());
        }
    }
    for identity in base.keys() {
        if !side.contains_key(identity) {
            changed.insert(identity.clone());
        }
    }
    changed
}

impl SourceMerger for WhipSourceMerger {
    fn merge_source(&self, base: Option<&str>, ours: &str, theirs: &str) -> SourceMergeVerdict {
        let Some(base) = base else {
            // Divergent add/add of a whole source file: no base to compose
            // over.
            return SourceMergeVerdict::Conflict;
        };
        // Fail closed unless every side compiles.
        let (Some(base_ir), Some(ours_ir), Some(theirs_ir)) =
            (compile_ir(base), compile_ir(ours), compile_ir(theirs))
        else {
            return SourceMergeVerdict::Conflict;
        };
        let (Some(base_blocks), Some(ours_blocks), Some(theirs_blocks)) = (
            split_declarations(base),
            split_declarations(ours),
            split_declarations(theirs),
        ) else {
            return SourceMergeVerdict::Conflict;
        };
        let base_map: BTreeMap<String, String> = base_blocks
            .iter()
            .map(|block| (block.identity.clone(), block.text.clone()))
            .collect();
        let ours_map: BTreeMap<String, String> = ours_blocks
            .iter()
            .map(|block| (block.identity.clone(), block.text.clone()))
            .collect();
        let theirs_map: BTreeMap<String, String> = theirs_blocks
            .iter()
            .map(|block| (block.identity.clone(), block.text.clone()))
            .collect();

        let ours_changed = changed_identities(&base_map, &ours_map);
        let theirs_changed = changed_identities(&base_map, &theirs_map);

        // Both-modified-same-declaration: trivially a conflict unless both
        // sides landed the identical text (one change, reunified).
        for identity in ours_changed.intersection(&theirs_changed) {
            if ours_map.get(identity).map(|text| norm(text))
                != theirs_map.get(identity).map(|text| norm(text))
            {
                return SourceMergeVerdict::Conflict;
            }
        }

        // The certificate: every changed declaration must be a rule with a
        // computable footprint, and the two sides' slices must be disjoint
        // under the anti-dependence discipline.
        let footprint_for = |identity: &str,
                             side_ir: &IrProgram,
                             side_map: &BTreeMap<String, String>|
         -> Option<Footprint> {
            let Some(rule_name) = identity.strip_prefix("rule ") else {
                // A non-rule declaration ADDED by this side (absent in
                // base) cannot interfere with edits that compiled without
                // it: empty footprint. Modifying or deleting a non-rule
                // declaration has no footprint model — fail closed.
                return (!base_map.contains_key(identity) && side_map.contains_key(identity))
                    .then(Footprint::default);
            };
            let rule_name = rule_name.trim().to_owned();
            let mut footprint = Footprint::default();
            let mut present_anywhere = false;
            if base_map.contains_key(identity) {
                present_anywhere |= footprint.absorb_rule(&base_ir, &rule_name);
            }
            if side_map.contains_key(identity) {
                present_anywhere |= footprint.absorb_rule(side_ir, &rule_name);
            }
            present_anywhere.then_some(footprint)
        };
        let mut ours_footprints = Vec::new();
        for identity in &ours_changed {
            if theirs_changed.contains(identity) {
                continue; // identical both-sides change, already admitted
            }
            let Some(footprint) = footprint_for(identity, &ours_ir, &ours_map) else {
                return SourceMergeVerdict::Conflict;
            };
            ours_footprints.push(footprint);
        }
        let mut theirs_footprints = Vec::new();
        for identity in &theirs_changed {
            if ours_changed.contains(identity) {
                continue;
            }
            let Some(footprint) = footprint_for(identity, &theirs_ir, &theirs_map) else {
                return SourceMergeVerdict::Conflict;
            };
            theirs_footprints.push(footprint);
        }
        for ours_footprint in &ours_footprints {
            for theirs_footprint in &theirs_footprints {
                if !slices_disjoint(ours_footprint, theirs_footprint) {
                    return SourceMergeVerdict::Conflict;
                }
            }
        }

        // Assemble: base order with per-identity replacement/deletion;
        // additions append (ours' then theirs'), separated as blocks.
        let take = |identity: &str| -> Option<String> {
            if ours_changed.contains(identity) {
                ours_map.get(identity).cloned()
            } else if theirs_changed.contains(identity) {
                theirs_map.get(identity).cloned()
            } else {
                base_map.get(identity).cloned()
            }
        };
        let mut merged = String::new();
        for block in &base_blocks {
            if let Some(text) = take(&block.identity) {
                merged.push_str(&text);
            }
        }
        let mut append_new = |changed: &BTreeSet<String>, map: &BTreeMap<String, String>| {
            for identity in changed {
                if !base_map.contains_key(identity) {
                    if !merged.ends_with("\n\n") {
                        merged.push('\n');
                    }
                    merged.push_str(map.get(identity).expect("added block"));
                }
            }
        };
        append_new(&ours_changed, &ours_map);
        let mut theirs_only = theirs_changed.clone();
        theirs_only.retain(|identity| !ours_changed.contains(identity));
        append_new(&theirs_only, &theirs_map);

        // The merged result must itself compile, or nothing moved.
        if compile_ir(&merged).is_none() {
            return SourceMergeVerdict::Conflict;
        }
        SourceMergeVerdict::Certified { merged }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "workflow Demo\n\noutput result Report\n\nclass Report {\n  message string\n}\n\nclass Ticket {\n  status string\n}\n\nrule triage\n  when started\n=> {\n  record Ticket {\n    status \"open\"\n  }\n}\n\nrule close\n  when Ticket as t\n=> {\n  complete result {\n    message \"done\"\n  }\n}\n";

    /// Disjoint rule edits to the SAME file certify: ours rewrites
    /// `close`'s message, theirs adds an independent logging rule over a
    /// different fact — the merged source carries both and compiles (the
    /// case git can never grant).
    #[test]
    fn disjoint_rule_edits_certify_and_compose() {
        let ours = BASE.replace("message \"done\"", "message \"finished\"");
        let theirs = format!(
            "{BASE}\nclass Audit {{\n  note string\n}}\n\nrule log_start\n  when started\n=> {{\n  record Audit {{\n    note \"started\"\n  }}\n}}\n"
        );
        let SourceMergeVerdict::Certified { merged } =
            WhipSourceMerger.merge_source(Some(BASE), &ours, &theirs)
        else {
            panic!("expected a certificate");
        };
        assert!(merged.contains("message \"finished\""));
        assert!(merged.contains("rule log_start"));
        assert!(compile_program(&merged).ir.is_some());
    }

    /// The cross-declaration semantic conflict (merge-slice.maude's
    /// essential bite, at source granularity): ours changes what `triage`
    /// WRITES; theirs changes `close`, which READS that fact. Different
    /// declarations — a text merge would silently accept — but the
    /// write∩read slice overlap refuses the certificate.
    #[test]
    fn write_read_interference_across_rules_refuses_the_certificate() {
        let ours = BASE.replace("status \"open\"", "status \"reopened\"");
        let theirs = BASE.replace("message \"done\"", "message \"closed out\"");
        // ours changed `triage` (writes Ticket); theirs changed `close`
        // (reads Ticket): write-or-consume ∩ read ⇒ no certificate.
        assert_eq!(
            WhipSourceMerger.merge_source(Some(BASE), &ours, &theirs),
            SourceMergeVerdict::Conflict
        );
    }

    /// Both-modified-same-declaration is trivially a conflict; the
    /// IDENTICAL change on both sides reunifies as one change.
    #[test]
    fn same_declaration_conflicts_unless_identical() {
        let ours = BASE.replace("message \"done\"", "message \"ours\"");
        let theirs = BASE.replace("message \"done\"", "message \"theirs\"");
        assert_eq!(
            WhipSourceMerger.merge_source(Some(BASE), &ours, &theirs),
            SourceMergeVerdict::Conflict
        );
        let same = BASE.replace("message \"done\"", "message \"agreed\"");
        assert!(matches!(
            WhipSourceMerger.merge_source(Some(BASE), &same, &same.clone()),
            SourceMergeVerdict::Certified { .. }
        ));
    }

    /// Fail-closed walls: a non-rule change (class), a non-compiling
    /// side, and a missing base all refuse.
    #[test]
    fn non_rule_changes_and_broken_sides_fail_closed() {
        let ours = BASE.replace("message string", "message string\n  extra int");
        let theirs = format!(
            "{BASE}\nclass Audit {{\n  note string\n}}\n\nrule log_start\n  when started\n=> {{\n  record Audit {{\n    note \"x\"\n  }}\n}}\n"
        );
        assert_eq!(
            WhipSourceMerger.merge_source(Some(BASE), &ours, &theirs),
            SourceMergeVerdict::Conflict,
            "a changed class has no footprint model: fail closed"
        );
        assert_eq!(
            WhipSourceMerger.merge_source(Some(BASE), "not whip at all", BASE),
            SourceMergeVerdict::Conflict
        );
        assert_eq!(
            WhipSourceMerger.merge_source(None, BASE, BASE),
            SourceMergeVerdict::Conflict
        );
    }
}
