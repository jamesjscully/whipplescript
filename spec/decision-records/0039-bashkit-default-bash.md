# DR-0039 — Bashkit is the default governed bash on native and DO placements

Status: accepted (Jack, 2026-07-13). Implementation in progress as part of the
GaugeDesk Durable Object placement vertical. Research and spike evidence:
`spec/in-isolate-bash-design-note.md`. Amends DR-0033 Decision 7 by separating
virtual `bash` from real subprocess execution; confirms the direction recorded
when the DO parity tracker dropped P5/P6.

## Problem

The native managed harness currently exposes a deliberately restricted wrapper
around an opaque OS shell, while the Durable Object tool executor exposes no
`bash`. Treating DO bash as an HTTP sidecar would make ordinary `grep`/`sed`/
`awk`/`jq`-class work pay a container round trip and would give native and DO
placements different model-facing semantics. Treating Bashkit as merely a DO
optimization would preserve the same split.

At the same time, many capabilities are not bash: compilers, real language
runtimes, browsers, large-file transforms, service APIs, and specialized tools
need explicit remote execution or service boundaries. A general capability
broker should not be disguised as shell escalation.

## Decision

1. **Bashkit is WhippleScript's default `bash` implementation on both native and
   Durable Object managed harnesses.** The tool schema and semantics are owned by
   WhippleScript and placement-neutral. Native does not silently prefer the OS
   shell when Bashkit is available.

2. **Bashkit runs in-process over WhippleScript's governed workspace VFS.** File
   reads and writes cross the same labeled store boundary as first-class file
   tools. No fork/exec, ambient filesystem, or ambient network authority is
   implied by the name `bash`.

3. **Non-bash external capabilities are brokered explicitly.** Real builds,
   compilers, browsers, arbitrary binaries, and other external systems become
   typed effects routed through the existing sans-IO HTTP machinery and
   capability/provider registry. They are not an automatic fallback path hidden
   inside the `bash` tool.

4. **Unsupported Bashkit behavior fails honestly at the bash boundary.** An
   unavailable or incompatible command must not return a successful stub.
   Specific incompatibilities are handled when observed: add or correct a
   builtin, fix the compatibility adapter, or introduce a separately governed
   brokered capability when the operation is genuinely not bash.

5. **Bashkit remains behind a WhippleScript-owned adapter.** The dependency is
   pinned behind `WhipShell`; its VFS, resource limits, clock/random inputs, and
   command hooks are controlled by WhippleScript. Dependency replacement does
   not change the model-facing tool contract.

6. **The initial DO compatibility work is a port, not a redesign.** The spike
   proved the interpreter and custom VFS under `wasm32-wasip1`, and the composite
   host stayed well below the Worker size limit. The known
   `wasm32-unknown-unknown` clock/feature-gating gap is implementation work for
   the Cloudflare target. It is not evidence for a brokered-bash architecture.

## Consequences

- Native and DO conformance tests run the same bash scripts, VFS fixtures,
  limits, and expected deltas.
- `command.run` means governed virtual bash for managed harness packages.
  Arbitrary native subprocess execution, where retained for compatibility or a
  delegated harness, is a different placement capability and cannot satisfy
  this contract by name alone.
- The DO placement sprint includes Bashkit integration, its Cloudflare clock
  adapter, store-backed VFS, limits, and common-host tests.
- Capability brokering remains a first-class follow-on within the same sprint;
  it is driven by actual non-bash capability requirements rather than a generic
  shell escape hatch.

## Rejected

- **Bashkit only on DO, OS shell by default on native.** Two default bash
  semantics would make placement selection observable to packages and agents.
- **Broker every bash call to a container.** This makes the common file/text hot
  path slow and needlessly enlarges the external trust boundary.
- **Automatically escalate unknown Bashkit commands to real exec.** It hides a
  materially different authority and cost boundary behind one tool call.
- **Wait for perfect bash compatibility before adoption.** The spike already
  establishes the architectural fit; concrete incompatibilities can be fixed
  under the honest-failure rule.
