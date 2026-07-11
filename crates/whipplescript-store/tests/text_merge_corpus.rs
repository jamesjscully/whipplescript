//! Text-merge evaluation corpus v1 (spec/text-merge-spec.md §10) — the
//! EXECUTABLE failure-mode registry. In-repo scope (decided 2026-07-11):
//! named-mode fixtures with pinned dispositions + a deterministic
//! metamorphic sweep; corpus mining (Wikipedia revisions, git .md merges)
//! and the agent red-team loop grow this file's fixture set later without
//! changing the harness.
//!
//! The acceptance asymmetry is enforced structurally: a BAD MERGE (clean
//! outcome with wrong bytes, or a clean outcome where the mode must
//! escalate) fails the suite, always. A SPURIOUS CONFLICT is a pinned,
//! visible disposition — `EscalatesByDesign` for behavior we accept with
//! rationale, `EscalatesCandidateFix` for known over-escalation the
//! aligner should eventually win back. Changing any disposition requires
//! editing this registry: that edit IS the review.

use whipplescript_store::text_merge::{
    text_merge, MergePiece, Provenance, TextMergeConfig, TextMergeOutcome,
};

/// What the registry pins for a case.
enum Expect {
    /// Must compose cleanly to exactly these bytes. Anything else fails.
    Composes(&'static str),
    /// Must escalate: composing here would be a bad merge (semantic
    /// overlap the algorithm cannot see the inside of).
    Escalates,
    /// Escalates today and we accept that with rationale (the string).
    /// A clean composition here is NOT a failure of correctness, only a
    /// registry drift signal — the disposition must be re-reviewed, so
    /// it still fails the suite until the pin is updated.
    EscalatesByDesign(&'static str),
    /// Escalates today but is a spurious conflict the aligner should
    /// eventually compose; pinned so improvement shows up as a test
    /// failure demanding a golden update (the ratchet, made visible).
    EscalatesCandidateFix(&'static str),
}

struct Case {
    mode: &'static str,
    name: &'static str,
    base: &'static str,
    ours: &'static str,
    theirs: &'static str,
    expect: Expect,
}

fn registry() -> Vec<Case> {
    vec![
        // -- near-duplicate blocks: the aligner's classic enemy. Two
        //    byte-identical paragraphs, each side editing a different
        //    copy. Unique-token anchoring gets no help (every word
        //    appears twice); prefix/suffix trimming must localize.
        Case {
            mode: "near-duplicate-blocks",
            name: "each side edits a different copy",
            base: "The cache invalidation strategy relies on generation counters to stay correct under load.\n\nThe cache invalidation strategy relies on generation counters to stay correct under load.",
            ours: "The cache invalidation strategy relies on generation counters to stay correct under pressure.\n\nThe cache invalidation strategy relies on generation counters to stay correct under load.",
            theirs: "The cache invalidation strategy relies on generation counters to stay correct under load.\n\nThe cache invalidation strategy relies on generation stamps to stay correct under load.",
            expect: Expect::Composes(
                "The cache invalidation strategy relies on generation counters to stay correct under pressure.\n\nThe cache invalidation strategy relies on generation stamps to stay correct under load.",
            ),
        },
        // -- shared-filler slicing: two rewrites of one clause that share
        //    filler words. Composing would interleave two intents; the
        //    overlap/proximity rules must refuse.
        Case {
            mode: "shared-filler-slicing",
            name: "same clause rewritten two ways",
            base: "The results were, on the whole, quite good for the team.",
            ours: "The results were, in the end, quite bad for the team.",
            theirs: "The results were, on balance, rather good for the team.",
            expect: Expect::Escalates,
        },
        // -- paragraph split: one side breaks a paragraph in two, the
        //    other edits a word a safe distance away.
        Case {
            mode: "paragraph-split",
            name: "split point far from the word edit",
            base: "Alignment anchors carry no semantic meaning at all. They exist only to make matching efficient and the merge itself never depends on where they fall.",
            ours: "Alignment anchors carry no semantic meaning at all.\n\nThey exist only to make matching efficient and the merge itself never depends on where they fall.",
            theirs: "Alignment anchors carry no semantic meaning at all. They exist only to make matching cheap and the merge itself never depends on where they fall.",
            expect: Expect::Composes(
                "Alignment anchors carry no semantic meaning at all.\n\nThey exist only to make matching cheap and the merge itself never depends on where they fall.",
            ),
        },
        // -- move-and-edit, composing shape (corpus finding 2026-07-11):
        //    when the reorder diff expresses as insert+delete AROUND the
        //    block interior, the in-place edit rides through on untouched
        //    base tokens and the insertion-pair exemption (spec §7.2)
        //    lets it compose — to the IDEAL result: the moved block
        //    carries the other side's edit. Move detection isn't needed
        //    for this shape at all.
        Case {
            mode: "move-and-edit",
            name: "reorder vs in-place edit of the moved block interior",
            base: "First paragraph about the tokenizer.\n\nSecond paragraph about the anchors.\n\nThird paragraph about the compose rules.",
            ours: "Second paragraph about the anchors.\n\nFirst paragraph about the tokenizer.\n\nThird paragraph about the compose rules.",
            theirs: "First paragraph about the tokenizer.\n\nSecond paragraph about the content-defined anchors.\n\nThird paragraph about the compose rules.",
            expect: Expect::Composes(
                "Second paragraph about the content-defined anchors.\n\nFirst paragraph about the tokenizer.\n\nThird paragraph about the compose rules.",
            ),
        },
        // -- move-and-edit, boundary shape: the edit touches the moved
        //    block's FIRST word, which sits adjacent to the reorder's
        //    delete — proximity fires and escalation is the honest
        //    outcome (dropping or misplacing the edit would be a bad
        //    merge). This is the shape true move detection (spec §11)
        //    would win back.
        Case {
            mode: "move-and-edit",
            name: "reorder vs edit at the moved block's boundary",
            base: "First paragraph about the tokenizer.\n\nSecond paragraph about the anchors.\n\nThird paragraph about the compose rules.",
            ours: "Second paragraph about the anchors.\n\nFirst paragraph about the tokenizer.\n\nThird paragraph about the compose rules.",
            theirs: "First paragraph about the tokenizer.\n\nRevised paragraph about the anchors.\n\nThird paragraph about the compose rules.",
            expect: Expect::EscalatesByDesign(
                "edit adjacent to the reorder's delete; move detection (deferred) would relocate it — until then escalation never drops it",
            ),
        },
        // -- one giant paragraph: prose with no structure at all; distant
        //    edits must still compose (word atoms carry the granularity,
        //    not newlines).
        Case {
            mode: "one-giant-paragraph",
            name: "distant edits in unbroken prose",
            base: "It was a bright cold day in April and the clocks were striking thirteen while Winston Smith his chin nuzzled into his breast in an effort to escape the vile wind slipped quickly through the glass doors of Victory Mansions though not quickly enough to prevent a swirl of gritty dust from entering along with him and the hallway smelt of boiled cabbage and old rag mats.",
            ours: "It was a bright cold day in March and the clocks were striking thirteen while Winston Smith his chin nuzzled into his breast in an effort to escape the vile wind slipped quickly through the glass doors of Victory Mansions though not quickly enough to prevent a swirl of gritty dust from entering along with him and the hallway smelt of boiled cabbage and old rag mats.",
            theirs: "It was a bright cold day in April and the clocks were striking thirteen while Winston Smith his chin nuzzled into his breast in an effort to escape the vile wind slipped quickly through the glass doors of Victory Mansions though not quickly enough to prevent a swirl of gritty dust from entering along with him and the hallway smelt of boiled cabbage and fresh rag mats.",
            expect: Expect::Composes(
                "It was a bright cold day in March and the clocks were striking thirteen while Winston Smith his chin nuzzled into his breast in an effort to escape the vile wind slipped quickly through the glass doors of Victory Mansions though not quickly enough to prevent a swirl of gritty dust from entering along with him and the hallway smelt of boiled cabbage and fresh rag mats.",
            ),
        },
        // -- punctuation-dense text: code-ish content where Other tokens
        //    dominate; word-gap counting must still separate the edits.
        Case {
            mode: "punctuation-dense",
            name: "assignments edited on different variables",
            base: "x = f(a, b); y = g(c, d); z = h(e, k);",
            ours: "x = f(a, b2); y = g(c, d); z = h(e, k);",
            theirs: "x = f(a, b); y = g(c, d2); z = h(e, k);",
            expect: Expect::Composes("x = f(a, b2); y = g(c, d2); z = h(e, k);"),
        },
        // -- CJK: no whitespace, so an entire clause is ONE word token
        //    (Unicode alphanumeric run). Any two edits to the clause are
        //    a same-token overlap. Coarse but honest; character-tier
        //    atoms for space-free scripts are a candidate refinement.
        Case {
            mode: "cjk-single-run",
            name: "two edits inside one unspaced clause",
            base: "系统在高负载下保持一致性并正确处理缓存失效问题",
            ours: "系统在高压力下保持一致性并正确处理缓存失效问题",
            theirs: "系统在高负载下保持一致性并迅速处理缓存失效问题",
            expect: Expect::EscalatesByDesign(
                "space-free scripts tokenize as one word run; character-tier atoms are a candidate refinement, coarse escalation is honest",
            ),
        },
        // -- whitespace reflow: rewrapping moves newline tokens; an edit
        //    adjacent to a moved wrap point collides with it. Whitespace
        //    is atom-bearing in v1 (identity requires it); reflow-
        //    insensitive comparison is a known candidate dial.
        Case {
            mode: "whitespace-reflow",
            name: "rewrap vs word edit at the wrap point",
            base: "The merge engine composes disjoint\nedits without ceremony and every\nconflict is a structured object.",
            ours: "The merge engine composes disjoint edits\nwithout ceremony and every conflict\nis a structured object.",
            theirs: "The merge engine composes disjoint\nedits without drama and every\nconflict is a structured object.",
            expect: Expect::EscalatesCandidateFix(
                "reflow-only changes are real space-token edits in v1; a class-normalized comparison that still emits verbatim bytes could compose this",
            ),
        },
        // -- convergent edit plus a nearby one-sided edit: the agreed
        //    change is inert for conflict detection, so the extra edit
        //    composes even though it sits near the convergent one.
        Case {
            mode: "convergent-plus-nearby",
            name: "same edit both sides, extra edit one side",
            base: "alpha beta gamma delta epsilon zeta",
            ours: "alpha beta GAMMA delta epsilon ZETA",
            theirs: "alpha beta GAMMA delta epsilon zeta",
            expect: Expect::Composes("alpha beta GAMMA delta epsilon ZETA"),
        },
        // -- delete vs adjacent edit: the deletion's neighborhood is
        //    exactly where interleave risk lives; zero-gap must escalate.
        Case {
            mode: "delete-adjacency",
            name: "deletion touching an opposite-side edit",
            base: "one two three four five",
            ours: "one four five",
            theirs: "one two three FOUR five",
            expect: Expect::Escalates,
        },
    ]
}

fn config() -> TextMergeConfig {
    TextMergeConfig::default()
}

/// Never-fabricate, restated for the corpus: every algorithmic piece is
/// a contiguous substring of the input its provenance names.
fn assert_never_fabricate(outcome: &TextMergeOutcome, base: &str, ours: &str, theirs: &str) {
    let pieces = match outcome {
        TextMergeOutcome::Clean { pieces, .. } => pieces,
        TextMergeOutcome::Conflicted { pieces } => pieces,
    };
    for piece in pieces {
        match piece {
            MergePiece::Merged { text, provenance } => {
                let sources: &[&str] = match provenance {
                    Provenance::Base => &[base],
                    Provenance::Ours => &[ours],
                    Provenance::Theirs => &[theirs],
                    Provenance::Both => &[ours, theirs],
                };
                assert!(
                    sources.iter().all(|source| source.contains(text.as_str())),
                    "fabricated piece {text:?}"
                );
            }
            MergePiece::Conflict {
                base_text,
                ours_text,
                theirs_text,
            } => {
                assert!(base.contains(base_text.as_str()));
                assert!(ours.contains(ours_text.as_str()));
                assert!(theirs.contains(theirs_text.as_str()));
            }
        }
    }
}

/// The registry run: zero bad merges is a hard gate; every disposition
/// is pinned; the scorecard prints per-mode outcomes for review.
#[test]
fn failure_mode_registry() {
    let mut composed = 0usize;
    let mut escalated_semantic = 0usize;
    let mut escalated_by_design = 0usize;
    let mut escalated_candidate_fix = 0usize;
    for case in registry() {
        let outcome = text_merge(case.base, case.ours, case.theirs, &config());
        assert_never_fabricate(&outcome, case.base, case.ours, case.theirs);
        let clean_bytes = match &outcome {
            TextMergeOutcome::Clean { merged, .. } => Some(merged.as_str()),
            TextMergeOutcome::Conflicted { .. } => None,
        };
        match (&case.expect, clean_bytes) {
            (Expect::Composes(expected), Some(actual)) => {
                assert_eq!(
                    actual, *expected,
                    "BAD MERGE (wrong bytes) in {}/{}",
                    case.mode, case.name
                );
                composed += 1;
            }
            (Expect::Composes(_), None) => panic!(
                "spurious conflict in {}/{}: expected clean composition — \
                 if intentional, repin as EscalatesCandidateFix",
                case.mode, case.name
            ),
            (Expect::Escalates, Some(actual)) => panic!(
                "BAD MERGE in {}/{}: must-escalate case composed to {actual:?}",
                case.mode, case.name
            ),
            (Expect::Escalates, None) => escalated_semantic += 1,
            (Expect::EscalatesByDesign(reason), Some(actual)) => panic!(
                "disposition drift in {}/{}: pinned escalates-by-design ({reason}) \
                 but composed to {actual:?}; re-review and repin",
                case.mode, case.name
            ),
            (Expect::EscalatesByDesign(_), None) => escalated_by_design += 1,
            (Expect::EscalatesCandidateFix(reason), Some(actual)) => panic!(
                "disposition drift in {}/{}: pinned candidate-fix ({reason}) \
                 but now composes to {actual:?}; verify bytes and repin as Composes",
                case.mode, case.name
            ),
            (Expect::EscalatesCandidateFix(_), None) => escalated_candidate_fix += 1,
        }
    }
    println!(
        "corpus scorecard: {composed} composed, {escalated_semantic} escalated (semantic), \
         {escalated_by_design} escalated (by design), \
         {escalated_candidate_fix} escalated (candidate fix), 0 bad merges"
    );
}

/// The metamorphic sweep at corpus scale: deterministic generated edit
/// pairs over synthetic prose; every outcome upholds never-fabricate,
/// identity, and clean-side symmetry. No labels needed — property
/// violations here are exactly the silent-bad-merge class.
#[test]
fn metamorphic_sweep_at_scale() {
    const WORDS: &[&str] = &[
        "system",
        "cache",
        "merge",
        "anchor",
        "token",
        "branch",
        "cut",
        "conflict",
        "resolve",
        "compose",
        "escalate",
        "provenance",
        "segment",
        "diff",
        "patience",
        "region",
        "memory",
        "draft",
        "head",
        "base",
        "workspace",
        "honest",
        "certified",
        "atom",
        "word",
        "space",
        "clean",
        "record",
        "fold",
        "review",
    ];
    fn splitmix(seed: u64) -> u64 {
        let mut z = seed.wrapping_add(0x9e3779b97f4a7c15);
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^ (z >> 31)
    }
    let config = config();
    let mut clean = 0usize;
    let mut conflicted = 0usize;
    for seed in 0u64..500 {
        // A 2-4 paragraph document of seeded words.
        let paragraphs = 2 + (splitmix(seed) % 3) as usize;
        let mut base_words: Vec<String> = Vec::new();
        for paragraph in 0..paragraphs {
            let length = 15 + (splitmix(seed * 5 + paragraph as u64) % 25) as usize;
            for index in 0..length {
                let word =
                    WORDS[(splitmix(seed * 11 + (paragraph * 100 + index) as u64) % 30) as usize];
                base_words.push(if index == 0 && paragraph > 0 {
                    format!("\n\n{word}")
                } else {
                    word.to_owned()
                });
            }
        }
        let base = base_words.join(" ");
        // Seeded mutations: replace, delete, or duplicate a word.
        let mutate = |salt: u64| -> String {
            let mut words = base_words.clone();
            let edits = 1 + (splitmix(seed * 13 + salt) % 3) as usize;
            for edit in 0..edits {
                let position =
                    (splitmix(seed * 17 + salt * 7 + edit as u64) % words.len() as u64) as usize;
                match splitmix(seed * 19 + salt * 3 + edit as u64) % 3 {
                    0 => words[position] = format!("edited{salt}{edit}"),
                    1 => {
                        // Delete, but never a paragraph-break carrier.
                        if !words[position].starts_with('\n') && words.len() > 5 {
                            words.remove(position);
                        }
                    }
                    _ => {
                        let duplicate = words[position].clone();
                        words.insert(position, duplicate);
                    }
                }
            }
            words.join(" ")
        };
        let ours = mutate(1);
        let theirs = mutate(2);
        let outcome = text_merge(&base, &ours, &theirs, &config);
        assert_never_fabricate(&outcome, &base, &ours, &theirs);
        match &outcome {
            TextMergeOutcome::Clean { merged, .. } => {
                clean += 1;
                let swapped = text_merge(&base, &theirs, &ours, &config);
                let TextMergeOutcome::Clean {
                    merged: swapped_merged,
                    ..
                } = swapped
                else {
                    panic!("symmetry broken at seed {seed}: swap conflicted");
                };
                assert_eq!(merged, &swapped_merged, "symmetry broken at seed {seed}");
            }
            TextMergeOutcome::Conflicted { .. } => conflicted += 1,
        }
        // Identity laws on every generated document.
        let TextMergeOutcome::Clean { merged, .. } = text_merge(&base, &ours, &base, &config)
        else {
            panic!("identity broken at seed {seed}");
        };
        assert_eq!(merged, ours, "identity bytes broken at seed {seed}");
    }
    println!("metamorphic sweep: 500 seeds, {clean} clean, {conflicted} conflicted, 0 violations");
}
