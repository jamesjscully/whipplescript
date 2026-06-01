# Documentation Improvement Tracker

Status: completed docs pass

This tracker covers the next product-oriented documentation pass. The target
reader is an AI enthusiast who can program and wants to plug WhippleScript into
agent workflows, try it locally, and understand what works today without first
reading implementation specs.

## Goals

- Make the first 10 minutes clear, satisfying, and honest.
- Keep user-facing docs separate from design trackers where practical.
- Explain concepts before API details.
- Show expected command output for successful paths.
- Make current stability and provider limitations explicit without burying the
  value proposition.
- Add smoke coverage for documented commands so docs do not drift.

## Work Items

| ID | Status | Area | Task | Acceptance |
| --- | --- | --- | --- | --- |
| DOC-001 | [x] | Tutorial | Add `docs/tutorial.md` with a compelling happy path that routes work to an agent, adds review, runs with the fixture provider, and inspects results. | A new user can follow the tutorial from a clean checkout and understand why WhippleScript is useful beyond `minimal-noop`. |
| DOC-002 | [x] | Concepts | Add `docs/concepts.md` explaining workflow, fact, event, rule, effect, agent, provider, worker, `run`/`step`/`worker`/`dev`, fixture provider, and real providers. | README and quickstart can link to one short conceptual primer before the language reference. |
| DOC-003 | [x] | Information architecture | Reduce direct user-doc links into `spec/` by adding user-facing wrappers for plugin authoring, operator guidance, and troubleshooting. | `README.md` and `docs/README.md` primarily point to `docs/` pages; `spec/` remains available as design record. |
| DOC-004 | [x] | Expected output | Add shortened expected output snippets for `whip check`, `whip dev`, `whip facts`, `whip effects`, and `whip trace --check`. | Readers can compare their terminal output to documented success shapes without reading full JSON blobs. |
| DOC-005 | [x] | Examples | Add `docs/examples.md` with a table of examples, what each demonstrates, whether credentials are needed, and suggested next steps. | Users can choose the right example without opening every `.whip` file. |
| DOC-006 | [x] | Current state | Add a clear “what works today” section or page covering local fixture experiments, unstable syntax/API, experimental real providers, planned releases, and production caveats. | Stability language is consistent across README, docs, and relevant spec indexes. |
| DOC-007 | [x] | Troubleshooting | Add `docs/troubleshooting.md` for first-10-minute failures: missing Rust/Cargo, `whip` not on `PATH`, `run` produced no facts, store confusion, missing `--root`, provider credentials, skipped real-provider checks, and install failures. | Common beginner failures have direct, user-facing fixes. |
| DOC-008 | [x] | Providers | Expand `docs/providers.md` into “How to plug this into my agents”: profiles, providers, `tell`, fixture behavior, Codex/Claude/Pi concepts, credentials/config expectations, and experimental limits. | A programmer understands the integration model before attempting real providers. |
| DOC-009 | [x] | Docs testing | Add a docs smoke script that runs the quickstart commands and checks for expected output such as `StartupSeen`. | CI or local readiness can catch stale quickstart commands. |
| DOC-010 | [x] | User-doc polish | Remove or replace `Status: draft` noise in `docs/` with friendlier stability notes where needed. | User-facing pages feel like product docs, not internal trackers. |
| DOC-011 | [x] | Manual | Rework `docs/manual.md` to reference the concepts/tutorial pages and avoid becoming the first required read. | The manual works as a deeper guide after quickstart/tutorial, not the entry point. |
| DOC-012 | [x] | API reference | Keep `docs/api-reference.md` synchronized with `whip help` and current command behavior, including `revise`. | API reference remains factual and command-complete. |

## Suggested Order

1. DOC-002: concepts page.
2. DOC-001: happy path tutorial.
3. DOC-005: examples guide.
4. DOC-007: troubleshooting.
5. DOC-008: provider integration guide.
6. DOC-009: docs smoke script.
7. DOC-003, DOC-006, DOC-010, DOC-011, DOC-012: cleanup and consistency pass.

## Notes

- Keep examples honest about credentials and current implementation limits.
- Prefer fixture-backed flows for first-run documentation.
- Use `spec/` links for design depth, not as the primary user journey.
- When a spec says a subsystem is stable, clarify whether that means
  “implementation tests are stable” rather than “public user-facing API is
  stable.”

## Completion

All tracked work items are complete for this pass. Future docs work should open
a new tracker or append a new section with fresh scope.
