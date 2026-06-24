# 0019: Files Package

Status: accepted design baseline

## Decision

WhippleScript should add `std.files` as the standard package for
capability-scoped file and document I/O.

The package owns:

```text
file store declarations
read / write effects for single files
import / export effects for typed structured formats
bounded turn-scoped file-store grants for agents
format codec contracts and evidence
```

**v0 is scoped to a deterministic storage boundary** (revised after design
review; the first draft was over-scoped). v0 keeps only deterministic,
replay-safe parts:

```text
v0 codecs:      text, markdown, json, jsonl, csv, bytes
v0 operations:  read, write, import, export
v0 paths:       literal/static only
v0 provider:    local
deferred:       xlsx, docx, dynamic Expr paths, non-filesystem providers
```

`import` / `export` depend on a platform **typed fact-batch admission**
primitive (atomic, idempotent, replay-reconstructed admission of N facts from one
validated effect outcome), modeled once at the platform level like signal
admission — not a package-level power.

The detailed source and provider contract is specified in
[`../files.md`](../files.md).

## Rationale

Workflows often need to read and write ordinary files:

```text
CSV exports
Excel workbooks
Markdown notes
plain text logs
Word documents
opaque artifacts
```

JSON ingestion and script capabilities are not enough:

```text
JSON ingestion validates already-structured bytes but does not own storage
std.ingress.file observes arrivals/changes but does not read document content
std.script can call external tools but should not be the normal file I/O path
agent-provider file tools are provider-specific and should not be ambient
```

`std.files` gives WhippleScript a direct, auditable, provider-backed way to move
between external files, typed workflow values, and artifacts.

## Source Shape

The central resource is a file store:

```whip
file store project_files {
  provider local
  root "./"

  allow read ["docs/**", "data/**"]
  allow write ["reports/**"]

  formats [text, markdown, json, jsonl, csv, xlsx, docx, bytes]
}
```

The core operations are:

```whip
read markdown from project_files at "docs/brief.md" as brief

import csv IssueRow from project_files at "data/issues.csv" as imported

write markdown to project_files at "reports/summary.md" {
  body summary.markdown
  mode create
} as written

export xlsx IssueRow to project_files at "reports/issues.xlsx" {
  rows ready_issues
  sheet "Issues"
  mode upsert
} as exported
```

Agent access is explicit and turn-scoped:

```whip
tell analyst
  with access to project_files {
    read ["docs/**", "data/**"]
    write ["reports/**"]
  }
"""markdown
Analyze the input files and write a report.
"""
```

## Boundaries

Core owns:

```text
effect lifecycle
artifacts and evidence
capability/profile enforcement
turn-grant composition point
event log and replay invariants
```

`std.files` owns:

```text
file store resource declarations
file path policy
format codecs
read/write/import/export effect schemas
agent file-access grant schema
file-operation CLI
```

Providers own:

```text
local filesystem access
cloud store APIs
document service APIs
path resolution and provider-specific security
codec implementation details
```

## Relationship To Existing Packages

`std.ingress.file` watches or imports outside file arrival events and emits
typed signals. `std.files` reads and writes file content deliberately.

`std.memory` may learn from file/document artifacts after `std.files` reads
them, but memory does not own file I/O.

`std.script` remains the escape hatch for custom processing. It must not become
the default way to read CSV, Excel, Markdown, or Word documents.

`coerce` can interpret messy text after a file has been read, but deterministic
format parsing belongs to `std.files`.

## Security Contract

File I/O is an exfiltration and prompt-injection boundary. The package must be
fail-closed:

```text
no ambient filesystem access
no reads/writes in guards
all reads/writes are effects
all paths are normalized under declared roots
dynamic paths are checked at runtime
symlink traversal is denied by default or explicitly governed
writes require an explicit create/replace/upsert/append mode
content hashes and codec versions are recorded in evidence
runtime authorization repeats even for checked source
```

An agent may use file tools only through explicit `with access to <file-store>`
grants and only within the effective intersection of the store policy, grant
policy, provider binding, and agent profile.

## Consequences

- `std.files` should be added to the standard package inventory.
- `std.ingress.file` should reference file stores for watch/import providers
  where practical, but remain about outside observations rather than file
  content reads.
- The construct grammar needs file-store declarations (lowering through
  `metadata_only`) and file effect operations (`typed_effect_call`). Turn-access
  grants are metadata on the `agent.tell` effect, not a separate lowering class.
- The platform must model **typed fact-batch admission** before `import` /
  `export` ship. This is a platform prerequisite, tracked alongside the standard
  package ports.
- Implementation is local provider + text/markdown/json/jsonl/csv/bytes with
  literal paths. XLSX, DOCX, dynamic paths, and non-filesystem providers are
  deferred to separate, dedicated designs.

## Open Questions

- Should `mode create` be required explicitly, or should it be the default write
  mode with a check diagnostic?
- Should `read markdown` return a plain body string, a `MarkdownDocument`, or
  both through a common document result shape?
- What is the smallest useful row-source expression for `export`?

Resolved by the design review (no longer open): XLSX formula evaluation and DOCX
rendering are deferred entirely, so neither blocks v0; dynamic-path security is
deferred with literal-only v0 paths; turn-grant and resource lowering are settled
(metadata on `agent.tell`, and `metadata_only`).
