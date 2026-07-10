# DR-0036 — Per-turn workspace cut and dynamic guarantee report

Status: accepted + built (2026-07-10; drafted as the cross-repo dependency
named by GaugeWright ADR 0082 stage 3, built the same day — witness model
`models/maude/turn-witness.maude`; resolver-witnessed cut segments aggregated
into `host.turn.workspace_cut` evidence + a populated `workspace_cut_ref`;
envelope `guarantee` declarations evaluated into the report's `dynamic`
section as held / **violated** / not-evaluated — "violated" added to the §2
vocabulary because an evaluated-and-failed guarantee is neither held nor
not-evaluated, and the advancement consumer needs to see it; test
`receipt_workspace_cut_and_dynamic_guarantees_from_witnessed_turn`).
Residual: `no_tainted_reads:<label-class>` reports not-evaluated until
label-class read tainting is witnessed. Cross-refs:
DR-0024 (owned brokered harness — the evidence producer), DR-0027/0028
(authority envelope / governance semantics the dynamic guarantees cite),
DR-0034 (managed vs. delegated — both classes must satisfy or honestly decline
this surface), GaugeWright ADR 0080 (the seam contract this extends) and
ADR 0082 (the consumer).

## Problem

The host protocol's `TurnReceipt` promises more evidence than the runtime
delivers today:

1. **`workspace_cut_ref` is never populated.** The field exists in the
   protocol, but `finish_execution` hardcodes `None`. No consumer can learn,
   from the receipt, what a turn did to its workspace.
2. **The guarantee report is static.** `host.turn.guarantee` records the same
   fixed list of admission-mechanics guarantees every turn
   (`signed_policy_identity_verified`, `package_ifc_checked_under_verified_envelope`,
   …) plus an echo of the command's *granted* resources. It certifies that
   admission was done right; it certifies nothing **observed during the run**.

The read side does not share this gap: `output.flow_signature` already
certifies, per turn, which read handles flowed into each output field, and
hosts consume it.

Why it matters now: GaugeWright ADR 0082 gates merge **advancement** ("does
this turn's work auto-advance to `main` or hold for human review") on turn
facts. On a local desktop the host owns the workspace git repo and can derive
write facts itself — but that truth is not placement-neutral. On a remote or
managed placement (the DO runtime, delegated harnesses) the host does *not*
own the disk, and the receipt is the only possible authority. Per the seam
contract (GaugeWright ADR 0080), local embedding is an optimization of the
same command/event/receipt contract — so facts a host is expected to act on
belong **in the receipt's evidence**, not in host-side derivation.

## Decision

### 1. Populate `workspace_cut_ref`

Every terminal receipt for a turn that ran with a workspace references a
**workspace-cut evidence item**: the runtime's own claim of the turn's
workspace delta — paths added / modified / deleted, with content references
(hashes/handles, never inline bodies), labeled like every other evidence
item. A turn with no workspace delta references an explicitly-empty cut
(distinguishable from "not reported"). A harness class that cannot witness
the delta (a delegated foreign runtime without workspace mediation) must
**decline honestly** — `workspace_cut_ref` absent — rather than fabricate;
consumers treat absence as "unwitnessed", never as "no changes".

### 2. A dynamic section in the guarantee report

The guarantee report keeps its static admission set and gains a **dynamic
section**: named guarantees evaluated **per turn** under the cited policy
epoch, each either *held* or *not-evaluated* (never silently omitted).
Initial vocabulary, chosen for the advancement consumer but deliberately
generic:

- `writes_within:<scope>` — every write in the workspace cut falls inside
  the named scope (a path family declared in the envelope).
- `no_reads_beyond_grant` — every read in the flow signature is within the
  command's granted resources.
- `no_tainted_reads:<label-class>` — no read carried the named label class.

The envelope declares which dynamic guarantees a turn must evaluate (they are
policy, and WhippleScript owns the policy language — GaugeWright ADR 0080);
the report cites the policy epoch it evaluated under. Consumers match
guarantee **names**; they never re-evaluate semantics.

### 3. What this record does *not* place here

The advancement gate itself — whether a host advances its `main` on these
facts — is the host's governance decision (GaugeWright ADR 0082 keeps it in
GaugeDesk). WhippleScript certifies facts; it does not decide merges.

## Consequences

- The receipt becomes sufficient for placement-neutral write-side policy;
  hosts on remote placements stop being blind and hosts on local placements
  can cross-check their git truth against the runtime's claim.
- The static/dynamic split keeps the existing report backward-compatible:
  present consumers read the static list unchanged.
- DR-0034's class split gets one more concrete obligation: managed harnesses
  witness the cut natively; delegated harnesses either mediate the workspace
  (and witness) or decline, per their wire contract (DR-0035).
