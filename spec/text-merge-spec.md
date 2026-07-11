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

**Insertion-pair exemption (settled 2026-07-11, corpus finding):** two
pure insertions at DISTINCT points are exempt from the proximity rule.
The hazard §7.3 guards against — slicing one rewrite into interleaved
fragments via shared anchor words — requires base-consuming ops; a pure
insertion is atomic in the output (its full text lands contiguously),
so distinct-point insertion pairs cannot mechanically interleave a
single passage. Both added texts survive whole, ordered by base
position. This exemption is what lets the block-reorder-vs-in-place-
edit case compose to the ideal result (the moved block carries the
other side's edit) whenever the reorder diff expresses as insert+delete
around the block interior — see the move-and-edit entries in the
corpus registry. Mixed pairs (insertion vs replace/delete) and
same-point insertion pairs keep their conflict rules unchanged.

### 7.3 The proximity rule (the tuning dial)

Two ops from opposite sides conflict if their base ranges overlap, OR
if the base tokens strictly between them contain fewer than `d` **Word**
tokens (Space/Other tokens provide no separation). Default `d = 2`,
override `WHIPPLESCRIPT_TEXT_MERGE_GAP`. Exception: pure insertion
pairs at distinct points (§7.2) — atomicity makes them interleave-safe
at any distance.

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
   inputs). Refined by §12.3 for remembered resolutions, which are
   recorded human content under a distinct `resolved` provenance.
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
  the alignment tier later — but NOT core-neutrally: composing side
  B's in-place edit into side A's relocated copy weakens
  never-fabricate from slice-level to atom-level and changes the
  piece/provenance structure, so it takes its own design pass + model
  extension (move-vs-move, move-vs-delete, edit-straddling-a-move
  semantics) before any code. The evaluation corpus (§10) is its
  acceptance gate.
- **Deeper anchor trees** for very large documents (two-level covers
  the 8 MiB cap comfortably).
- **Region-level resolution memory + editor save flow** — whip half
  BUILT 2026-07-11 per §12 (`save_with_base`, `merge_preview`,
  `record_region_resolutions`, memory-aware refinement, partial-memory
  model searches); the gaugedesk fold UI remains (SUB-6 there).
- **The evaluation corpus itself** (§10) — the harness and fixtures
  land with the red-team campaign, not this slice; v1 ships the
  metamorphic property tests inline.

## 12. Editor save flow + region-level resolution memory (SUB-6 co-design)

Settled 2026-07-11 (Jack delegated the engineering decisions; gaugedesk
half tracked as SUB-6 in its implementation tracker). The two features
are one design: the editor's conflict fold is the surface that MINTS
region resolutions, and region memory is what makes resolving pay
forward.

### 12.1 The save verb (whip side)

One store-level verb, consumed by gaugedesk's
`WhippleWorkspaceProvider`:

    save_with_base(branch, path, draft, base_cut_id,
                   resolutions: [RegionResolution], cut_id, at)
      -> Written { cut_id }                          -- head hadn't moved
       | Merged  { cut_id, merged, pieces }          -- composed cleanly
       | Conflicted { head_cut_id, pieces }          -- no write; fold me

Semantics: load the path's body at `base_cut_id` (the revision the
editor's draft started from) and at the current head. Head unmoved (or
body identical) → plain mediated write. Otherwise three-way with
base = base-cut body, ours = head body (the line's content), theirs =
draft (the editor's proposal), through the same region-memory-aware
merge as branch reconciliation (§12.3) — one merge behavior whip-wide,
same dials. Clean → write the merged body (optimistic head advance;
stale head retries the whole verb) and return the pieces so the editor
can render provenance. Conflicted → return the regions and write
NOTHING.

**Decision — editor-save conflicts are transient, not persisted.**
Branch-merge conflicts stay first-class open rows (the ask surface for
a daemon-mediated, possibly-absent resolver). An editor save has the
resolving human present holding the draft; persisting an open-conflict
row for a decision made in the next thirty seconds is noise, and the
draft lives client-side if abandoned. The conflict-bearing-state
doctrine is about branch lines, not in-flight PUTs.

A read-only twin, `merge_preview(branch, path, draft, base_cut_id) ->
pieces`, is the same compute without the write — the editor's live
fold (gaugedesk already streams file-change events; a change to the
open path while the draft is dirty triggers a preview).

### 12.2 Region resolutions: minting

When the editor submits a resolved save, `resolutions` carries one
entry per region the user settled:

    RegionResolution { base_text, ours_text, theirs_text, resolution_text }

Each entry stores `resolution_text` in the content store (a content
id — erasure honesty applies automatically) and records a memory row
in the existing resolution-memory table, kind-tagged `region`, keyed
by the content hashes of the three region texts. Recording rides the
same call as the save: resolving and saving are one atomic intent.
`resolve_conflict`'s take-ours/take-theirs/authored-body shape (vw
note §7.3) is reused per-region in the editor fold.

### 12.3 Region memory: applying

A wrapper `text_merge_with_memory(base, ours, theirs, config, lookup)`
post-processes the §7.5 outcome: each Conflict piece's triple key is
looked up; a remembered resolution WITH LIVE PAYLOAD replaces the
region as a Merged piece with provenance `resolved`; if every region
resolves, the outcome upgrades to Clean. Partial memory shrinks the
conflict set honestly — never fakes clean. The wrapper is used by BOTH
`refine_source_conflicts` stage 3 and `save_with_base`.

**Decision — auto-apply is ON, exactly like path-level memory.** Same
doctrine (rerere as ordinary workspace-plane knowledge), exact triple
match only, apply only while the payload is live, workspace-scoped.
The extra safety at region level is structural: region boundaries
depend on the alignment heuristic, so an aligner upgrade degrades
remembered keys to MISSES — never misapplication. Fail-closed under
drift, by construction.

**Invariant restatement (amends §7.6.1):** never-fabricate governs
ALGORITHMIC composition — every algorithmically merged piece is a
contiguous slice of one input. A `resolved` piece is recorded human
content applied by exact content-addressed key: the same honesty
posture as path-level resolution memory, distinguishable in the piece
provenance, erasure-respecting. The Maude model's never-fabricate bite
is unchanged (it models the algorithmic layer); the memory tier's
exact-triple/live-only semantics are already modeled in
resolution-memory.maude, which the region tier reuses; the build adds
a search pinning that partial region memory never yields Clean.

### 12.4 Gaugedesk half (SUB-6 there)

The file PUT gains `{content, base_cut, resolutions?}`; the viewer
tracks the cut id its content came from. Responses: plain save →
unchanged UX; `merged` → editor refreshes to the merged body with a
quiet affordance ("merged with concurrent changes"), recorded on the
AUDIT plane (ADR-0082 posture: rationale in audit, not conversation);
`conflicted` (409) → inline region fold rendering base/ours/theirs
per region with pick-or-edit, submit = second save carrying
`resolutions`. Live fold: on a file-change event for a dirty open
file, call `merge_preview` and offer the fold proactively.

**Sequencing constraint:** gaugedesk pins whip by git rev; SUB-6's
build needs `save_with_base` reachable from that pin. Land the merge
cluster on v0.4, then repoint gaugedesk's pin (the SUB-3/4/5
host-governance commits are already cherry-picked onto v0.4, so the
protocol extensions travel); the interim dev-loop `[patch]` in
gaugedesk's root Cargo.toml (currently aimed at whipplescript-sub3)
gets updated or removed at that repoint.

### 12.5 Build order

1. Evaluation corpus scaffold (§10, in-repo) — the instrument lands
   before any further heuristic work.
2. Whip: `text_merge_with_memory` + region rows + `save_with_base` +
   `merge_preview` (+ the partial-memory model search).
3. Gaugedesk: provider + PUT contract + fold UI (after the pin
   repoint).
4. Move detection design pass (§11), corpus-gated.
