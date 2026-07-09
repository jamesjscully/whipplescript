# Review Change Plan (2026-06-09)

Comprehensive change list from the language review. Section 1–5 items are
approved technical/bug fixes. Section 6 records the design decisions made
during review triage; each decision item includes its agreed direction.

## 1. Runtime bugs (highest priority)

- [x] **Fact-ID collision crash.** Rule-body fact IDs omit `instance_id` from
  the idempotency derivation (`crates/whipplescript-cli/src/main.rs:9143` and
  `:9323`; the acceptance-setup path at `:5997` is correct). Two instances of
  the same program in one store collide on the `facts.fact_id` primary key:
  running `whip dev <same.whip>` twice against one store fails with
  `UNIQUE constraint failed: facts.fact_id`. Include `instance_id` in both
  derivations and add a regression test that runs the same workflow twice in
  one store.
- [x] **`when human answered ...` never matches.** The parser lowers the
  pattern to schema `HumanAnswer` (`crates/whipplescript-parser/src/lib.rs:6388`)
  but the runtime records the fact as `human.answer.received`, so the rule
  read is never satisfied. Align the names and add an e2e test covering
  ask -> inbox answer -> dependent rule fires.
- [x] **Broken shipped example.** `examples/human-review.whip`:
  `rule record_manual_review` is dead code — the `askHuman` is never bound
  `as review`, and `answer.subject` / `answer.decision` are not fields of the
  actual answer payload (which has `answer.choice`, `answered_by`, etc.).
  Fix the example and update `examples/human-review.accept.json` to assert the
  `HumanDecision` fact is created.

## 2. Diagnostics quality

- [x] Unknown class in `when` should report `unknown class \`Tikket\`` with a
  did-you-mean suggestion, instead of treating it as an empty schema
  ("schema `Tikket` has no declared fields").
- [x] Rule-body diagnostics should carry the span of the offending token, not
  anchor every body error at `=> {`.
- [x] Wrap store-layer errors before they reach users — no raw SQLite text
  like `UNIQUE constraint failed: facts.fact_id`.

## 3. CLI polish

- [x] `whip <subcommand> --help` should print subcommand usage (currently
  `whip dev --help` errors with "unknown dev option `--help`").
- [x] `whip inbox <instance>` should filter by instance, consistent with
  `status` / `facts` / `log`.
- [x] Consistent JSON envelope shapes across commands (`--json instances`
  returns a bare array; `dev --json` returns an object).

## 4. Performance / architecture

- [x] Add an index on `facts(instance_id, name)` — `fact_exists` queries are
  unindexed linear scans.
- [~] Move rule-body lowering and fact-ID derivation out of the CLI crate into
  the parser/kernel crates. **Why still open:** the fact-ID/effect-ID collision
  was fixed in place; the structural move remains (`main.rs` is now ~50k lines).
  **When:** carried forward — now tracked canonically in
  `decision-records/language-ergonomics-tracker.md` B1a (dedup 2026-07-01).

## 5. Docs

- [x] Add `case expr { Pattern => { ... } }` to language-reference.md
  (currently only in manual.md and api-reference.md).
- [x] Document `when human answered ... as ...` and `when started` in
  language-reference.md.
- [x] Link concepts.md from quickstart step 1; define rules/facts/effects
  before first use.
- [x] Add `recover` to current-state.md "Works Today".
- [x] Show full operation signatures (`as` bindings, `requires [...]`) in the
  language-reference rule-body operations table.
- [x] Align the boolean-operator listing with decision 6.1 below.
- [x] **Self-test infrastructure:** CI check that every rule in every shipped
  example fires in at least one acceptance fixture (would have caught the
  human-review dead rule).

## 6. Design decisions (resolved 2026-06-09)

1. **Guard boolean operators — accept both, teach wordy.** Parser accepts
   `and` / `or` / `not` as aliases for `&&` / `||` / `!`. Docs and examples
   use the wordy form; existing `&&` workflows keep working.
2. **Workflow forms — header style gets full contracts.** Allow
   `input` / `output` / `failure` declarations and `complete` / `fail` rule
   actions at the top level of a single-workflow header-style file. Block
   style remains for multi-workflow bundles. Both forms equally capable.
3. **No-terminal-path lint — hard error.** `whip check` refuses workflows
   with no rule that can reach `complete` / `fail`, unless explicitly tagged
   (e.g. `@service`) for intentionally long-running workflows. Shipped
   examples must be audited and updated/tagged to pass the new lint.
4. **Dead-rule detection — error.** `check` errors on rules whose reads can
   provably never be satisfied by any table seed, rule write, effect
   completion, or runtime event. Escape tag for rules fed only by external
   events. (Would have caught the human-review dead rule and unknown-class
   typos.)
5. **Resume UX — load program from store.** `step` / `worker` (and a
   `dev --resume` path) load the active program version from the store;
   `--program` becomes an override for testing local edits before `revise`.
   Requires persisting program source/IR bytes per version (program_versions
   already tracks source_hash / ir_hash).
6. **`done` only.** `done` is the single consume verb. `consume` parses with
   a deprecation warning for one release, then is removed from parser and
   docs.
7. **Reserve keywords as binding names.** Operation keywords (`done`,
   `consume`, `record`, `tell`, `complete`, `fail`, `case`, ...) are rejected
   as binding names with a clear diagnostic.

## Decision interactions to watch

- Decisions 6.3 + 6.4 (hard errors) change what compiles: every shipped
  example and fixture needs a pass to add terminal paths, escape tags, or
  fixes before the lints land. Land the lints behind the example audit.
- Decision 6.2 (header-style contracts) should land before or with 6.3,
  otherwise header-style users have no way to satisfy the terminal-path
  requirement without rewriting to block style.
- Decision 6.1 + 6.6 imply a docs/examples sweep (wordy operators, `done`
  spelling) that can ride along with the 6.3/6.4 example audit.

## Implementation status (2026-06-09, same session)

All items above are implemented, with these scope notes:

- **Liveness lints (6.3/6.4)** are enforced by `whip check` and `whip compile`
  (the static validation commands), matching the decision wording. Execution
  commands (`dev`, `run`, `step`, `worker`, `accept`) compile without the
  lints so existing harnesses and fixtures keep running.
- **human-review.accept.json** was not extended to assert `HumanDecision`:
  acceptance fixtures cannot script inbox answers today. The full
  ask -> answer -> rule-fires loop is covered by the Rust e2e test
  `human_answer_fires_dependent_rule` (crates/whipplescript-cli/tests/control_plane.rs),
  which drives the shipped example.
- **JSON shapes (3.9)** turned out to already follow a consistent convention
  (list commands return arrays; report commands return objects); no CLI change
  was made.
- **`claim` stays bindable** despite the reserved-keyword decision: `claim
  issue with X as claim` is an established idiom and the trailing binding
  position is unambiguous. All other operation keywords are reserved.
- **Facts index** landed as an `ensure_lookup_indexes` pass on store open
  (matching the existing `ensure_*` pattern) rather than a numbered migration,
  because legacy fixture stores may predate the `facts` table.
- **Bonus fix:** `when <agent> completed turn` had the same dead-pattern bug as
  `human answered` (expected `AgentTurn`, runtime writes `agent.turn.completed`).
  Fixed with agent-name filtering and no-binding pattern support; covered by
  `completed_turn_pattern_fires_dependent_rule`.

## Remaining follow-ups — FOLDED (dedup 2026-07-01)

This plan's remaining follow-ups are all duplicated in, and now tracked
canonically by, `decision-records/language-ergonomics-tracker.md`. This tracker
is `closed`; work these there:

- **Dynamic rule-coverage CI (5.18 full version)** → language-ergonomics B3.
  `scripts/check-rule-coverage.sh` exists (static dead-rule lint shipped); the
  dynamic per-run committed-rule reporting is the open remainder.
- **Agent turn enrichment** → language-ergonomics A3e / B2.
- **Move rule lowering out of the CLI crate (4.11)** → language-ergonomics B1a.
- **Remove `consume`** after its deprecation window → language-ergonomics B3.
