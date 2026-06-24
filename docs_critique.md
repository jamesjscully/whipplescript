# WhippleScript Documentation Critique

Audit date: 2026-06-11

Current status: all issues below have been addressed in the current worktree.
The original findings are preserved, with resolution evidence added here so the
file remains an issue record rather than becoming stale critique.

Standard used: documentation should be correct, progressively layered,
searchable, semantically precise, versioned, runnable, tested, explicit about
edge cases, and separated by audience.

Checks run during this audit:

- `scripts/check-docs-quickstart.sh` passed.
- Local Markdown links in `README.md`, `docs/*.md`, and top-level `spec/*.md`
  resolve.
- `python3 -m mkdocs build --strict` could not run because `mkdocs` is not
  installed in the environment.
- `whip --version` reports `whipplescript 0.1.0`.
- Top-level `whip --help` reports `whipplescript stage-0-skeleton` and omits
  some implemented commands.
- `whip check examples/*.whip` without `--root` fails for the multi-workflow
  examples `revision-parent-child.whip` and
  `revision-validation-approval.whip`.

Resolution checks added or run after fixes:

- `cargo run -q -p whipplescript -- --help` now prints package version plus
  implementation stage and lists `notify`, `otel-export`, `leases`, `ledger`,
  and `counters`.
- `scripts/check-docs-examples.sh` verifies every cataloged example check
  command, including required `--root` flags.
- `scripts/check-docs-snippets.sh` verifies the quickstart, example catalog,
  and complete tutorial workflow.
- `scripts/check-docs-site.sh` builds the MkDocs site in strict mode from
  pinned docs requirements.
- `cargo fmt --all -- --check`, `git diff --check`, local Markdown link scan,
  and the focused CLI/control-plane tests passed.

## Resolved Changes

| Issue | Resolution |
| --- | --- |
| DOC-001 | Added version-scope language to `README.md`, `docs/README.md`, `docs/install.md`, and `docs/current-state.md`; CLI help now prints `version (stage)`. |
| DOC-002 | Added missing commands to top-level CLI help and `docs/api-reference.md`, including usage, stores, JSON behavior, and exit behavior. |
| DOC-003 | Added newer constructs/effect kinds to `docs/api-reference.md`, `docs/language-reference.md`, and `docs/providers.md`. |
| DOC-004 | Added lexical structure, syntax shape, expression precedence/type rules, and matching/commit semantics to `docs/language-reference.md`. |
| DOC-005 | Added command columns to `docs/examples.md` and `scripts/check-docs-examples.sh`. |
| DOC-006 | Added `docs/diagnostics.md` and linked it from troubleshooting, docs home, agent guide, and MkDocs nav. |
| DOC-007 | Added `scripts/check-docs-snippets.sh`, documented code-block conventions and docs verification scripts, and made the tutorial workflow explicitly checked. |
| DOC-008 | Added a `spec/README.md` status legend and made `docs/` explicitly authoritative for user-facing behavior. |
| DOC-009 | Split machine-readable JSON contracts into `docs/json-reference.md` and internal Rust APIs into `docs/rust-api.md`; reduced the CLI page to command/reference links. |
| DOC-010 | Added pinned `docs/requirements.txt`, `scripts/check-docs-site.sh`, MkDocs nav entries, and release-readiness docs checks. |

## DOC-001: Version Contract Is Not Clear Enough

Severity: high

Principle violated: versioned docs; maintenance as part of the product.

Evidence:

- `README.md` says WhippleScript is pre-1.0.
- `docs/current-state.md` says stable-enough does not mean a semver promise.
- `whip --version` reports `0.1.0`.
- Top-level `whip --help` reports `stage-0-skeleton`.
- Install docs use `releases/latest`, while the docs themselves are not
  versioned by release.

Why this matters:

A reader cannot tell whether a page describes the current checkout, the latest
GitHub release, v0.1.0, or a moving pre-1.0 development branch.

Requirements to close:

- Put a clear doc-version banner on the docs home and generated site.
- State whether docs track `main`, a release tag, or both.
- Align CLI stage naming with release/version wording, or explain the stage.
- Add a short compatibility note for `latest` installer users versus local
  checkout users.

## DOC-002: CLI Reference Omits Implemented Commands

Severity: high

Principle violated: correctness; searchability.

Evidence:

- `docs/api-reference.md` lists the current command set as:
  `check, compile, run, revise, step, worker, dev, accept, instances, status,
  log, facts, effects, runs, artifacts, inbox, items, evidence, diagnostics,
  trace, pause, resume, cancel, retry, recover, doctor`.
- `crates/whipplescript-cli/src/main.rs` implements and has individual help
  for `notify`, `otel-export`, `leases`, `ledger`, and `counters`.
- Top-level `whip --help` also omits those commands, even though
  `whip help notify`, `whip help otel-export`, `whip help leases`,
  `whip help ledger`, and `whip help counters` work.

Why this matters:

The docs and top-level help both hide live surface area. Users cannot discover
documented language features such as event ingress, coordination resources, or
OpenTelemetry export from the CLI/API reference.

Requirements to close:

- Add the missing commands to `docs/api-reference.md`.
- Add usage, JSON behavior, exit behavior, store behavior, and examples for
  each command.
- Fix top-level `whip --help` so the executable and docs agree.

## DOC-003: Newer Language Features Are Not Integrated Into The Reference

Severity: high

Principle violated: semantic precision; consistency.

Evidence:

- `docs/language-reference.md` has brief sections for sum types, typed JSON
  exec ingestion, scheduled time, events, coordination resources, and
  observability near the end.
- The compact reference tables in `docs/api-reference.md` do not include
  top-level `event`, `lease`, `ledger`, `counter`, or `harness`.
- The rule-body operation tables omit `notify`, lease acquire/release,
  ledger append, and counter consume.
- `docs/providers.md` omits effect kinds such as `event.notify`,
  `lease.acquire`, `lease.release`, `ledger.append`, and `counter.consume`.

Why this matters:

The docs read like an older reference plus appended new features. A user can
find the new features only if they already know what to search for, and the
effect model is incomplete across pages.

Requirements to close:

- Integrate each implemented construct into the main declaration/body/effect
  tables.
- Add complete syntax, semantics, examples, runtime effects, failure behavior,
  inspection commands, and cross-links for each feature.
- Make `docs/language-reference.md`, `docs/api-reference.md`, and
  `docs/providers.md` agree on the effect taxonomy.

## DOC-004: The Language Reference Is Not Yet Spec-Precise

Severity: high

Principle violated: semantic precision.

Evidence:

- `docs/language-reference.md` is useful, but it lacks a true lexical/syntax
  section: identifiers, comments, string forms, multiline delimiter rules,
  whitespace/newline significance, reserved words, object/array literal
  grammar, and complete declaration grammar.
- Expression documentation lists supported forms but not operator precedence,
  associativity, short-circuit behavior, evaluation errors, or type coercion
  rules.
- Rule semantics describe atomic commits and effect scoping, but the exact
  matching algorithm, fan-out behavior, conflict behavior, fact key semantics,
  and source-to-IR lowering rules are spread across `spec/`.

Why this matters:

Reference documentation for a language must let an implementer or power user
answer exact semantic questions without reading Rust code or design trackers.

Requirements to close:

- Add a grammar or grammar-like appendix.
- Add a lexical structure section.
- Add an expression semantics table with precedence and result typing.
- Move or summarize stable semantics from `spec/` into the user-facing
  reference, with links back to design rationale only where useful.

## DOC-005: Example Catalog Overstates Copy-Paste Simplicity

Severity: medium

Principle violated: runnable examples; executable clarity.

Evidence:

- `docs/examples.md` says all examples below check with no credentials.
- Running `whip check examples/*.whip` without roots fails for:
  `examples/revision-parent-child.whip` and
  `examples/revision-validation-approval.whip`.
- The failure is legitimate because those files contain multiple workflows and
  require `--root`.

Why this matters:

The examples are probably correct, but the catalog does not state the command
needed for every listed example. This breaks the copy-run-debug loop.

Requirements to close:

- Add a command column or per-example notes for required `--root` values.
- Add a checked script that validates every catalog command, not just selected
  examples.
- Prefer local relative links in docs intended for checkout use, with website
  rendering handling repository links separately if needed.

## DOC-006: Error Documentation Is Too Thin

Severity: medium

Principle violated: honest treatment of edge cases; searchability.

Evidence:

- `docs/troubleshooting.md` covers a small number of common problems.
- `examples/invalid/*.diagnostics` contains many targeted compiler errors, but
  the user-facing docs do not index them.
- Durable runtime diagnostics are documented as JSON shapes, but there is no
  diagnostic catalog by code/category, cause, and repair.

Why this matters:

Good language docs let a user paste an error message into search and reach a
page explaining what happened and how to fix it.

Requirements to close:

- Add a diagnostic/error guide.
- Group parse, type/check, liveness, provider/runtime, revision, assertion,
  and fixture errors separately.
- For each common diagnostic, show broken code, explanation, and fixed code.
- Link invalid examples to the relevant diagnostic guide entries.

## DOC-007: Runnable Snippets Are Not Systematically Tested

Severity: medium

Principle violated: runnable, tested examples.

Evidence:

- `scripts/check-docs-quickstart.sh` tests the quickstart path.
- There is no equivalent extraction/check for code snippets in
  `docs/tutorial.md`, `docs/manual.md`, `docs/language-reference.md`, or
  `docs/providers.md`.
- Several pages contain partial snippets by design, but they are not labeled
  consistently as partial or complete.

Why this matters:

The docs rely heavily on examples. Without snippet tests or explicit labels,
small syntax drift will quietly undermine trust.

Requirements to close:

- Mark every code block as complete, partial, expected-error, or illustrative.
- Add a docs snippet test harness for complete `.whip` examples and shell
  commands that can safely run.
- Keep partial snippets short and link to a complete checked example nearby.

## DOC-008: Audience Separation Is Better, But Still Leaky

Severity: medium

Principle violated: audience separation; progressive disclosure.

Evidence:

- `docs/` is intended for users and agents.
- `spec/` contains overlapping quickstart, examples, troubleshooting,
  operator guide, language sketch, semantics, and type-system material.
- `docs/language-reference.md` links into design records for several features,
  while stable user-facing details remain only in `spec/`.

Why this matters:

Design history is valuable, but users looking for authoritative behavior can
end up in trackers with mixed statuses and older context.

Requirements to close:

- For each stable behavior, make `docs/` authoritative.
- Keep `spec/` for rationale, history, and future work.
- Add a status legend to `spec/README.md` explaining which documents are
  normative, historical, draft, or tracker-only.

## DOC-009: API Reference Mixes Public And Internal Contracts

Severity: medium

Principle violated: audience separation; correctness.

Evidence:

- `docs/api-reference.md` includes CLI commands, JSON output, status values,
  event types, provider config JSON, artifact manifests, and Rust crate APIs.
- It says Rust APIs are internal-stability APIs, but they sit in the same
  reference as user-facing CLI and JSON contracts.
- JSON shapes are described with "field sets may grow", but per-schema
  stability expectations and required/optional fields are incomplete.

Why this matters:

Operators, integration authors, and Rust contributors need different contract
levels. Mixing them makes it harder to know what can be depended on.

Requirements to close:

- Split CLI reference, JSON/report schemas, provider config schemas, and Rust
  internal APIs into separate pages.
- For every JSON schema, identify required fields, optional fields, stability
  promise, and schema id/version.
- Keep internal Rust APIs explicitly out of the public user contract.

## DOC-010: Tooling And Site Build Requirements Are Under-Specified

Severity: low

Principle violated: maintenance as part of the product.

Evidence:

- `docs/README.md` says to install `mkdocs` and run `mkdocs serve`.
- `mkdocs.yml` uses strict mode.
- `python3 -m mkdocs build --strict` could not run in this environment because
  `mkdocs` is not installed.

Why this matters:

The site build is part of documentation quality, but contributors do not have
one canonical command that provisions and validates it.

Requirements to close:

- Add a repo script such as `scripts/check-docs-site.sh`.
- Pin MkDocs dependencies in a lockable location.
- Include the site build in release/readiness checks if the website is a
  supported doc surface.
