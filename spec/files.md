# `std.files`: capability-scoped file and document I/O

Status: spec drafted 2026-06-14 from package design discussion
([`decision-records/0019-files-package.md`](decision-records/0019-files-package.md)).
Stage: spec -> modeling -> implementation + testing -> review.

> **Reserved-class prerequisites:** the `file store` declaration is `metadata_only`
> (already authorable). `read`/`write`/`import`/`export` lower through
> `typed_effect_call` — see the Construct Graph Contract below (each requires
> `Capability<file.read>` etc. *and* declares `lowering class: typed_effect_call`;
> the "capability-scoped" framing is the capability *requirement*, not the lowering
> class). The promotion EXECUTED as attribution keeping the `file.*` kind strings
> (spec/std-files.md E4, slice F5): the embedded std.files manifest registers the
> four `typed_effect_call` construct rows and the admission rows behind them,
> lowering keeps emitting the same builtin kinds, and durable history is
> untouched — no rekey, no Route-B re-home. The **typed fact-batch admission**
> primitive for `import`/`export` shipped with the import vertical.

## Implementation Status

| Operation | Status (2026-06-16) |
| --- | --- |
| `file store` declaration | implemented (parses, lowers, snapshots) |
| `read text` / `read markdown` (body codecs) | implemented end-to-end (runtime) |
| `read json` / `jsonl` / `csv` (structured) | rejected at parse time — these are the `import` surface |
| `read bytes` (artifact) | rejected at parse time — deferred read codec |
| `write text` / `write markdown` (body codecs) | implemented end-to-end (runtime) |
| `write json` / `csv` (structured) | rejected at parse time — these are the `export` surface |
| `import jsonl` / `import json` / `import csv` | implemented end-to-end (runtime) — decode + per-row validate + atomic typed fact-batch admission |
| `export jsonl` / `export json` / `export csv` | implemented end-to-end (runtime) — collection-valued projection (`where`-filtered) serialized with the write mode policy |

**Historical note (Route B, closed by std.files slice F2/F5):** v0 originally
took Route B — builtin `file.*` effect kinds the worker resolved directly, with
no provider registration and no package manifest, bypassing the admission gate.
The typed_effect_call promotion later executed as **attribution keeping the
`file.*` kind strings** (spec/std-files.md E4): the embedded std.files manifest
registers the contracts/constructs/capability/provider/binding rows, the
builtin-kind admission bypass was deleted from the native `policy_block_on`,
and lowering still emits the same kinds (idempotency keys hash the kind string,
so durable history is untouched).

**The capability layer is live** (slice F2): each `file.*` contract requires
exactly its own kind string (capability id == effect kind, M3), the embedded
manifest seeds capability/provider/binding rows at store init, and an unbound
`file.*` kind blocks loudly as `blocked_by_capability` (a stale store fails
loud, never hangs). The `file store` declaration's `root` and `allow read/write
[...]` globs remain the per-store path boundary on top of that gate, and the
`tell ... with access to` turn grant is enforced as the store-policy
intersection (Q3).

**What v0 `read`/`write` *do* enforce** (matching Security And Policy below): both
are effects (never run in guards/checks), require a declared `file store`, refuse
paths that are absolute or use `..` to climb out of the store root, and — when
the store declares an `allow read`/`allow write [...]` list — require the path to
match one of those globs (an absent list means any path inside the root). A
denied path is refused before any disk access, settling as `file.read.failed` /
`file.write.failed`.
A successful read settles `file.read.completed`, carrying the decoded content as
its binding value so `after <binding> succeeds as r` can read `r.content`. A
`write` requires an explicit `mode` (no silent overwrite); the mode is enforced
against the on-disk state (`create` fails if the file exists, `replace` fails if
it does not), a violation is an ordinary `file.write.failed` routed to `after w
fails`, and a success settles `file.write.completed` with the byte count and a
content hash. The `body` expression resolves at commit against the (after-)context,
so `after r succeeds as v { write … { body v.content … } }` writes the read body.

## Framing

**Files are an external resource boundary.**

WhippleScript should support ordinary file and document workflows, but not by
giving workflows or agents ambient filesystem access. `std.files` declares named
file stores, gates paths through provider capabilities, and exposes deterministic
format codecs as ordinary effects.

The package answers:

```text
which store may this workflow touch?
which paths may it read or write?
which formats can be parsed or rendered?
what content hash, artifact, and evidence prove what happened?
```

It does not own file arrival events, model-backed document understanding, or
process execution. Those compose through `std.ingress`, `coerce`, and
`std.script`.

## v0 Scope

`std.files` is deliberately a *storage and path boundary* first. v0 keeps only
the parts whose behavior is deterministic and replay-safe, and defers the parts
that mix layout, library-version drift, or model interpretation:

```text
v0 codecs:      text, markdown, json, jsonl, csv, bytes   (all deterministic)
v0 operations:  read, write, import, export
v0 paths:       literal or dynamic `at <Expr>`; runtime authorization is the
                authority (containment + globs + canonicalized symlink
                re-check), literal paths additionally checked at compile time
v0 provider:    local (the `provider` clause default; unknown providers are a
                check error)
```

Deferred to a later, separately-designed pass (see Deferred Scope):

```text
docx and xlsx codecs        (version-stability + replay determinism design)
full dynamic-path security engine   (hostile-provider canonicalization design;
                                     dynamic paths themselves are accepted)
non-filesystem providers     (S3/GitHub/Drive/SharePoint path-namespace contract)
```

Platform prerequisite: `import` / `export` move typed rows between files and
workflow facts, which requires the platform **typed fact-batch admission**
primitive (atomic, idempotent, replay-reconstructed admission of N facts from one
validated effect outcome). That primitive is modeled once at the platform level —
like signal admission — and `std.files` rides on it. It is not a package-level
power. `import`/`export` are not accepted until that primitive exists.

## Source Surface

### File Store Declaration

```whip
use std.files

file store project_files {
  provider local
  root "./"

  allow read ["docs/**", "data/**"]
  allow write ["reports/**", "out/**"]

  formats [text, markdown, json, jsonl, csv, bytes]
}
```

A `file store` is a named resource. The local provider resolves paths relative
to `root`; future providers may map the same package surface to S3, GitHub
contents, Google Drive, SharePoint, or another document store.

The store declaration is a policy boundary. It does not read files, grant agent
filesystem access, or watch for file changes by itself.

### Reading Files

`read` loads one file/document into a typed value or artifact:

```whip
read text from project_files at "docs/notes.txt" as notes
read markdown from project_files at "docs/brief.md" as brief
```

v0 `read` decodes only the `text` and `markdown` body codecs (both UTF-8 content
bodies). Structured codecs (`json`/`jsonl`/`csv`) are the typed-row `import`
surface, and `read bytes` (an artifact with a content hash) is a deferred read
codec; the parser rejects them from `read` with a diagnostic. For structured
content today, `read text` then `coerce`.

Meaning:

```text
read path from a declared file store
validate the path against the store's read policy
decode the requested format deterministically
return a typed value/artifact with content hash and evidence
```

`read` is an effect. It never runs in guards and never reads during static
checking.

### Importing Structured Data

`import` turns a structured file into typed workflow facts:

```whip
class IssueRow {
  title string
  priority string
}

import csv IssueRow from project_files at "data/issues.csv" as imported
```

JSON/JSONL import reuses the deterministic JSON ingestion rules:

```whip
import jsonl IssueRow from project_files at "data/issues.jsonl" as imported
```

`import` is all-or-nothing:

```text
every row/item validates -> record typed facts and complete the effect
any row/item fails       -> fail the effect and record no partial facts
```

The success output is an import result such as row count, artifact refs, and
hashes. The rows themselves become ordinary typed facts, so downstream rules use
normal per-fact fan-out.

Admitting N rows as N facts is the platform **typed fact-batch admission**
primitive (see v0 Scope), not a `std.files`-specific power: the validated effect
outcome admits the whole batch atomically, each fact carries a derived
idempotency key, and replay reconstructs the same facts without re-reading the
file. `import` is gated on that primitive being modeled.

> **v0 status:** `import jsonl` and `import json` run end-to-end. The runtime
> decodes rows (jsonl = one JSON object per line; json = a top-level array),
> validates each against the row schema's required (non-optional, non-literal)
> fields, and admits the whole batch atomically as typed `<Schema>` facts via
> `SqliteStore::admit_fact_batch` — so `when <Schema>` rules fan out over the
> rows. Any invalid row fails the effect and admits nothing. Per-row keys use the
> row index by default; if the row schema marks a field `@key`, that field's value
> keys the row instead (`H(effect_key, natural_key)`) and is recorded as the
> fact's key. `import csv` maps a header row over each record (values decode as
> strings; quoted fields may contain commas, `""` escapes a quote); v0 assumes one
> record per line.

### Writing Files

`write` renders one value or artifact to a file:

```whip
write markdown to project_files at "reports/summary.md" {
  body summary.markdown
  mode create
} as written
```

Text and markdown writes can render body text directly:

```whip
write text to project_files at "reports/notes.txt" {
  body notes.body
  mode replace
} as written
```

Writes must declare write intent. Initial modes:

```text
create   fail if the file already exists
replace  fail if the file does not already exist
upsert   create or replace
append   append only, for formats where append is deterministic
```

No silent overwrite. The mode is **required** — omitting it is a check error
(v0 chose the fail-closed option over a silent `create` default). v0 `write`
renders the `text`/`markdown` body codecs; structured `json`/`csv` rendering is
`export` (deferred) and is rejected from `write` with a diagnostic.

### Exporting Structured Data

`export` writes typed rows/items to a structured file:

```whip
export csv IssueRow to project_files at "reports/issues.csv" {
  rows ready_issues
  mode create
} as exported
```

The row source must be a typed collection, projection result, or value whose
element type matches the exported schema. There is no ad hoc string rendering
for structured formats.

> **v0 status:** `export jsonl`/`json`/`csv` run end-to-end. The row source is a
> **collection-valued projection** ([DR-0022](decision-records/0022-collection-valued-projections.md)):
> `export <fmt> <Schema> to <store> at <path> { [where <pred>] mode <mode> } as <r>`
> serializes the `<Schema>` facts (optionally `where`-filtered), in the store's
> deterministic `(name, key)` order, with the same `mode` policy as `write` (no
> silent overwrite) and the same root + `allow write` boundary. csv emits a header
> from the schema's field order then one record per row. Settles
> `file.export.completed` with the row count + content hash. The general collection
> value is exposed only in the `export { … }` clause in v0 (per DR-0022); the
> `rows <named-collection>` surface above is the future general form.

### Agent File Access

Agent file access uses the same turn-grant shape as memory access:

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

The grant is a narrowing of the file store policy:

```text
store allows read ["docs/**", "data/**"]
turn grants read ["docs/**"]
effective grant is read ["docs/**"]
```

An agent without an explicit file-store grant has no `std.files` access through
the workflow. Provider-native filesystem tools may exist, but those remain
agent-provider profile policy and must not be treated as an implicit fallback
for this package.

## Format Scope

v0 codecs (all deterministic and replay-safe):

```text
text      UTF-8 text body
markdown  Markdown body plus optional metadata view
json      deterministic typed JSON value import/export
jsonl     deterministic typed row stream import/export
csv       typed row import/export with deterministic header mapping
bytes     opaque artifact bytes with content hash
```

## Deferred Scope

These are intentionally out of v0 and each needs its own design pass; they are
not "drafted but unimplemented" — they are deliberately excluded:

```text
xlsx      typed rows with sheet/table clauses
docx      text/markdown/document views and rendering
pdf       extraction/rendering
images / OCR
presentation decks
cloud-office-native collaboration semantics
complex spreadsheet formula recalculation
dynamic `at <Expr>` paths      (v0 accepts only literal paths)
non-filesystem providers       (S3 / GitHub / Drive / SharePoint)
```

XLSX and DOCX are deferred specifically because their decoders are
library-version dependent, so the same bytes can decode differently across
versions — which would break the "decode deterministically / replay does not
re-read the filesystem" contract. They deserve a later provider-specific design
that pins codec version and defines replay-against-recorded-output semantics.
PDF, OCR, and rich office collaboration additionally mix extraction policy,
layout, permissions, and model-backed interpretation.

## Relationship To Other Packages

`std.ingress.file` observes outside file arrivals or changes and emits typed
signals. `std.files` deliberately reads and writes file content.

Example composition:

```whip
use std.ingress
use std.files

signal files.received {
  path string
}

file store project_files {
  provider local
  root "./"
  allow read ["data/**"]
  formats [csv]
}

source file as inbox {
  store project_files
  watch "data/*.csv"

  observe as file
  emit files.received {
    path file.path
  }
}

rule import_received_file
  when files.received as file
=> {
  import csv IssueRow from project_files at file.path as imported
}
```

Other relationships:

```text
std.memory   may learn from file/document artifacts, but does not own file I/O
std.script   remains an escape hatch, not the ordinary file I/O path
coerce       can interpret messy text after std.files reads it
core artifacts store durable bytes, hashes, render outputs, and evidence refs
```

## Security And Policy

File I/O is a prompt-injection and data-exfiltration boundary. The package must
fail closed:

```text
no ambient filesystem access
no reads or writes in guards
every operation is an effect
every operation requires a declared file store
every operation checks read/write policy at runtime
paths may be literal or dynamic `at <Expr>`; runtime authorization is the
  authority, and a LITERAL path is additionally validated against the store's
  allow globs at compile time
absolute paths are denied unless a provider explicitly supports them
.. traversal is denied
symlink escape is denied: the path is canonicalized after the policy match and
  containment is re-checked against the canonicalized store root before the
  operation (fail-closed, no disk content touched)
writes require create/replace/upsert/append mode
content hashes are recorded for reads and writes
large or sensitive artifacts obey artifact retention/redaction policy
```

The runtime must repeat authorization even if a source was checked elsewhere.
Compiled source is not authority.

Dynamic `at <Expr>` paths are ACCEPTED (decided posture, spec/std-files.md
"Dynamic `at <Expr>` path security"): runtime authorization — root containment,
`..`/absolute denial, allow-glob match, and the canonicalize-and-recheck symlink
guard — runs per value before any disk access and is the authority. Literal
paths additionally get the compile-time policy check, restoring static
auditability without banning the dynamic form. The FULL dynamic-path security
engine (canonicalization contract for hostile providers, per-value
glob-intersection formalization) stays deferred until the first non-filesystem
provider makes it load-bearing.

## Construct Graph Contract

`std.files` uses ordinary construct families and the settled lowering-class
taxonomy (see the catalog in
[`construct-lowering-preservation.md`](construct-lowering-preservation.md#lowering-class-catalog)).
v0 paths are literal string clauses, not arbitrary `Expr`.

### `file store`

```text
family: declaration_block
shape: file store <name> { <clauses> }
provides: Resource<FileStore>
requires: provider kind, root/ref policy, path policy, format declarations
lowering class: metadata_only (resource is a registry/capability identity,
  not a lowering-emitted runtime object)
```

### `read`

```text
family: effect_operation
shape: read <format> from <store: FileStoreRef> at <path: StringLiteral> [block] as <binding>
requires: Resource<FileStore>
requires: Capability<file.read>
provides: EffectHandle<FileReadResult<Format>>
lowering class: typed_effect_call
```

### `import`

```text
family: effect_operation
shape: import <format> <schema> from <store: FileStoreRef> at <path: StringLiteral> [block] as <binding>
requires: Resource<FileStore>
requires: Schema<T>
requires: Capability<file.import>
provides: EffectHandle<FileImportResult<T>>
admits: a typed fact batch of T on success, via the platform typed
  fact-batch admission primitive (not a package-level fact write)
lowering class: typed_effect_call
prerequisite: typed fact-batch admission (see v0 Scope)
```

### `write`

```text
family: effect_operation
shape: write <format> to <store: FileStoreRef> at <path: StringLiteral> <block> as <binding>
requires: Resource<FileStore>
requires: Value<body | artifact>
requires: Capability<file.write>
provides: EffectHandle<FileWriteResult>
lowering class: typed_effect_call
```

### `export`

```text
family: effect_operation
shape: export <format> <schema> to <store: FileStoreRef> at <path: StringLiteral> <block> as <binding>
requires: Resource<FileStore>
requires: Schema<T>
requires: Value<rows of T>
requires: Capability<file.export>
provides: EffectHandle<FileExportResult>
lowering class: typed_effect_call
```

### Turn Access Grant

Turn-access grants are **not a lowering class**. A `with access to <file-store>
{ … }` clause is authority-narrowing metadata on the `agent.tell` effect
(Proposal A): it is validated as required ports on the `tell` node and lowered as
bounded sub-authority fields on that effect; in-turn file-tool calls are recorded
as evidence, not durable child effects.

```text
shape: with access to <file-store> { read [...] write [...] }
appears in: tell clauses
requires: Resource<FileStore>
requires: agent/provider capability to expose bounded file tools
provides: TurnGrant<FileStoreAccess>
lowering: metadata on the agent.tell effect (no separate lowering class)
```

The checker must reject a file-store grant if the target agent/profile/provider
cannot expose bounded file tools while preserving the ordinary `agent.tell`
lifecycle and evidence.

## Capabilities

Package capability ids EQUAL effect kinds (M3 id==kind, spec/std-files.md
"Spec amendments" 1):

```text
file.read
file.write
file.import
file.export
```

There is no `files.turn_access` capability: turn grants are harness-plane
authority-narrowing metadata on `agent.tell` (the third enforcement plane),
recorded as evidence-metadata — not a capability id.

Provider bindings may further constrain stores, paths, formats, maximum sizes,
artifact retention, and write modes.

## Evidence

Every successful or failed operation should record:

```text
store
provider
normalized path
requested path
format
operation
content hash for bytes read or written, when available
size
codec version
artifact refs
write mode and previous hash for writes
validation diagnostics for import/export
redaction/retention decisions
```

This is what lets a trace answer "which file did this workflow read or write?"
without replay reading the filesystem again.

## Non-Goals

- No ambient file access for workflows or agents.
- No hidden fallback through `std.script`.
- No model-backed extraction inside file codecs.
- No filesystem reads during checks, guards, or replay.
- No dynamic-path security ENGINE in v1 — dynamic `at <Expr>` paths are
  accepted under runtime authorization (see Security And Policy); the hostile-
  provider canonicalization/glob-intersection design is deferred to the first
  non-filesystem provider.
- No XLSX or DOCX codecs in v0 (deferred; see Deferred Scope).
- No package-level fact writes — `import` rides on the platform fact-batch
  admission primitive.
- No cloud-drive collaboration semantics in the first pass.
