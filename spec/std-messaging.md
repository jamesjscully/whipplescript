# `std.messaging` — concrete package design (v1)

Status: concrete design 2026-07-04, std-package campaign (Process step 6).
Binds to [`std-package-ecosystem-shape.md`](std-package-ecosystem-shape.md)
(M1–M8, E1–E7; not re-decided here). Substrate: [`messaging.md`](messaging.md)
(framing, envelope, provider scope — referenced, not duplicated),
[`event-ingress.md`](event-ingress.md), [`human-review.md`](human-review.md),
[`admission-and-idempotency.md`](admission-and-idempotency.md).

## Design-tracker checklist

- **Core functionality.** Talking through communication platforms: declared
  `channel` routes, durable outbound `send`, generic inbound `Message`
  observation, and explicit typed mappings (`source interaction`). Channel/send
  and the 13-field `Message` envelope are SHIPPED (parser/lib.rs:17923 channel;
  body.rs:1910 send; parser/lib.rs:5714-5736 Message; e2e soft_middle.rs:527,
  :613). This design adds what is missing: real providers, capability reports,
  the receipt schema, interactions, and the human-review migration answer.
- **Why it belongs as a package.** Messaging is a semantic domain (E2) whose
  value is provider plurality behind one envelope. The provider set is exactly
  the thing that must grow without core changes; the construct surface is small
  and already lock-exempt via the built-in registry (main.rs:16073-16105).
- **What is NOT in the package.** Typed outside-input admission (std.ingress
  owns `signal`/`source` generally; messaging contributes only the
  `interaction` source provider); peer workflow signals (`emit signal … to`,
  kind `event.notify`→`signal.emit` per rename slice A — std.ingress); NL
  parsing of message text into domain values (explicit `coerce` per
  "Relationship To Ingress" in messaging.md); any hidden request/reply
  lifecycle; HTTP transport (std.ingress unless a distinct messaging need
  appears); the live `askHuman`/inbox surface (stays core until the migration
  below executes).
- **Target feature set (v1).** `MessageSendReceipt` typed receipt; provider
  capability reports with send/receive-capable distinguishable; three local
  providers (local mailbox, desktop notification, stdio) + `fixture` as a named
  provider; `interactions` block on send with correlation evidence; `source
  interaction` typed callback mapping; capability-conditioned static checks.
- **Dependencies.** Core: channel/send/Message (shipped); ingress admission
  identity (events UNIQUE(instance_id, idempotency_key), migrations/0001:75-76;
  models/maude/admission.maude). Ecosystem substrate: S5 CapabilityProvider
  seam + `capability_bound` promotion (M2) gates real provider dispatch; S6
  embedded manifests (M5) delivers the manifest below. Sibling packages: none
  required; std.coercion is the documented path from `msg.text` to domain types.
- **Provider expectations.** Every provider ships a capability report (below);
  the checker admits syntax only when the selected provider's report supports
  it; providers never append arbitrary facts — inbound goes through envelope
  admission ("Lowering And Runtime Boundary", messaging.md).
- **Open naming/boundary questions.** (1) Operator doors: `whip message`
  (injection, shipped main.rs:29490) vs a local-mailbox listing surface —
  recommendation: add `whip mailbox` for the local provider; the `whip inbox`
  SURFACE stays unchanged through migration step 1 (step 1 rehomes its backing
  store onto the local mailbox), with construct-level change only at step 2. (2) Channel `provider` identifiers are
  free-form today (messaging-demo.whip declares `provider slack` with no
  semantics); recommendation: short names (`local`, `desktop`, `stdio`,
  `fixture`) resolved against contributed provider kinds, unknown = check error
  once slice 2 lands. (3) Whether `human.ask` is renamed/retired is owned by
  the migration question below, not by rename slices A–C.
- **Verdict.** Build. Highest-leverage package after the substrate slices: the
  surface is shipped, the gap is providers, and std.messaging.local is the
  second real customer of the M2 CapabilityProvider seam after std.memory.

## Surface

Declarations and operations (grammar stays in the core parser per M1;
manifest rows authorize post-parse):

- `channel <name> { provider <p> [workspace <w>] [destination "<d>"] }` —
  shipped, metadata_only declaration_block (parser/lib.rs:17923, :5984).
- `send via <channel> { text <expr> [markdown <expr>] [thread_id <expr>]
  [interactions { … }] } as <binding>` — shipped except `interactions` (new,
  slice 6); effect_operation, lowering class capability_call.
  - **Effect kind: `capability.call`** (shipped: the built-in contract
    registers `effect_kind: "capability.call"`, parser/lib.rs:3311; the
    lowering emits kind `capability.call` with target capability
    `messaging.send`, kernel/rule_lowering.rs:2288-2299; the seeded provider
    row keys effect_kind `capability.call`,
    migrations/0001_runtime_store.sql:416).
  - **Capability id / target: `messaging.send`** — also the CONTRACT id.
    M3-reconciliation note: `messaging.send` appears in M3's id==kind example
    list as a capability id equal to the CONTRACT id, not the runtime effect
    kind. The kind-string default-required-capability derivation
    (store/lib.rs:6873-6877) governs generic kinds only; `capability.call`
    effects fall back to the TARGET capability instead
    (store/lib.rs:6497-6510). Promoting the runtime kind to `messaging.send`
    would be a FOURTH effect-kind rename outside M4's three decided slices,
    with idempotency-key rekey + store-open-guard + regeneration obligations —
    NOT proposed here; if ever wanted, it must enter as a new gated M4-style
    rename slice flagged for Jack.
  - **Derived facts: `capability.call.completed` / `capability.call.failed`**
    (kernel/effect_handlers.rs:1873-1882), not `messaging.send.*`.
- `when message from <channel> as msg` — shipped inbound observation binding
  the built-in `Message` schema over per-channel fact `message.<channel>`
  (parser/lib.rs:6987, :12073-12134). Inbound is ADMISSION, not an effect:
  there is no `messaging.receive` effect kind and no receive capability id in
  v1; the gate is the declared channel + envelope validation + the H3
  low-integrity label. If a later provider needs store-policy gating for
  inbound, a capability is added then (registered open item, not v1).
- `source interaction as <name> { channel <c> interactions [<names>] observe
  as <b> emit <signal> { … } }` — new (slice 6); source_declaration family,
  lowering class signal_source, admission identity = the external-signal key
  per "Interactions" in messaging.md. No new effect kind: admitted signals ride
  ingress admission. NOTE: `signal_source` is not package-authorable as
  shipped, so this construct cannot appear as a manifest constructs[] row
  without an S6 catalog/privilege extension — see the registered gap in the
  Manifest section.

**`MessageSendReceipt`** — new built-in output schema. The shipped contract
already names it (parser/lib.rs:3308) but no schema exists and the fixture
returns generic `{summary, target}` (effect_handlers.rs:1884-1887). Defined:

```text
message_id           string   durable outbound id (effect-key derived, stable across replay)
channel              string
provider             string   provider id that accepted the message
status               "accepted" | "delivered"
provider_message_id  string?  provider correlation handle when reported
thread_id            string?
destination          string?
accepted_at          string   provider-acknowledged instant
```

Failure is NOT a receipt: it settles `capability.call.failed` (the shipped
derived fact for capability.call effects, with EffectError kind
`capability.call`, kernel/effect_handlers.rs:1873-1882) with the DR-0032
`EffectError` base and routes to `fails as`. `status` starts at `accepted`;
`delivered` is emitted only by providers whose report includes it (none in v1).

## Providers

Per M2, seam classification per provider class; `fixture` becomes a NAMED
provider over the CapabilityProvider host projection (replacing the anonymous
else-branch at kernel/effect_handlers.rs:1883-1887), selected registry-honestly
via the promoted `capability_bound` (store/lib.rs:6928).

| provider | seam (M2) | direction | identity | correlation | interactions | receipts | hosting |
|---|---|---|---|---|---|---|---|
| `fixture` | in-process CapabilityProvider | bidirectional | claimed | provider_message_id | buttons, reactions | accepted/failed | none |
| `std.messaging.local` | in-process CapabilityProvider over store tables | bidirectional | claimed (CLI `--by`) | provider_message_id, thread_id | buttons, reactions | accepted/failed | local_cli |
| `std.messaging.desktop` | subprocess adapter (notify-send / osascript) | outbound_only | — | none | none | accepted/failed | local_cli |
| `std.messaging.stdio` | subprocess adapter, JSONL over child stdio | bidirectional | claimed | provider_message_id when supplied | buttons | accepted/failed | local_daemon |

Report axes and vocabulary are messaging.md "Provider Capability Report",
narrowed for v1: `delivery_receipts` ⊆ {accepted, failed}; `identity` ⊆
{anonymous, claimed_actor} (no verified_actor provider exists — any check
demanding verified identity fails closed); `content` ⊆ {text, markdown};
addressing = fixed_destination (+ thread_reply for local/stdio). Reports are
DATA (compiled constants mirrored by manifest `bindings[]` rows' config_json),
never code — consistent with M8.

Provider notes:

- **local** is the reference provider ("`std.messaging.local`", messaging.md):
  outbound writes a mailbox row + receipt; inbound = the shipped `whip message`
  injection path, upgraded: real `received_at` (today filled with `""`,
  main.rs:29584), `--by` actor identity, and a minted unique
  `provider_message_id` — fixing the silent dedup where message_id =
  idempotency_key(instance, channel, text, markdown) (main.rs:29577); resending
  identical text becomes distinct messages, while admission idempotency still
  dedups replays of the SAME delivery. Store-backed ⇒ DO-portable once mailbox
  tables ride RuntimeStore.
- **desktop** exists to prove outbound-only channels are first-class
  ("`std.messaging.desktop`", messaging.md); markdown stripped to text;
  subprocess ⇒ native-only (DO counterpart = DO tracker Phase 8).
- **stdio** exchanges JSONL envelopes with a configured child process
  ("`std.messaging.stdio`", messaging.md); inbound lines admitted through the
  same envelope-validation path as `whip message`; claimed identity only.

## Manifest

Per M5, an embedded `std.messaging` manifest (delivered by ecosystem slice S6;
content defined here) contributes:

- `libraries[]`: `std.messaging` with `effect_contracts[]`: contract id
  `messaging.send` (runtime effect kind `capability.call`, output_schema
  `MessageSendReceipt`, required_capabilities `["messaging.send"]` — closing
  the lock-time subset check by construction); `constructs[]`: `channel`
  (declaration_block/metadata_only), `send` (effect_operation/capability_call,
  target_capability `messaging.send`). `source interaction` is NOT a manifest
  constructs[] row in v1 — see the registered gap below.
- `capabilities[]`: `messaging.send`.
- `bindings[]` — the M2 load-bearing selection rows: one `capability_bindings`
  row per provider (`fixture`, `std.messaging.local`, `std.messaging.desktop`,
  `std.messaging.stdio`), each with capability `messaging.send` and that
  provider's capability report in the BINDING's `config_json`. These are the
  rows the promoted `capability_bound` actually queries
  (store/lib.rs:6928-6947); today only the seed
  `binding_messaging_send_builtin` (migrations/0001:444) satisfies the gate,
  and every send is `blocked_by_capability` without one.
- NO `providers[]` rows. For `capability.call` effects the policy gate never
  consults `effect_providers` — `policy_block_on` routes straight to the
  capability schema/binding/profile checks (store/lib.rs:6495-6511) — so rows
  keyed to a messaging kind would be exactly the dead-row hazard M2 exists to
  end. (The shipped seed row that exists, migrations/0001:416, is keyed
  effect_kind `capability.call` and is likewise not consulted by this gate.)
- `profiles[]`: `messaging.send` in the default allowlist rows (policy gate,
  store/lib.rs:6951; harness surface, harness_tools.rs:2315-2318).

**Channel→binding provider selection (designed rule).** `capability_bound` is
keyed (program_id, capability) — it cannot express which of four bound
providers a given channel means. The rule: a channel's `provider <p>`
identifier must resolve to the provider id of exactly one binding bound for
capability `messaging.send` (unknown or unbound identifier = check error,
matching checklist open question 2 and the static-check list). Dispatch then
selects the binding row whose provider id matches the channel's declared
provider and uses that binding's `config_json` capability report; the
program×capability `capability_bound` gate stays as the policy check, and
channel-scoped provider resolution is a separate, explicit selection step on
top of it.

**Registered gap — `source interaction` cannot ride the manifest as shipped.**
Its lowering class `signal_source` is `package_authorable: false` in the
platform catalog (core/lib.rs; asserted at core/lib.rs:1149), and manifest
validation hard-rejects such rows (cli/main.rs:17082-17084: package constructs
must use an authorable platform lowering); the only shipped privilege
mechanism, `reserved_keyword_privileges` (core/lib.rs:559-582), covers
reserved keywords, not lowering-class authorability. Resolution is a named S6
dependency (see Spec amendments): either S6 extends the catalog (promote
`signal_source` to package-authorable for std, or extend the privilege tuple
to cover lowering classes) — which triggers a
std-construct-authorization.maude re-model — or `source interaction` stays
core-registered (as `channel` effectively is today) and the manifest carries
no row for it, an asymmetry this design accepts until S6 decides.
std.ingress/std.time embedded manifests hit the same wall per E4.

## Static checks

M8 tier: **hand-coded core checks named here** (tier 2). No rule-of-three
generic engine applies — send has one success outcome + `fails as`. Shipped
already: declared-channel checks for send and `when message from`
(parser/lib.rs:7014, :6987). New, all conditioned on the selected provider's
capability report (checks are core code; the report is package data):

- `send via <c>` requires the channel provider's report `direction` ∈
  {outbound_only, bidirectional}.
- `when message from <c>` requires `direction` ∈ {inbound_only, bidirectional}
  — desktop channels are a check error here (send/receive-capable
  distinguishable, the v1 acceptance test).
- `interactions { … }` requires the report to list each requested interaction
  family; `source interaction` additionally requires `source_mapping:
  interaction_callback`, a declared channel, a declared signal, and interaction
  names present on some send in the program.
- A rule/mapping demanding verified actor identity is a check error against
  every v1 provider (fail-closed per report `identity`).
- Unknown channel `provider` identifier = check error once reports exist
  (examples/messaging-demo.whip's `provider slack` gets updated in slice 2).

## Information-flow face

DR-0029 posture ("Cross-package information flow"): std.messaging exports no
`@tool` in v1, so it carries no `workflow_tools[].information_flow` contract;
its IFC face is the channel crossings themselves, already labeled in the
shipped engine — `send via` is an egress sink (ifc.rs:1484-1485), inbound
messages are a low-integrity source under H3 (ifc.rs:1701-1715), authority
comes from consumer-side `grant channel <name> -> <provider:dest> <level>`
labels (parser/lib.rs:8503) with fail-closed untrusted bottom as default, and
per-field redaction covers send payloads (ifc.rs:2743, `redact … keep` shipped).
This is exactly X3 (no package-asserted authority): providers ship with zero
label authority; operators grant per channel. Additions in this design keep
the posture: interaction callbacks are admitted as low-integrity claimed-actor
input unless a future provider's report upgrades identity (identity ladder maps
to integrity labels); `source interaction` signals pass ingress admission and
the H8 internal-signal gate before any fact lands; providers may not append
facts directly ("Lowering And Runtime Boundary", messaging.md). Desktop is an
egress-only sink with no return edge.

## Human-review migration onto messaging (OWNED design question)

Registered by the ecosystem note ("Cross-cutting registered items" item 4).
Today `askHuman` is a fully live parallel surface: `human.ask` effect, inbox
store items, `whip inbox|show|answer (--choice|--text) [--by]`,
`human.answer.received` fact, typed `HumanAnswer` binding, liveness lint
(body.rs:1702; store/lib.rs:3499; main.rs:29157-29242, :31078).
[`human-review.md`](human-review.md) marks the target as messaging + ingress.

**Recommendation.** Keep `askHuman` as a CORE construct; migrate its
TRANSPORT onto std.messaging.local in two gated steps, executed only after the
local provider proves parity. The typed question/answer semantics (structured
choices, `HumanAnswer` binding, answered-without-ask liveness lint, severity)
are real value the generic `Message` envelope does not carry — but they are
sugar over messaging, not a parallel transport, which honors messaging.md's
non-goal (no `askHuman` PRIMITIVE inside the messaging package) without a
user-facing regression.

- **Step 1 (transport unification):** inbox items become local-mailbox rows;
  `whip inbox` becomes a filtered projection over the local mailbox; the
  `whip inbox answer` path becomes a local-provider inbound delivery. Surface,
  facts, and bindings unchanged; one store, two views.
- **Step 2 (visible lowering):** `askHuman` lowers to `send via` an implicit
  local `human` channel carrying an `interactions` block (choices as buttons,
  freeform as reply) + a generated `source interaction` mapping admitting the
  answer as the existing `human.answer.received` shape. No hidden lifecycle:
  the compiled form is visible outbound send + visible inbound observation,
  exactly the "Deferred Request Sugar" discipline.

**Parity gates before step 1 starts:** local provider shipped with claimed
actor identity (`--by` equivalence), correlation (provider_message_id ↔
question_id), interactions (choices), and a liveness story equal to the current
lint. Whether the `human.ask` effect kind is then renamed/retired is decided
inside the migration slice under the M4 one-way pre-release posture (it is NOT
one of the three decided renames). **The live surface stays untouched until
both gates pass; this design changes zero human-review code.**

## v1 implementation slices

Each independently gateable under the per-piece review discipline.

1. **Receipt + named fixture.** Define built-in `MessageSendReceipt`; move the
   fixture to a named `fixture` CapabilityProvider (rides ecosystem S5);
   validate provider output against the receipt via the existing
   CapabilityContract projection (effect_handlers.rs:1797-1801). Tests: e2e
   send asserts typed receipt fields; fixture-output-violates-schema settles
   failed. Model: none new (contract validation is existing machinery).
2. **Capability reports + conditioned checks.** Report data for the four
   providers; the five static checks above; update messaging-demo.whip.
   Tests: desktop channel + `when message from` = check error (the
   distinguishability acceptance test); bidirectional passes; negative
   fixtures per check. Model: extend
   models/maude/std-construct-authorization.maude with a
   report-conditioned-admission property (coverage + bite, negative fixture
   with a `RESIDUAL:Cfg` soup variable).
3. **std.messaging.local.** Mailbox tables + outbound rows + minted
   provider_message_id; `whip message` upgrades (received_at, --by, dedup
   fix); `whip mailbox` listing. Tests: outbound→mailbox→inbound roundtrip
   e2e; replay asserts the send outcome is re-read not re-run; dedup semantics
   pinned. Model: delivery/reply separation rides admission.maude identity.
4. **std.messaging.desktop.** Subprocess adapter, outbound-only,
   markdown→text. Tests: fake notifier binary fixture (success + nonzero-exit
   → `capability.call.failed` with EffectError base, kind `capability.call`).
5. **std.messaging.stdio.** JSONL child-process adapter, bidirectional;
   inbound lines through envelope admission. Tests: scripted child fixture
   roundtrip; malformed JSONL rejected at admission, no fact lands.
6. **Interactions + `source interaction`.** Parse the interactions block
   (bounded correlation evidence on the send, refs not content); parse
   `source interaction`; admission via the external-signal key; local + stdio
   callbacks exercise it. Tests: button callback → typed signal fact e2e;
   un-correlated callback refused; provider without interaction_callback
   refused at check. Model: new messaging-interaction Maude property —
   callback admitted only with (declared channel, matching interaction name,
   valid correlation) — coverage + bite.

The embedded manifest itself lands with ecosystem slice S6 using the Manifest
section above as its content spec.

## Spec amendments

- **spec/messaging.md, "Send":** delete the stale sentence claiming inbound
  "still needs a runtime messaging provider and remains deferred" — inbound
  shipped at fixture parity (`whip message` + e2e soft_middle.rs:613); only
  LIVE providers were deferred, and this design builds them.
- **spec/messaging.md, "Initial Provider Scope":** add `fixture` as a named
  first-class provider (M2 fixture-as-named-provider) and record the v1
  narrowing `delivery_receipts ⊆ {accepted, failed}`.
- **spec/messaging.md, "Provider Capability Report":** note reports are
  machine-checked manifest/compiled data validated at check time (this
  design), no longer prose-only.
- **S6 dependency (ecosystem note / std-package-ecosystem-shape.md, slice
  S6):** to admit a `source interaction` constructs[] row, S6 must either
  promote `signal_source` to package-authorable for std or extend the
  privilege mechanism (reserved_keyword_privileges, core/lib.rs:559-582) to
  cover lowering classes — either path re-models
  models/maude/std-construct-authorization.maude (coverage + bite). If S6
  declines, `source interaction` stays core-registered and the manifest
  carries no row for it (asymmetry noted in the Manifest section).
- **spec/human-review.md, "Core Effects":** `human.notify` and `human.approve`
  never existed in code (verified: only `human.ask` is wired) — remove them;
  point the direction note at this file's "Human-review migration onto
  messaging" section as the owning design.

## Deferred with cause

- **Slack / GitHub comments / email / Linear-Jira / chat / HTTP providers** —
  cause: platform-specific auth, identity, threading, interaction semantics
  (messaging.md "Initial Provider Scope" deferral stands; tracker note binds).
  Re-entry: a capability report + the M2 HTTP sans-IO seam; HTTP-webhook is
  the first candidate and must re-litigate the std.ingress boundary.
- **`request` sugar** — cause: hidden request/reply lifecycle risk; no source
  syntax accepted. Re-entry: only as transparent lowering per messaging.md
  "Deferred Request Sugar"; migration step 2 is the natural forcing case.
- **verified_actor identity** — cause: no v1 provider can verify; checks
  fail closed meanwhile. Re-entry: first hosted/webhook provider design.
- **`delivered` receipt status** — cause: no provider reports delivery
  callbacks. Re-entry: same provider that unlocks verified identity.
- **Typed attachments** — cause: attachments stay `array<string>` refs;
  bytes/blob handling is deferred with std.files bytes (files.md deferred
  set). Re-entry: files bytes design.
- **Dynamic recipients** — cause: fixed destinations keep the egress label
  per-channel; dynamic addressing needs an IFC story for recipient-as-data.
  Re-entry: with the first provider whose report claims dynamic_recipient.
- **DO-plane desktop/stdio** — cause: subprocess adapters are native-only by
  DR-0033 posture. Re-entry: DO tracker Phase 8 compute plane. (local runs on
  DO once mailbox tables ride RuntimeStore — not deferred, sequenced.)
- **`messaging.receive` capability id** — cause: inbound is admission, not an
  effect; no store-policy demand exists. Re-entry: first inbound provider
  needing operator-side admission policy.
- **Human-review migration execution** — cause: gated on local-provider
  parity (gates named above). Re-entry: the two-step plan in this file; live
  surface untouched meanwhile.
