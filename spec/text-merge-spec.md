# Text merge: recursive token-level three-way merge

Status: SPECIFIED 2026-07-11 (design settled in discussion with Jack
2026-07-11; supersedes the earlier line-level `merge_text` sketch and the
sentence-atom/markdown-parse variant, both rejected in that discussion).
Formal model: `models/maude/text-merge-compose.maude`. Implementation:
`crates/whipplescript-store/src/text_merge.rs` (pure, host-agnostic).
Consumer: the versioned workspace's both-modified blob path
(`merge.rs` → `PathConflict` refinement) and, downstream, gaugedesk's
compare-and-swap editor save + merge preview (SUB-6 there).

## 1. Purpose and doctrine fit

The blob plane's three-way merge (`merge.rs`) is path-level: per path,
an unchanged side yields to the changed one, byte-identical outcomes
converge, and *any* other divergence escalates. That is the right floor —
escalate-never-fake — but it means every concurrent edit to the same text
file is a conflict, which is far too coarse for prose and collaborative
editing.

The house merge doctrine is **merge granularity = legibility depth**:
declaration-granularity for whip source (slice certificates,
merge-slice.maude), path-level for opaque bytes. This tier extends
structure-aware merge *down* into semi-legible content — text we can
tokenize but not parse — without touching either neighbor:

- Whip-legible sources keep declaration-granularity merge. The
  merge-slice bite (text merge on legible content silently accepts
  cross-declaration semantic conflicts) is untouched: this tier is
  **never** applied to content the slicer can read.
- Binary/oversized/invalid-UTF-8 blobs keep the path-level escalation
  exactly as today.

Two design decisions from the settling discussion shape everything:

1. **The atom is the word, not the sentence or the line.** Newlines are
   too sparse in prose for line diff to work; sentence segmentation was
   rejected as a semantic dependency the algorithm doesn't need. Disjoint
   word-level edits inside one sentence compose; the incoherent-interleave
   hazard is handled by the proximity rule (§7.3), not by coarser atoms.
2. **Intermediate alignment anchors are chosen for algorithmic efficiency
   alone.** They carry no semantic meaning, so they need no semantic
   correctness: content-defined boundaries over the token stream (the
   FastCDC idea at token granularity) replace markdown parsing and
   sentence segmentation entirely. One engine for all text — no CJK
   segmentation problem, no per-filetype segmenter stacks, no markdown
   edge cases in the alignment path.

## 2. The certified/heuristic split

The architecture mirrors the slicer-client split. Two layers with an
asymmetric contract:

- **Composition is certified.** Given *any* pair of per-side edit
  scripts over base coordinates, the compose layer guarantees:
  - **Never-fabricate:** every byte of output is a contiguous slice of
    exactly one of base/ours/theirs. The merge selects and orders; it
    never synthesizes.
  - **Both-touched ⇒ conflict:** overlapping (or proximate, §7.3) change
    regions from opposite sides always escalate as a structured conflict
    region carrying base + both sides. No silent winner.
  These two properties are the Maude model's bite
  (`text-merge-compose.maude`) and are restated as Rust property tests.
- **Alignment is heuristic and swappable.** The tokenizer, anchor
  segmentation, and diff produce the edit scripts. A worse aligner can
  only *inflate the conflict count* (or mislocate region boundaries
  within the honest three-slice conflict payload); it can never produce
  a fabricated or silently-wrong clean merge of genuinely overlapping
  edits. This is what makes aggressive iteration on the heuristic safe:
  correctness never rides on it.

## 3. Eligibility

`text_merge` is attempted only when ALL hold; otherwise the path-level
conflict stands unrefined:

- The path conflicted as **modify/modify** with a present base
  (`PathConflict` with `base`, `ours`, `theirs` all `Some`). Add/add
  (no base) and modify/delete keep escalating — a deletion never
  silently loses to concurrent work, and two unrelated creations have
  no base coordinates to compose on.
- All three bodies are available (not erased) and valid UTF-8 (the
  content store's text tier guarantees this for stored rows; imported
  or reassembled content is re-checked).
- Each body is at most `WHIPPLESCRIPT_TEXT_MERGE_MAX_BYTES` (default
  8 MiB). Oversized inputs escalate path-level, honestly tagged.
- The path is not whip-legible source (`.whip`), which is owned by the
  declaration-granularity engine.

## 4. Tokenization (the atom layer)

Deterministic, whitespace-preserving, Unicode-safe. The token stream
concatenates back to the exact input (tokens tile the text):

- **Word:** maximal run of alphanumeric characters (Unicode
  `is_alphanumeric`) or `_`.
- **Space:** maximal run of whitespace.
- **Other:** each remaining character is its own token (punctuation,
  symbols).

Word atoms are the semantic unit; Space and Other tokens participate in
diff and compose identically (they are atoms too — never-fabricate
covers whitespace), but do not count toward the proximity gap (§7.3),
so `foo. bar` offers no more separation than `foo bar`.

## 5. Anchor segmentation (the efficiency tier)

Purpose: avoid O(N·D) token-level diff over whole documents when edits
are local, by matching identical regions in O(1) via hashes and
descending only into changed spans. Properties:

- Boundaries are **content-defined over token hashes**: a segment ends
  after a token whose 64-bit FNV-1a hash satisfies `hash & mask == 0`,
  subject to min/max segment lengths (in tokens). Defaults: min 16,
  average 64 (6 mask bits), max 512. An edit re-anchors only its
  neighborhood; distant segments keep identical hashes.
- Each segment carries the FNV-1a hash over its tokens' bytes; the
  document is a sequence of segment hashes. Alignment first diffs the
  two segment-hash sequences (Myers), then runs token-level Myers only
  inside replaced segment runs. Deeper trees (segments of segments) are
  a compatible extension for very large documents; v1 is two-level.
- Anchors are **ephemeral and NOT identity-bearing** — the sharp
  contrast with `chunking.rs`, whose boundaries are frozen because they
  mint storage identity. Merge anchors exist only inside one merge
  computation; parameters and even the whole scheme can change freely
  between versions without re-keying anything. (Determinism still
  matters *within* a run: same binary + same inputs ⇒ same outcome, so
  recorded merges replay. The outcome payload carries
  `algorithm: "text-merge/1"` so provenance survives upgrades.)

## 6. Alignment (per-side edit scripts)

For each side S ∈ {ours, theirs}, diff base → S (segment tier, then
token tier inside changed runs; token equality is byte equality — hashes
accelerate, bytes decide, so hash collisions cannot corrupt alignment).
The result is a normalized **edit script**: a sorted, non-overlapping,
non-adjacent list of operations

    replace(base_token_range [i, j), replacement = token range of S)

where `i == j` encodes a pure insertion at base gap `i`, and an empty
replacement encodes deletion. Adjacent/overlapping ops are coalesced
during normalization, so each op is a maximal changed region on that
side. Myers with token interning is deterministic; ties broken
consistently (prefer-delete-first), so scripts are a pure function of
(base, side).

## 7. Certified compose

Input: base token stream + the two normalized edit scripts. Output: a
piece list in base order.

### 7.1 Composition rules

Walking base coordinates with both scripts:

- **Untouched base spans** emit as base pieces.
- **One-sided ops** (no opposite-side op within reach, §7.3) apply:
  the replacement tokens emit as a piece attributed to that side.
- **Convergent ops** — identical base range AND byte-identical
  replacement — emit once, attributed `both` (the token-level analogue
  of the manifest merge's `o == t` rule).
- **Everything else within reach escalates** as a conflict region
  (§7.4).

### 7.2 Insertions

Two insertions at the same base gap conflict unless byte-identical
(ordering two concurrent insertions at one point is a semantic choice
no algorithm should fake). An insertion coincident with an opposite-side
op whose range covers or abuts that gap is within reach and escalates
with it.

### 7.3 The proximity rule (the tuning dial)

Two ops from opposite sides conflict if their base ranges overlap, OR
if the base tokens strictly between them contain fewer than `d` **Word**
tokens (Space/Other tokens provide no separation). Default `d = 2`,
override `WHIPPLESCRIPT_TEXT_MERGE_GAP`.

This is the guard against the word-atom hazard: two rewrites of the
same sentence that happen to slice into technically-disjoint ranges by
anchoring on shared filler words get escalated instead of interleaved.
It is deliberately conservative at launch (precision-first: a silent
bad merge costs more than a spurious conflict) and is the parameter the
evaluation program (§10) ratchets with corpus evidence — progressive
rigor applied to a heuristic.

### 7.4 Conflict regions

Conflicting ops close transitively (A conflicts with B, B with C ⇒ one
region). A region's base span is the smallest base range covering every
involved op; each side's text for the region is its own ops applied to
that span (a contiguous slice of that side by construction). The region
emits as

    conflict { base_text, ours_text, theirs_text }

— the token-level twin of `PathConflict`: base + both sides + (from the
envelope) both sides' provenance, never `<<<<<<<` markers, never a
picked winner.

### 7.5 Outcome surface

    TextMergeOutcome =
      Clean      { pieces: Vec<MergePiece>, merged: String }
    | Conflicted { pieces: Vec<MergePiece> }

    MergePiece = Merged { text, provenance: base|ours|theirs|both }
               | Conflict { base_text, ours_text, theirs_text }

`Clean.merged` is the piece concatenation. `Conflicted` is a legal,
tagged state — conflicts don't block; the piece list is exactly the
editor-fold / merge-preview payload (per-region provenance feeds review
of even the *merged* spans). JSON serialization is part of the surface
(serde), tagged with `algorithm: "text-merge/1"`.

### 7.6 Invariants (modeled + property-tested)

1. **Never-fabricate:** every piece's text is a contiguous substring of
   exactly one input (conflict regions: three substrings of their three
   inputs).
2. **Both-touched ⇒ conflict:** no clean composition exists when ops
   from opposite sides overlap or violate the proximity rule.
3. **Identity:** merge(b, b, r) = r; merge(b, l, b) = l;
   merge(b, x, x) = x.
4. **Symmetry:** swapping sides yields the same clean bytes; conflicts
   swap ours/theirs.
5. **Disjoint preservation:** edits beyond the proximity reach both
   survive verbatim.

## 8. Wire-in

Text merge is the third stage of the existing conflict-refinement
pipeline (`refine_source_conflicts` in vcs.rs, shared by rebase-down
and merge-up): resolution memory first (a human resolution beats the
algorithm), certified `.whip` declaration merge second, token-level
text merge last, for eligible (§3) paths. `Clean` folds the merged
body into the manifest (new content id minted). `Conflicted` keeps the
path escalating exactly as today — and because the merge is a
deterministic pure function of the content-addressed (base, ours,
theirs) triple the conflict row already carries, the region detail is
recomputable on demand by any consumer (editor fold, merge preview)
with no conflict-schema change.

## 9. Formal model

`models/maude/text-merge-compose.maude` models the certified layer over
abstract atom sequences: base as an indexed atom list, per-side ops as
`rep(lo, hi, atoms)`, compose as guarded rewrites. Coverage: disjoint
ops compose preserving both; convergent ops take-once; overlap and
proximity violations escalate. Bite (NoSolution + `RESIDUAL:Cfg` soup
variable, house pattern): no reachable clean composition for
overlapping ops, none for proximity-violating ops, and no reachable
output containing an atom absent from all three inputs
(never-fabricate). The heuristic tier is deliberately NOT modeled —
that is the point of the split.

## 10. Evaluation program (summary; SSOT for the strategy discussion)

Acceptance is asymmetric: **zero bad merges** on the whole suite, then
minimize spurious conflicts. Corpus: synthetic scripted-edit pairs over
real documents (ground truth by construction; oversample hard regions),
mined real merges (Wikipedia revision histories, git `.md` merge
commits), and label-free metamorphic fuzzing of the §7.6 invariants
plus reflow-invariance. Adversarial red-team over the enumerable attack
surface (near-duplicate blocks, shared-filler-word slicing, split/join
edits, move-and-edit, one-giant-paragraph, CJK, punctuation-dense text)
feeding a NAMED failure-mode registry: each mode gets a fixture family
and a disposition — fixed / escalates-by-design / accepted-with-
rationale. User corrections of merged results flow back as labeled
counterexamples. The aligner is a pure function of three strings, so
the whole corpus re-scores in seconds and every field failure is a
replayable fixture. Candidate first campaign for gauge/mark when it
exists. The proximity dial `d` and anchor parameters are the tunables;
loosening them requires corpus evidence.

## 11. v1 scope and deferrals

In: everything above. Out (deferred, compatible by construction):

- **Move detection** — a moved-and-edited block degrades to
  delete+insert (conflict or one-sided, per the scripts). Slots into
  the alignment tier later; the certified core is untouched.
- **Deeper anchor trees** for very large documents (two-level covers
  the 8 MiB cap comfortably).
- **Resolution-memory integration** for region-level conflicts
  (path-level resolution memory exists; region-level keying is a
  follow-up).
- **The evaluation corpus itself** (§10) — the harness and fixtures
  land with the red-team campaign, not this slice; v1 ships the
  metamorphic property tests inline.
