//! Token-level three-way text merge (spec/text-merge-spec.md; certified
//! compose layer modeled in models/maude/text-merge-compose.maude).
//!
//! The blob plane's path-level merge (`merge.rs`) escalates every
//! both-modified path. This tier refines eligible modify/modify conflicts:
//! the atom is the WORD (with whitespace and punctuation as atoms too, so
//! the token stream tiles the text exactly), and alignment anchors are
//! content-defined segments over token hashes — chosen for algorithmic
//! efficiency alone, carrying no semantic meaning. Unlike `chunking.rs`,
//! whose boundaries mint frozen storage identity, merge anchors are
//! EPHEMERAL: they exist only inside one merge computation and may change
//! freely between versions.
//!
//! The architecture splits certified from heuristic (the slicer-client
//! split restated): tokenization + anchoring + diff produce per-side edit
//! scripts over base coordinates, and ANY pair of scripts composes under
//! the certified rules — never-fabricate (every output byte is a
//! contiguous slice of exactly one input) and both-touched-escalates
//! (overlapping or proximate cross-side edits become structured conflict
//! regions carrying base + both sides, never a picked winner). A worse
//! aligner can only inflate the conflict count, never break correctness.
//!
//! Pure and host-agnostic — no store handle, no feature gate. Whip-legible
//! sources never come here (declaration-granularity merge owns them; the
//! merge-slice bite is untouched).

use std::collections::HashMap;

/// Provenance tag recorded on the merge outcome payload, versioning the
/// algorithm across upgrades (anchors are ephemeral; recorded merges keep
/// their meaning through this tag, not through frozen parameters).
pub const TEXT_MERGE_ALGORITHM: &str = "text-merge/1";

/// Tuning knobs. `proximity_gap` is the dial from spec §7.3: cross-side
/// edits separated by fewer than this many WORD tokens escalate instead of
/// composing — the guard against interleaving two rewrites of one sentence
/// that anchor on shared filler words. Conservative by default
/// (precision-first); loosening requires corpus evidence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextMergeConfig {
    /// Minimum WORD tokens between cross-side edits (spec §7.3).
    pub proximity_gap: usize,
    /// Per-input byte ceiling; larger bodies stay at path-level escalation.
    pub max_bytes: usize,
    /// Anchor segment bounds, in tokens (spec §5). `segment_avg_bits` sets
    /// the content-defined boundary mask (average segment = 2^bits tokens).
    pub segment_min: usize,
    pub segment_avg_bits: u32,
    pub segment_max: usize,
}

impl Default for TextMergeConfig {
    fn default() -> Self {
        Self {
            proximity_gap: 2,
            max_bytes: 8 * 1024 * 1024,
            segment_min: 16,
            segment_avg_bits: 6,
            segment_max: 512,
        }
    }
}

impl TextMergeConfig {
    /// Defaults with the two operational dials read from the environment
    /// (`WHIPPLESCRIPT_TEXT_MERGE_GAP`, `WHIPPLESCRIPT_TEXT_MERGE_MAX_BYTES`);
    /// malformed values fall back rather than disabling the guard.
    pub fn from_env() -> Self {
        let mut config = Self::default();
        if let Some(gap) = env_usize("WHIPPLESCRIPT_TEXT_MERGE_GAP") {
            config.proximity_gap = gap;
        }
        if let Some(max) = env_usize("WHIPPLESCRIPT_TEXT_MERGE_MAX_BYTES") {
            if max > 0 {
                config.max_bytes = max;
            }
        }
        config
    }

    /// Size eligibility (spec §3); UTF-8 validity is a type-level given.
    pub fn within_size(&self, base: &str, ours: &str, theirs: &str) -> bool {
        base.len() <= self.max_bytes
            && ours.len() <= self.max_bytes
            && theirs.len() <= self.max_bytes
    }
}

fn env_usize(name: &str) -> Option<usize> {
    std::env::var(name).ok()?.trim().parse::<usize>().ok()
}

/// Which input a merged piece's bytes were selected from.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provenance {
    Base,
    Ours,
    Theirs,
    /// Convergent: both sides made the byte-identical change (taken once).
    Both,
}

/// One span of the merged document, in base order. `Conflict` is the
/// token-level twin of `PathConflict`: base + both sides, never `<<<<<<<`
/// markers, never a picked winner. The piece list IS the editor-fold /
/// merge-preview payload — provenance makes even merged spans reviewable.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MergePiece {
    Merged {
        text: String,
        provenance: Provenance,
    },
    Conflict {
        base_text: String,
        ours_text: String,
        theirs_text: String,
    },
}

/// The three-way outcome. `Conflicted` is a legal, tagged state to build
/// on (conflicts don't block); `Clean.merged` is the piece concatenation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TextMergeOutcome {
    Clean {
        pieces: Vec<MergePiece>,
        merged: String,
    },
    Conflicted {
        pieces: Vec<MergePiece>,
    },
}

impl TextMergeOutcome {
    pub fn pieces(&self) -> &[MergePiece] {
        match self {
            TextMergeOutcome::Clean { pieces, .. } => pieces,
            TextMergeOutcome::Conflicted { pieces } => pieces,
        }
    }
}

// ---------------------------------------------------------------------------
// Tokenization (spec §4): deterministic, whitespace-preserving, Unicode-safe.
// Tokens tile the text — concatenating them reproduces the input exactly.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TokenClass {
    Word,
    Space,
    Other,
}

#[derive(Clone, Copy, Debug)]
struct Token {
    start: usize,
    end: usize,
    class: TokenClass,
}

fn class_of(ch: char) -> TokenClass {
    if ch.is_alphanumeric() || ch == '_' {
        TokenClass::Word
    } else if ch.is_whitespace() {
        TokenClass::Space
    } else {
        TokenClass::Other
    }
}

fn tokenize(text: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut chars = text.char_indices().peekable();
    while let Some((start, ch)) = chars.next() {
        let class = class_of(ch);
        let mut end = start + ch.len_utf8();
        // Word and Space are maximal runs; Other is one char per token so
        // punctuation never glues into opaque multi-symbol atoms.
        if class != TokenClass::Other {
            while let Some(&(next_start, next_ch)) = chars.peek() {
                if class_of(next_ch) != class {
                    break;
                }
                end = next_start + next_ch.len_utf8();
                chars.next();
            }
        }
        tokens.push(Token { start, end, class });
    }
    tokens
}

fn token_text<'a>(text: &'a str, tokens: &[Token], range: (usize, usize)) -> &'a str {
    if range.0 >= range.1 {
        return "";
    }
    &text[tokens[range.0].start..tokens[range.1 - 1].end]
}

// ---------------------------------------------------------------------------
// Anchor segmentation (spec §5): content-defined boundaries over token
// hashes. Efficiency-only and ephemeral — never identity-bearing.
// ---------------------------------------------------------------------------

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// Segment token index boundaries: cut after a token whose hash satisfies
/// the mask, subject to min/max lengths. Returns segment (start, end) token
/// ranges tiling `tokens`.
fn segment_ranges(text: &str, tokens: &[Token], config: &TextMergeConfig) -> Vec<(usize, usize)> {
    let mask = (1u64 << config.segment_avg_bits.min(63)) - 1;
    let mut ranges = Vec::new();
    let mut start = 0usize;
    for (index, token) in tokens.iter().enumerate() {
        let length = index + 1 - start;
        let hash = fnv1a(&text.as_bytes()[token.start..token.end]);
        if (length >= config.segment_min && hash & mask == 0) || length >= config.segment_max {
            ranges.push((start, index + 1));
            start = index + 1;
        }
    }
    if start < tokens.len() {
        ranges.push((start, tokens.len()));
    }
    ranges
}

// ---------------------------------------------------------------------------
// Diff (spec §6): patience diff over interned ids — deterministic,
// O(n log n), degrades to a coarse whole-range replace when no unique
// anchors exist (coarseness only inflates conflicts; the certified core
// doesn't care). Two tiers: segment hashes first, tokens inside changed
// segment runs.
// ---------------------------------------------------------------------------

/// A replace op in some index space: `a` range → `b` range.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RawOp {
    a_start: usize,
    a_end: usize,
    b_start: usize,
    b_end: usize,
}

fn intern<'a>(
    table: &mut HashMap<&'a str, u32>,
    slices: impl Iterator<Item = &'a str>,
) -> Vec<u32> {
    slices
        .map(|slice| {
            let next = table.len() as u32;
            *table.entry(slice).or_insert(next)
        })
        .collect()
}

/// Longest strictly-increasing subsequence over `values`, returning the
/// chosen indexes (patience piles with backpointers).
fn lis_indexes(values: &[usize]) -> Vec<usize> {
    let mut pile_tops: Vec<usize> = Vec::new();
    let mut back: Vec<Option<usize>> = Vec::with_capacity(values.len());
    for (index, &value) in values.iter().enumerate() {
        let pile = pile_tops.partition_point(|&top| values[top] < value);
        back.push(if pile == 0 {
            None
        } else {
            Some(pile_tops[pile - 1])
        });
        if pile == pile_tops.len() {
            pile_tops.push(index);
        } else {
            pile_tops[pile] = index;
        }
    }
    let mut chain = Vec::new();
    let mut cursor = pile_tops.last().copied();
    while let Some(index) = cursor {
        chain.push(index);
        cursor = back[index];
    }
    chain.reverse();
    chain
}

/// Patience diff over id slices; ops are pushed in ascending order.
fn patience_diff(a: &[u32], b: &[u32]) -> Vec<RawOp> {
    let mut ops = Vec::new();
    let mut stack = vec![(0usize, a.len(), 0usize, b.len())];
    while let Some((mut a0, mut a1, mut b0, mut b1)) = stack.pop() {
        while a0 < a1 && b0 < b1 && a[a0] == b[b0] {
            a0 += 1;
            b0 += 1;
        }
        while a1 > a0 && b1 > b0 && a[a1 - 1] == b[b1 - 1] {
            a1 -= 1;
            b1 -= 1;
        }
        if a0 == a1 && b0 == b1 {
            continue;
        }
        if a0 == a1 || b0 == b1 {
            ops.push(RawOp {
                a_start: a0,
                a_end: a1,
                b_start: b0,
                b_end: b1,
            });
            continue;
        }
        // Unique-common anchors between the trimmed ranges.
        let mut count_a: HashMap<u32, (usize, usize)> = HashMap::new();
        for (index, &id) in a.iter().enumerate().take(a1).skip(a0) {
            let entry = count_a.entry(id).or_insert((0, index));
            entry.0 += 1;
            entry.1 = index;
        }
        let mut count_b: HashMap<u32, (usize, usize)> = HashMap::new();
        for (index, &id) in b.iter().enumerate().take(b1).skip(b0) {
            let entry = count_b.entry(id).or_insert((0, index));
            entry.0 += 1;
            entry.1 = index;
        }
        let mut pairs: Vec<(usize, usize)> = count_a
            .iter()
            .filter(|(_, (count, _))| *count == 1)
            .filter_map(|(id, (_, a_pos))| match count_b.get(id) {
                Some((1, b_pos)) => Some((*a_pos, *b_pos)),
                _ => None,
            })
            .collect();
        if pairs.is_empty() {
            ops.push(RawOp {
                a_start: a0,
                a_end: a1,
                b_start: b0,
                b_end: b1,
            });
            continue;
        }
        pairs.sort_unstable();
        let b_positions: Vec<usize> = pairs.iter().map(|&(_, b_pos)| b_pos).collect();
        let chain: Vec<(usize, usize)> = lis_indexes(&b_positions)
            .into_iter()
            .map(|index| pairs[index])
            .collect();
        // Recurse the gaps around the anchor chain (stack order is
        // irrelevant: ops are sorted afterwards).
        let mut prev = (a0, b0);
        for &(anchor_a, anchor_b) in &chain {
            stack.push((prev.0, anchor_a, prev.1, anchor_b));
            prev = (anchor_a + 1, anchor_b + 1);
        }
        stack.push((prev.0, a1, prev.1, b1));
    }
    ops.sort_unstable_by_key(|op| (op.a_start, op.a_end));
    ops
}

/// One side's edit script entry over BASE token coordinates:
/// replace base tokens [base_start, base_end) with the side's tokens
/// [repl_start, repl_end). `base_start == base_end` is a pure insertion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct EditOp {
    base_start: usize,
    base_end: usize,
    repl_start: usize,
    repl_end: usize,
}

/// Diff base → side into a normalized edit script: sorted, non-overlapping,
/// maximal (touching ops coalesced). Segment tier first (patience over
/// segment text ids), token tier inside changed segment runs.
fn build_script(
    base_text: &str,
    base_tokens: &[Token],
    side_text: &str,
    side_tokens: &[Token],
    config: &TextMergeConfig,
) -> Vec<EditOp> {
    let base_segments = segment_ranges(base_text, base_tokens, config);
    let side_segments = segment_ranges(side_text, side_tokens, config);
    let mut seg_table: HashMap<&str, u32> = HashMap::new();
    let base_seg_ids = intern(
        &mut seg_table,
        base_segments
            .iter()
            .map(|&range| token_text(base_text, base_tokens, range)),
    );
    let side_seg_ids = intern(
        &mut seg_table,
        side_segments
            .iter()
            .map(|&range| token_text(side_text, side_tokens, range)),
    );

    let mut script = Vec::new();
    for seg_op in patience_diff(&base_seg_ids, &side_seg_ids) {
        // Segment index ranges → token index ranges.
        let base_range = token_span(
            &base_segments,
            seg_op.a_start,
            seg_op.a_end,
            base_tokens.len(),
        );
        let side_range = token_span(
            &side_segments,
            seg_op.b_start,
            seg_op.b_end,
            side_tokens.len(),
        );
        // Token tier inside the changed run.
        let mut token_table: HashMap<&str, u32> = HashMap::new();
        let base_ids = intern(
            &mut token_table,
            base_tokens[base_range.0..base_range.1]
                .iter()
                .map(|token| &base_text[token.start..token.end]),
        );
        let side_ids = intern(
            &mut token_table,
            side_tokens[side_range.0..side_range.1]
                .iter()
                .map(|token| &side_text[token.start..token.end]),
        );
        for op in patience_diff(&base_ids, &side_ids) {
            script.push(EditOp {
                base_start: base_range.0 + op.a_start,
                base_end: base_range.0 + op.a_end,
                repl_start: side_range.0 + op.b_start,
                repl_end: side_range.0 + op.b_end,
            });
        }
    }
    script.sort_unstable_by_key(|op| (op.base_start, op.base_end));
    coalesce(script)
}

fn token_span(
    segments: &[(usize, usize)],
    seg_start: usize,
    seg_end: usize,
    token_len: usize,
) -> (usize, usize) {
    let start = segments
        .get(seg_start)
        .map_or(token_len, |&(token_start, _)| token_start);
    let end = if seg_end == 0 {
        start
    } else {
        segments
            .get(seg_end - 1)
            .map_or(token_len, |&(_, token_end)| token_end)
    };
    (start, end.max(start))
}

fn coalesce(script: Vec<EditOp>) -> Vec<EditOp> {
    let mut out: Vec<EditOp> = Vec::with_capacity(script.len());
    for op in script {
        if let Some(last) = out.last_mut() {
            if last.base_end == op.base_start && last.repl_end == op.repl_start {
                last.base_end = op.base_end;
                last.repl_end = op.repl_end;
                continue;
            }
        }
        out.push(op);
    }
    out
}

// ---------------------------------------------------------------------------
// Certified compose (spec §7; text-merge-compose.maude). Everything below
// holds for ANY pair of scripts: never-fabricate and both-touched-escalates
// are enforced here, not assumed of the aligner.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ItemSide {
    Ours,
    Theirs,
    /// Convergent pair: both sides made the byte-identical change. Inert
    /// for conflict detection — each side's own script is non-overlapping,
    /// so nothing on either side can overlap an agreed edit, and composing
    /// next to agreed content carries no cross-side interleave risk.
    Both,
}

#[derive(Clone, Copy, Debug)]
struct Item {
    side: ItemSide,
    op: EditOp,
}

/// Three-way merge of `ours` and `theirs` against `base`.
pub fn text_merge(
    base: &str,
    ours: &str,
    theirs: &str,
    config: &TextMergeConfig,
) -> TextMergeOutcome {
    let base_tokens = tokenize(base);
    let ours_tokens = tokenize(ours);
    let theirs_tokens = tokenize(theirs);
    let ours_script = build_script(base, &base_tokens, ours, &ours_tokens, config);
    let theirs_script = build_script(base, &base_tokens, theirs, &theirs_tokens, config);

    // Word-token prefix counts for the proximity gap (Space/Other tokens
    // provide no separation — spec §7.3).
    let mut word_prefix = Vec::with_capacity(base_tokens.len() + 1);
    word_prefix.push(0usize);
    for token in &base_tokens {
        let last = *word_prefix.last().expect("non-empty");
        word_prefix.push(last + usize::from(token.class == TokenClass::Word));
    }
    let gap_words = |left_end: usize, right_start: usize| -> usize {
        word_prefix[right_start.max(left_end)] - word_prefix[left_end]
    };

    // Convergent pairs collapse to Both items; the rest stay one-sided.
    let mut items: Vec<Item> = Vec::new();
    let mut theirs_used = vec![false; theirs_script.len()];
    for ours_op in &ours_script {
        let convergent = theirs_script.iter().enumerate().find(|(index, theirs_op)| {
            !theirs_used[*index]
                && theirs_op.base_start == ours_op.base_start
                && theirs_op.base_end == ours_op.base_end
                && token_text(
                    theirs,
                    &theirs_tokens,
                    (theirs_op.repl_start, theirs_op.repl_end),
                ) == token_text(ours, &ours_tokens, (ours_op.repl_start, ours_op.repl_end))
        });
        match convergent {
            Some((index, _)) => {
                theirs_used[index] = true;
                items.push(Item {
                    side: ItemSide::Both,
                    op: *ours_op,
                });
            }
            None => items.push(Item {
                side: ItemSide::Ours,
                op: *ours_op,
            }),
        }
    }
    for (index, theirs_op) in theirs_script.iter().enumerate() {
        if !theirs_used[index] {
            items.push(Item {
                side: ItemSide::Theirs,
                op: *theirs_op,
            });
        }
    }
    items.sort_by_key(|item| (item.op.base_start, item.op.base_end));

    // Pairwise cross-side conflicts (hard rules are independent of the
    // dial): strict interval overlap; an insertion point touching or inside
    // the other op's closed range; same-point double insertion; and the
    // proximity rule for disjoint neighbours.
    let conflicting = |a: &Item, b: &Item| -> bool {
        if a.side == ItemSide::Both || b.side == ItemSide::Both || a.side == b.side {
            return false;
        }
        let (first, second) = if a.op.base_start <= b.op.base_start {
            (&a.op, &b.op)
        } else {
            (&b.op, &a.op)
        };
        if first.base_start < second.base_end && second.base_start < first.base_end {
            return true; // strict overlap
        }
        let a_point = a.op.base_start == a.op.base_end;
        let b_point = b.op.base_start == b.op.base_end;
        if a_point && b_point {
            return a.op.base_start == b.op.base_start; // double insertion
        }
        if a_point && b.op.base_start <= a.op.base_start && a.op.base_start <= b.op.base_end {
            return true; // insertion touching/inside the other op
        }
        if b_point && a.op.base_start <= b.op.base_start && b.op.base_start <= a.op.base_end {
            return true;
        }
        if second.base_start >= first.base_end {
            return gap_words(first.base_end, second.base_start) < config.proximity_gap;
        }
        false
    };

    // Union-find over conflicting pairs, then absorb any op whose interval
    // lies within a region's span (spans only grow; iterate to fixpoint).
    let mut parent: Vec<usize> = (0..items.len()).collect();
    fn root(parent: &mut [usize], index: usize) -> usize {
        let mut cursor = index;
        while parent[cursor] != cursor {
            parent[cursor] = parent[parent[cursor]];
            cursor = parent[cursor];
        }
        cursor
    }
    let mut any_conflict = false;
    for a in 0..items.len() {
        for b in (a + 1)..items.len() {
            if conflicting(&items[a], &items[b]) {
                let (ra, rb) = (root(&mut parent, a), root(&mut parent, b));
                parent[ra.max(rb)] = ra.min(rb);
                any_conflict = true;
            }
        }
    }
    if any_conflict {
        loop {
            let mut changed = false;
            let mut spans: HashMap<usize, (usize, usize, usize)> = HashMap::new();
            for (index, item) in items.iter().enumerate() {
                let group = root(&mut parent, index);
                let entry = spans.entry(group).or_insert((usize::MAX, 0, 0));
                entry.0 = entry.0.min(item.op.base_start);
                entry.1 = entry.1.max(item.op.base_end);
                entry.2 += 1;
            }
            for (index, item) in items.iter().enumerate() {
                let group = root(&mut parent, index);
                for (&other, &(span_start, span_end, members)) in &spans {
                    if other == group || members < 2 {
                        continue;
                    }
                    if item.op.base_start <= span_end && span_start <= item.op.base_end {
                        let ra = root(&mut parent, index);
                        parent[ra.max(other)] = ra.min(other);
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
    }

    // Group items; singleton groups compose, multi-member groups are
    // conflict regions.
    let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
    for index in 0..items.len() {
        let group = root(&mut parent, index);
        groups.entry(group).or_default().push(index);
    }
    #[derive(Debug)]
    enum Emit {
        Op(Item),
        Region { span_start: usize, span_end: usize },
    }
    let mut emits: Vec<Emit> = Vec::new();
    for (_, member_indexes) in groups {
        if member_indexes.len() == 1 {
            emits.push(Emit::Op(items[member_indexes[0]]));
        } else {
            let span_start = member_indexes
                .iter()
                .map(|&index| items[index].op.base_start)
                .min()
                .expect("non-empty group");
            let span_end = member_indexes
                .iter()
                .map(|&index| items[index].op.base_end)
                .max()
                .expect("non-empty group");
            emits.push(Emit::Region {
                span_start,
                span_end,
            });
        }
    }
    emits.sort_by_key(|emit| match emit {
        Emit::Op(item) => (item.op.base_start, item.op.base_end),
        Emit::Region {
            span_start,
            span_end,
            ..
        } => (*span_start, *span_end),
    });

    // A side's text for a base span: the side's contiguous token range
    // covering it, located by accumulating (replacement − base) length
    // deltas of that side's ops before/inside the span. Contiguity is by
    // construction (between ops the side equals base), so never-fabricate
    // holds structurally: every emitted text is one slice of one input.
    let side_span = |script: &[EditOp], span_start: usize, span_end: usize| -> (usize, usize) {
        let mut start_delta = 0isize;
        let mut inner_delta = 0isize;
        for op in script {
            let op_len = op.base_end - op.base_start;
            let repl_len = op.repl_end - op.repl_start;
            let delta = repl_len as isize - op_len as isize;
            if op.base_end <= span_start && (op.base_start < span_start || op_len > 0) {
                start_delta += delta;
            } else if op.base_start >= span_start && op.base_end <= span_end {
                inner_delta += delta;
            }
        }
        let side_start = (span_start as isize + start_delta) as usize;
        let side_end = (span_end as isize + start_delta + inner_delta) as usize;
        (side_start, side_end)
    };

    let mut pieces: Vec<MergePiece> = Vec::new();
    let push_merged = |pieces: &mut Vec<MergePiece>, text: &str, provenance: Provenance| {
        if !text.is_empty() {
            pieces.push(MergePiece::Merged {
                text: text.to_owned(),
                provenance,
            });
        }
    };
    let mut cursor = 0usize;
    let mut conflicted = false;
    for emit in &emits {
        let (start, end) = match emit {
            Emit::Op(item) => (item.op.base_start, item.op.base_end),
            Emit::Region {
                span_start,
                span_end,
                ..
            } => (*span_start, *span_end),
        };
        push_merged(
            &mut pieces,
            token_text(base, &base_tokens, (cursor, start)),
            Provenance::Base,
        );
        match emit {
            Emit::Op(item) => {
                let (side_text_body, side_tokens_ref, provenance) = match item.side {
                    ItemSide::Ours | ItemSide::Both => (
                        ours,
                        &ours_tokens,
                        if item.side == ItemSide::Both {
                            Provenance::Both
                        } else {
                            Provenance::Ours
                        },
                    ),
                    ItemSide::Theirs => (theirs, &theirs_tokens, Provenance::Theirs),
                };
                push_merged(
                    &mut pieces,
                    token_text(
                        side_text_body,
                        side_tokens_ref,
                        (item.op.repl_start, item.op.repl_end),
                    ),
                    provenance,
                );
            }
            Emit::Region {
                span_start,
                span_end,
                ..
            } => {
                conflicted = true;
                let ours_range = side_span(&ours_script, *span_start, *span_end);
                let theirs_range = side_span(&theirs_script, *span_start, *span_end);
                pieces.push(MergePiece::Conflict {
                    base_text: token_text(base, &base_tokens, (*span_start, *span_end)).to_owned(),
                    ours_text: token_text(ours, &ours_tokens, ours_range).to_owned(),
                    theirs_text: token_text(theirs, &theirs_tokens, theirs_range).to_owned(),
                });
            }
        }
        cursor = end.max(cursor);
    }
    push_merged(
        &mut pieces,
        token_text(base, &base_tokens, (cursor, base_tokens.len())),
        Provenance::Base,
    );

    if conflicted {
        TextMergeOutcome::Conflicted { pieces }
    } else {
        let merged: String = pieces
            .iter()
            .map(|piece| match piece {
                MergePiece::Merged { text, .. } => text.as_str(),
                MergePiece::Conflict { .. } => unreachable!("clean outcome has no conflicts"),
            })
            .collect();
        TextMergeOutcome::Clean { pieces, merged }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn merge(base: &str, ours: &str, theirs: &str) -> TextMergeOutcome {
        text_merge(base, ours, theirs, &TextMergeConfig::default())
    }

    fn merged_text(outcome: &TextMergeOutcome) -> &str {
        match outcome {
            TextMergeOutcome::Clean { merged, .. } => merged,
            TextMergeOutcome::Conflicted { .. } => panic!("expected clean merge: {outcome:?}"),
        }
    }

    /// Never-fabricate (spec §7.6 invariant 1): every piece's text is a
    /// contiguous substring of the input its provenance names.
    fn assert_never_fabricate(outcome: &TextMergeOutcome, base: &str, ours: &str, theirs: &str) {
        for piece in outcome.pieces() {
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
                        "piece {text:?} not in its provenance source"
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

    /// Identity (spec §7.6 invariant 3): an unchanged side yields entirely.
    #[test]
    fn identities() {
        let base = "The quick brown fox jumps over the lazy dog.";
        let edited = "The quick brown fox leaps over the sleepy dog.";
        assert_eq!(merged_text(&merge(base, base, edited)), edited);
        assert_eq!(merged_text(&merge(base, edited, base)), edited);
        assert_eq!(merged_text(&merge(base, edited, edited)), edited);
        assert_eq!(merged_text(&merge(base, base, base)), base);
    }

    /// Word atoms: disjoint edits INSIDE one sentence compose when
    /// separated by enough words — the case line- and sentence-granularity
    /// merges can never grant.
    #[test]
    fn disjoint_word_edits_in_one_sentence_compose() {
        let base = "The quick brown fox jumps over the lazy dog tonight.";
        let ours = "The swift brown fox jumps over the lazy dog tonight.";
        let theirs = "The quick brown fox jumps over the lazy cat tonight.";
        let outcome = merge(base, ours, theirs);
        assert_never_fabricate(&outcome, base, ours, theirs);
        assert_eq!(
            merged_text(&outcome),
            "The swift brown fox jumps over the lazy cat tonight."
        );
    }

    /// Both sides rewriting the same words is a real conflict carrying all
    /// three slices — never a picked winner (spec §7.6 invariant 2).
    #[test]
    fn overlapping_rewrites_escalate_with_three_slices() {
        let base = "The quick brown fox jumps over the lazy dog.";
        let ours = "The nimble crimson fox jumps over the lazy dog.";
        let theirs = "The slow grey fox jumps over the lazy dog.";
        let outcome = merge(base, ours, theirs);
        assert_never_fabricate(&outcome, base, ours, theirs);
        let TextMergeOutcome::Conflicted { pieces } = &outcome else {
            panic!("expected conflict: {outcome:?}");
        };
        let conflict = pieces
            .iter()
            .find_map(|piece| match piece {
                MergePiece::Conflict {
                    base_text,
                    ours_text,
                    theirs_text,
                } => Some((base_text, ours_text, theirs_text)),
                _ => None,
            })
            .expect("one conflict region");
        assert!(conflict.0.contains("quick brown"));
        assert!(conflict.1.contains("nimble crimson"));
        assert!(conflict.2.contains("slow grey"));
    }

    /// The proximity rule (spec §7.3): cross-side edits one word apart
    /// escalate at the default dial, and compose when the dial is zero —
    /// the ratchet the evaluation program tunes.
    #[test]
    fn proximity_dial_escalates_near_edits() {
        let base = "alpha beta gamma delta";
        let ours = "ALPHA beta gamma delta";
        let theirs = "alpha beta GAMMA delta";
        let near = merge(base, ours, theirs);
        assert!(
            matches!(near, TextMergeOutcome::Conflicted { .. }),
            "one intervening word < default gap 2: {near:?}"
        );
        let loose = text_merge(
            base,
            ours,
            theirs,
            &TextMergeConfig {
                proximity_gap: 0,
                ..TextMergeConfig::default()
            },
        );
        assert_eq!(merged_text(&loose), "ALPHA beta GAMMA delta");
    }

    /// Convergent edits are one change (spec §7.1), tagged Both.
    #[test]
    fn convergent_edits_take_once() {
        let base = "shared draft sentence stays here.";
        let both = "shared final sentence stays here.";
        let outcome = merge(base, both, both);
        assert_eq!(merged_text(&outcome), both);
        assert!(outcome.pieces().iter().any(|piece| matches!(
            piece,
            MergePiece::Merged {
                provenance: Provenance::Both,
                ..
            }
        )));
    }

    /// Same-point concurrent insertions conflict regardless of the dial
    /// (ordering them is a semantic choice no algorithm should fake);
    /// identical insertions converge.
    #[test]
    fn same_point_insertions() {
        let base = "one two three four five six.";
        let ours = "one two hello three four five six.";
        let theirs = "one two world three four five six.";
        let differing = text_merge(
            base,
            ours,
            theirs,
            &TextMergeConfig {
                proximity_gap: 0,
                ..TextMergeConfig::default()
            },
        );
        assert!(matches!(differing, TextMergeOutcome::Conflicted { .. }));
        let identical = merge(base, ours, ours);
        assert_eq!(merged_text(&identical), ours);
    }

    /// Symmetry (spec §7.6 invariant 4): swapping sides yields identical
    /// clean bytes; conflicts swap ours/theirs.
    #[test]
    fn symmetry() {
        let base = "The quick brown fox jumps over the lazy dog tonight, quietly and alone.";
        let left = "The swift brown fox jumps over the lazy dog tonight, quietly and alone.";
        let right = "The quick brown fox jumps over the lazy dog tonight, loudly and alone.";
        assert_eq!(
            merged_text(&merge(base, left, right)),
            merged_text(&merge(base, right, left))
        );
        let conflict_left = "The nimble fox jumps over the lazy dog tonight, quietly and alone.";
        let conflict_right = "The sly fox jumps over the lazy dog tonight, quietly and alone.";
        let forward = merge(base, conflict_left, conflict_right);
        let backward = merge(base, conflict_right, conflict_left);
        let slices = |outcome: &TextMergeOutcome| -> Vec<(String, String, String)> {
            outcome
                .pieces()
                .iter()
                .filter_map(|piece| match piece {
                    MergePiece::Conflict {
                        base_text,
                        ours_text,
                        theirs_text,
                    } => Some((base_text.clone(), ours_text.clone(), theirs_text.clone())),
                    _ => None,
                })
                .collect()
        };
        let forward_slices = slices(&forward);
        let backward_slices = slices(&backward);
        assert_eq!(forward_slices.len(), 1);
        assert_eq!(backward_slices.len(), 1);
        assert_eq!(forward_slices[0].0, backward_slices[0].0);
        assert_eq!(forward_slices[0].1, backward_slices[0].2);
        assert_eq!(forward_slices[0].2, backward_slices[0].1);
    }

    /// Deletion composes against a distant edit; delete-vs-edit of the
    /// same words escalates.
    #[test]
    fn deletions() {
        let base = "keep one two three four five keep tail here.";
        let ours = "keep two three four five keep tail here.";
        let theirs = "keep one two three four five keep tail now.";
        let outcome = merge(base, ours, theirs);
        assert_never_fabricate(&outcome, base, ours, theirs);
        assert_eq!(
            merged_text(&outcome),
            "keep two three four five keep tail now."
        );
        let both_touch = merge(
            "alpha beta gamma delta epsilon",
            "alpha delta epsilon",
            "alpha beta GAMMA delta epsilon",
        );
        assert!(matches!(both_touch, TextMergeOutcome::Conflicted { .. }));
    }

    /// Prose paragraphs: edits in different sentences of one paragraph
    /// compose — the everyday case the path-level merge always escalated.
    #[test]
    fn paragraph_edits_in_different_sentences_compose() {
        let base = "Whipplescript mediates every write through the workspace API. \
                    The merge engine composes disjoint edits without ceremony. \
                    Conflicts surface as structured objects, never markers.";
        let ours = "Whipplescript mediates and witnesses every write through the workspace API. \
                    The merge engine composes disjoint edits without ceremony. \
                    Conflicts surface as structured objects, never markers.";
        let theirs = "Whipplescript mediates every write through the workspace API. \
                      The merge engine composes disjoint edits without ceremony. \
                      Conflicts surface as structured, provenance-bearing objects, never markers.";
        let outcome = merge(base, ours, theirs);
        assert_never_fabricate(&outcome, base, ours, theirs);
        let merged = merged_text(&outcome);
        assert!(merged.contains("mediates and witnesses every write"));
        assert!(merged.contains("structured, provenance-bearing objects"));
    }

    /// Large-document locality: a distant pair of edits in a big body
    /// composes (the anchor tier keeps alignment local), and unrelated
    /// far-apart content is untouched.
    #[test]
    fn large_document_distant_edits_compose() {
        let paragraph =
            "This sentence carries enough distinct words to anchor alignment across paragraphs. ";
        let mut body = String::new();
        for index in 0..200 {
            body.push_str(&format!("Paragraph {index} begins. {paragraph}"));
        }
        let base = body.clone();
        let ours = base.replacen("Paragraph 3 begins.", "Paragraph three begins.", 1);
        let theirs = base.replacen("Paragraph 190 begins.", "Paragraph one-ninety starts.", 1);
        let outcome = merge(&base, &ours, &theirs);
        assert_never_fabricate(&outcome, &base, &ours, &theirs);
        let merged = merged_text(&outcome);
        assert!(merged.contains("Paragraph three begins."));
        assert!(merged.contains("Paragraph one-ninety starts."));
    }

    /// Unicode text (no ASCII spaces needed): word atoms are Unicode
    /// alphanumeric runs, so CJK-adjacent scripts merge without any
    /// sentence segmentation.
    #[test]
    fn unicode_words_merge() {
        let base = "ναὸς σοφίας καὶ γνώσεως καὶ ἀληθείας ἵσταται ἐνθάδε";
        let ours = "ναὸς ΣΟΦΙΑΣ καὶ γνώσεως καὶ ἀληθείας ἵσταται ἐνθάδε";
        let theirs = "ναὸς σοφίας καὶ γνώσεως καὶ ἀληθείας ἵσταται ἐκεῖ";
        let outcome = merge(base, ours, theirs);
        assert_never_fabricate(&outcome, base, ours, theirs);
        assert_eq!(
            merged_text(&outcome),
            "ναὸς ΣΟΦΙΑΣ καὶ γνώσεως καὶ ἀληθείας ἵσταται ἐκεῖ"
        );
    }

    /// Metamorphic sweep over generated edit pairs: every outcome upholds
    /// never-fabricate, and clean outcomes uphold symmetry. Deterministic
    /// (splitmix-seeded positions, no wall clock, no rng crate).
    #[test]
    fn metamorphic_sweep() {
        const WORDS: &[&str] = &[
            "alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta", "iota", "kappa",
            "lambda", "mu", "nu", "xi", "omicron", "pi", "rho", "sigma", "tau", "upsilon",
        ];
        fn splitmix(seed: u64) -> u64 {
            let mut z = seed.wrapping_add(0x9e3779b97f4a7c15);
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
            z ^ (z >> 31)
        }
        for seed in 0u64..40 {
            let length = 12 + (splitmix(seed) % 30) as usize;
            let base_words: Vec<&str> = (0..length)
                .map(|index| WORDS[(splitmix(seed * 31 + index as u64) % 20) as usize])
                .collect();
            let base = base_words.join(" ");
            let mutate = |salt: u64| -> String {
                let mut words = base_words.clone();
                let position = (splitmix(seed * 7 + salt) % length as u64) as usize;
                words[position] = "EDITED";
                words.join(" ")
            };
            let ours = mutate(1);
            let theirs = mutate(2);
            let outcome = merge(&base, &ours, &theirs);
            assert_never_fabricate(&outcome, &base, &ours, &theirs);
            if let TextMergeOutcome::Clean { merged, .. } = &outcome {
                let swapped = merge(&base, &theirs, &ours);
                assert_eq!(
                    merged,
                    merged_text(&swapped),
                    "clean merges are side-symmetric (seed {seed})"
                );
            }
            // Identity holds for every generated body too.
            assert_eq!(merged_text(&merge(&base, &ours, &base)), ours.as_str());
        }
    }

    /// The outcome payload serializes with stable snake_case tags — the
    /// merge-preview surface contract.
    #[test]
    fn piece_serialization_shape() {
        let outcome = merge(
            "one two three four five",
            "ONE two three four five",
            "one two three four FIVE",
        );
        let json = serde_json::to_value(outcome.pieces()).expect("serialize");
        let first = &json[0];
        assert_eq!(first["kind"], "merged");
        assert_eq!(first["provenance"], "ours");
        assert_eq!(TEXT_MERGE_ALGORITHM, "text-merge/1");
    }
}
