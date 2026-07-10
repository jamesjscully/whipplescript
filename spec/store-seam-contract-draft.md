# Store-seam contract — whip's half (DRAFT for co-authoring)

Status: draft (2026-07-10) — whip-side half of the two-store seam contract
(untie-substrate readiness tracker Phase 5; research note §5's "separate
stores + three seam disciplines", merge rejected). The un-tie side's
boundary posture is GaugeDesk ADR 0080 ("WhippleScript is the runtime
enforcement boundary"); this draft aligns with it and names the items that
need their co-authoring before the contract is ratified. Formal model of
the crossing: `models/maude/seam-crossing.maude` (whip's stack half; the
un-tie side owns a Quint twin per the formal-tool division).

## The two stores, one sentence each

- **The authority's admission log** (gaugewright/gaugedesk): *decisions* —
  what was admitted, under which policy epoch, pointing at whip facts.
- **Whip's event log**: *happenings* — what ran, what changed, what
  evidence accrued; every fact body lives here and only here.

**One-owner-per-fact:** the admission log never copies a whip fact body;
it records a decision plus **pointers** (the referenceable handles below).
Whip never records an authority decision as its own fact; it consumes the
decision as an admitted command and logs the *happening*.

## Referenceable handles (what a decision may point at)

Exposed by `whip handles <instance>` (`whipplescript.handles.v0`) and by
the `whipplescript.host.v1` receipts:

| Handle | Shape | Stability |
|--------|-------|-----------|
| Event position | `(instance_ref, sequence)` | append-only log; a sequence never moves |
| Effect id | `effect_id` | immutable once created; status transitions are events |
| Workspace cut id | `cut_id` (+ `change_id` dual identity) | content-addressed manifest; append-only lineage |
| Evidence ref | `evidence_id` (receipts: `usage_ref`, `guarantee_report_ref`, `workspace_cut_ref`) | immutable evidence rows |
| Policy epoch | `(epoch, envelope_hash, signer)` | the authority's own artifact, echoed back verbatim |

## Jurisdiction table (which whip side-effects cross as admitted commands)

| Whip side-effect | Embedded (governed host) | Standalone whip |
|---|---|---|
| Open / fork an instance | **Admitted command** (`OpenInstanceCommand`, `ForkInstanceCommand`) | whip-internal verb (`whip run`, `whip fork`) |
| Agent turn | **Admitted command** (`StartTurnCommand`; receipt + labeled events back) | whip-internal effect (`tell`), envelope-enforced |
| Human ask / answer | **Admitted crossing both ways** (`LabeledHumanAsk` out, `AnswerHumanAskCommand` in) | whip-internal inbox |
| Workspace writes (file effects, exec import-back) | whip-internal, **witnessed**: the receipt's `workspace_cut_ref` is the runtime's claim; the authority decides *about* it by pointer (e.g. advancement, ADR 0082) | whip-internal, same witnessing |
| Native command execution | whip-internal execution under a **host-admitted capability** (`NativeCommandPolicy`); taints the workspace witness | same, operator-admitted allow-list |
| Provider/model call | whip-internal under the **admitted provider binding** (credentials host-resolved after admission) | standalone resolver fallback |
| External signal injection | **Admitted command** (internal signals refuse external injection — no laundering, H8) | `whip signal`, same refusal |
| Content erasure | whip-internal verb today (`whip branch erase`); **erasure policy is authority-owned** — the admitted-erasure command shape needs the un-tie half (see open items) | operator verb |
| Policy epoch bump | **Authority-owned**; whip consumes the new envelope and cites the epoch in every receipt/guarantee | env-configured envelope |

## Idempotent crossing semantics

Modeled in `seam-crossing.maude` (coverage + bite, gate-registered):

1. **Command id = the dedup key.** Delivery is at-least-once; folding is
   exactly-once. A re-delivered admitted command is absorbed by the
   recorded crossing (whip: effect idempotency keys + `stored_execution`
   receipt replay return the original receipt bit-for-bit).
2. **Admission precedes crossing.** An unadmitted command never folds
   (whip: protocol validation + `require_policy`/`require_governed` refuse
   before any state moves). Refusals are data, not errors.
3. **Receipts are the return crossing.** A receipt/labeled event carries
   pointers (never bodies) and the policy epoch it enforced; the authority
   validates it against its own command (`validate_for`) before admitting
   the decision it supports.
4. **Position-pair cut for backup/handoff.** `whip checkpoint <instance>
   --external-positions <json|@file>` records `(external scope positions,
   workspace cut id)` inside ONE `plane.positions` event in the same
   fenced pass as the cut — a cross-store backup restores both stores to
   that single coherent coordinate. The pair is readable back via
   `whip handles` (`position_pair`).

## Open items for the un-tie side (co-authoring needed before ratification)

- The **Quint twin** of `seam-crossing.maude` over their admission-shell
  semantics (formal-tool division: distributed/lifecycle on their side).
- The **admitted-erasure command** shape (erasure policy is theirs; whip's
  per-blob erasure discharges `HISTORY_PRESERVED` /
  `EXPORTED_COPY_NOT_RECALLED` mechanically and is ready to be driven).
- **Handoff-export format** details (whip's bundle format
  `whipplescript.bundle.v1` + the position pair are the ingredients).
- Ratification: on acceptance each repo records its own half (their ADR,
  our DR), per the research note's cross-repo governance rule.
