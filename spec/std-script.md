# `std.script`: package design — hard-off, identity, and the capability family

Status: concrete package design 2026-07-04 (standard-package campaign, Process
step 6). Governed by `spec/std-package-ecosystem-shape.md` (M1–M8, E1–E7); the
security design it packages is `spec/script-capabilities.md` (C9). This
document does not restate C9; it designs the PACKAGE: identity, import bite,
the runtime backstop behind the check-time gate, the `script.<name>`
capability family, and the raw-`exec` demotion.

## What is already real

C9's core shipped (commit bd9a0be) despite the ergonomics tracker row 18
reading todo in the impl column: capability form
`exec <name> with <record> -> <Type> as x` lowers to `exec.command` with
`mode=capability` (kernel/rule_lowering.rs:2608-2673); the operator script
manifest loads via `--script-manifest`/`WHIPPLESCRIPT_SCRIPT_MANIFEST`
(cli/main.rs:3032-3141); content pinning is fail-closed with the TOCTOU window
closed (read once, sha256 verify, staged copy, argv-direct spawn — no shell;
main.rs:23533-23568, proof-of-no-spawn test soft_middle.rs:1508); the `whip`
binary is refused at runtime (main.rs:23516-23525); `script.<name>` rides the
existing policy gate as `blocked_by_capability` (store/lib.rs:6491-6546);
hosted-profile check rejects raw exec (main.rs:3168-3242); evidence records
the executing hash (main.rs:23731-23738); secrets are `env:` refs only
(main.rs:23571-23591).

What does NOT exist: package identity (`use std.script` registers nothing),
the hard-off rule ("Static checks" in `spec/script-capabilities.md` —
unimplemented; exec is usable with zero imports), any formal model, and any
gate in front of dev-profile raw `exec "cmd"` beyond the env allowlist the
spec itself calls decorative (ExecProfile defaults to Dev; raw mode attaches
NO capability — rule_lowering.rs:2080-2084 attaches `script.<target>` only
for capability form). Those four gaps ARE this design.

## Design checklist

- **Core functionality.** Named, content-pinned process execution
  (`exec <name> with <record> -> <Type>`) plus the dev-tier raw form, with a
  package boundary that makes "scripts disabled" a machine-enforced fact at
  check time AND at effect admission.
- **Why it belongs as a package.** It is the one surface where text becoming
  execution is possible; the tracker's disabled-means-disabled note ("Current
  Notes" in the standard-package design tracker) demands an off switch no
  prompt-injection path can cross. A package import is the natural consent
  record for that switch, and per M5 `std.script` is the ONE std package that
  goes hard-off in v1.
- **What is NOT in the package.** DO-plane exec (native-only per DR-0033
  Decision 7; future home = DO tracker Phase 8 container sidecar — explicitly
  out of scope here); the owned-harness `bash` tool and agent-provider tools
  (agent-plane authority under agent grants/profiles, per "Hard exclusions
  and harness obligations" in `spec/script-capabilities.md` — hard-off
  obligates them not to act as a *fallback* for scripts, it does not govern
  them); per-script argv schemas, signed manifests, network script fetch (all
  "Out of scope (v1)" in the C9 spec, still deferred).
- **Target feature set (v1).** Embedded manifest + library identity; hard-off
  check gate; runtime admission backstop (import-conditional seeding, below);
  raw-exec demotion onto the capability plane; AST-based static checks
  replacing the line-scan lint; `script-hard-off.maude`; structured
  hash-mismatch evidence.
- **Dependencies.** Core: capability admission gate (`policy_block_on`
  choke points, store/lib.rs:6423/2508/4243 — the enforced gate per the
  constitution's ground truth 1), C3 `->` typed output + `with` stdin
  (`spec/json-ingestion.md`), DR-0032 typed failures, ExecProfile machinery.
  Packages: none. **Substrate: SC2 depends on constitution slice S6** — the
  shared embedded-manifest mechanism (binary-embedded manifest loading +
  validation through the third-party pipeline, catalog-privilege plumbing,
  ContractRegistry registration for std, the lock-exemption re-key, and the
  `std-construct-authorization.maude` re-model). None of that machinery
  exists today (`use std.script` registers nothing; no embedded-manifest
  loader exists). S6 lands first; SC2 supplies std.script's manifest DATA
  and its script-specific catalog rows, not the mechanism. `std.files`
  names `std.script` as its escape hatch ("Non-Goals" in `spec/files.md`)
  — a reference, not a dependency.
- **Provider expectations.** One provider, `exec`, seam class = subprocess
  adapter per M2 (the loft `CommandLoftClient` precedent); native-only;
  fixture behavior = the test-harness stub surface (spec-only, see Deferred).
- **Open naming/boundary questions.** (1) Effect kind stays `exec.command` —
  M3's id==kind rule applies to 1:1 operations, and this operation's
  capability is the parameterized family `script.<name>`, so no rename is
  forced; whether the kind should someday become `script.exec` is recorded
  open, NOT added to M4's decided rename set. (2) `exec` is today a core
  parser builtin, NOT a catalog reserved keyword — it is absent from
  PLATFORM_CONSTRUCT_CATALOG.reserved_keywords (core/lib.rs:554-558), so
  the privilege mechanism does not fire for it. SC2 explicitly ADDS `exec`
  to reserved_keywords and installs its privilege row `exec` → `std.script`
  (mirroring claim/renew/release → std.tracker, core/lib.rs:561-582). That
  addition is a real behavior change: from that slice on, any third-party
  manifest declaring a construct named `exec` becomes privilege-checked.
  Lowering class: `typed_effect_call`, the package-authorable class — NOT
  `core_effect` (see Manifest for why that class is unpassable). (3)
  `script.raw` reserves the manifest key `raw` (see Surface).
- **Verdict.** Build v1 now, model slice first (row 18's Model column is
  genuinely todo).

## Surface

No new source grammar. The package's source surface is the shipped `exec`
family, core-parsed per M1 (authorize-post-parse; grammar stays a core parser
arm):

```whip
use std.script

exec backup_repo with r -> Report as x        # capability form, both profiles
exec "cargo test" as y                        # raw form, dev profile only
```

- **Effect kind (exact): `exec.command`** — the single durable kind, with
  `mode=capability|raw` in the input record (shipped shape). Facts:
  `exec.command.completed` / `exec.command.failed` (DR-0032 EffectError base,
  effect_handlers.rs:1446-1466).
- **Capability ids (M3 pattern family):**
  - `script.<name>` — one id per operator-manifest entry; required capability
    attached by lowering for capability form (shipped,
    rule_lowering.rs:2080-2084). The embedded package manifest declares the
    FAMILY; concrete store rows are instantiated per operator script-manifest
    entry at load (main.rs:3143-3166).
  - `script.raw` — NEW: the demoted raw form's required capability (below).
    The operator script-manifest key `raw` is therefore reserved: manifest
    keys must match `[a-z_][a-z0-9_]*` and must not be `raw` (load error).
- **Library id:** `std.script`, `standard: true`. `use std.script` registers
  it in the compile-time ContractRegistry; usage-driven auto-registration is
  NOT added (hard-off inverts it: the import gates the construct, per M5).

## Hard-off semantics

Two layers, both required. The panel was unanimous that the check-time gate
alone does not resist forged IR; the runtime backstop is the load-bearing
security property.

**Layer 1 — check-time gate (M5 hard-off). LANDED 2026-07-04.** If `ir.uses`
does not contain `std.script`, EVERY `exec` source form — raw or capability,
either profile — is a check error, diagnostic id `security.script_disabled`.
Shipped as `check_script_hard_off` in `compile_source_path_for_validation`
(cli/main.rs), detecting `IrEffectKind::ExecCommand` effects; the 4 exec
examples were migrated to `use std.script` and their `.ir` snapshots
regenerated; tests `exec_is_hard_off_without_use_std_script` +
`hosted_check_rejects_raw_exec` (fixture migrated). This is the author-facing
consent surface and the only std package with a hard import requirement in v1
(E5 ladder). **Layer 2 remains the load-bearing security property** (below,
deferred to the S6d embedded-manifest seeding infra): Layer 1 alone does not
resist forged IR, since the `use` line is itself forgeable.

**Layer 2 — runtime backstop: import-conditional per-program seeding
(CHOSEN) over an IR-uses gate at exec dispatch.** All `exec.command` effects
carry a required `script.*` capability (capability form: `script.<name>`,
shipped; raw form: `script.raw`, new). Capability rows for a program are
seeded ONLY when both keys turn: (a) the program's registered IR imports
`std.script`, and (b) the authority source exists on the operator plane — the
script manifest entry for `script.<name>`; dev profile + allowlist presence
for `script.raw`. Any `exec.command` effect without a bound capability then
blocks at the store admission gate as `blocked_by_capability`
(store/lib.rs:6491-6546), surfaced with `security.script_disabled`, before
any provider run.

Rationale for the pick:

1. **It reuses the only gate with teeth.** `policy_block_on` at
   claimable_effects and start_run is the enforced choke point (constitution
   ground truth 1). No new enforcement engine; one seeding condition.
2. **Two-key property against forged IR.** An attacker who fabricates IR can
   add a `use std.script` line — the import is forgeable by construction. The
   attacker cannot mint operator-plane rows: `script.<name>` requires a
   manifest entry whose bytes the operator pinned; `script.raw` requires the
   dev posture the operator set. Forged IR therefore yields at most execution
   the operator already authorized, scoped to programs the operator installed
   the manifest for (per-program bindings, main.rs:3143-3166). An IR-uses
   check at dispatch, by contrast, reads the forgeable artifact itself and
   adds nothing beyond Layer 1 against the named threat.
3. **It lives on the store plane, not in the executor.** The dispatch
   alternative would add machinery to the hardcoded kind match in
   cli/main.rs — exactly what M7's hard rule forbids — and would need
   re-implementing for the Phase 8 DO compute plane. The admission gate is
   already store-trait territory and travels with the kernel lift for free.
4. **Fail-closed ordering.** Admission-time blocking means a disabled script
   never reaches a claim or run record; the `no exec boundary crossed`
   contract in "failure routing" in `spec/error-handling.md` holds by
   construction, and the existing pin-verify + argv-direct spawn path remains
   the inner defense for effects that ARE admitted.

Defense-in-depth retained: the executor's existing pre-spawn checks (hash
verify, `whip` exclusion, env-ref resolution) are unchanged; the backstop sits
in front of them, not instead of them.

## Raw `exec` demotion

Per the disabled-means-disabled tracker note, raw dev `sh -c` exec stops
bypassing the capability plane (today it attaches no capability — the
recorded hole):

- Raw lowering attaches required capability `script.raw`.
- `script.raw` is seeded per program only when: dev profile active AND
  `WHIPPLESCRIPT_EXEC_ALLOW` is non-empty AND the program imports
  `std.script`. Hosted profile NEVER seeds it — hosted raw exec is now
  blocked at admission even from forged IR, not merely rejected at check.
- The allowlist keeps its shipped role as a prefix filter inside the raw
  handler (main.rs:23705-23718) — a convenience filter, not the boundary.
- Net dev-loop cost: one `use std.script` line.

## Providers

Per M2 seam classification: **subprocess adapter** (seam 2, the loft
precedent). One provider row, id `exec`, native-only (worker id `whip-exec`).
It is NOT an HTTP step machine and NOT a native adapter trait; no DO
counterpart exists or is designed here (DO tracker Phase 8 owns the
container-sidecar compute plane). The CapabilityProvider host-projection seam
(M2's new seam) is not used: `exec.command` is a builtin kind with its own
handler, not a `capability.call`.

## Manifest

Per M5, `std.script` ships as an embedded manifest, validated by the same
pipeline as third-party manifests, catalog-privileged:

- **libraries[]:** `std.script` (standard) with the `exec.command` effect
  contract — input `exec.input`, output typed-by-`->` per C3, required
  capability = the `script.<name>` family (family declaration, satisfying the
  M3 lock-time check `contract caps ⊆ manifest capabilities[]` by pattern).
- **constructs[]:** `exec` registered as `effect_operation`, lowering class
  **`typed_effect_call`**, core-parsed (M1 authorize-post-parse). The class
  is a RECORDED CHOICE, not a default: `core_effect` is
  package_authorable:false (core/lib.rs:406-410, target_capability
  Forbidden), and the third-party manifest pipeline unconditionally rejects
  non-authorable lowering classes (cli/main.rs:13200-13204, :17082-17084)
  BEFORE the reserved-keyword privilege check (:13217) — the privilege
  mechanism grants reserved keywords only, never class authorability — so a
  `core_effect` row can never pass SC2's validation gate. Registering under
  the package-authorable `typed_effect_call` class while KEEPING the
  `exec.command` effect kind is exactly the E4/std.files precedent
  (class promotion, zero durable-history churn). Plus: add `exec` to
  PLATFORM_CONSTRUCT_CATALOG.reserved_keywords (it is absent today) and
  install the privilege tuple `exec` → `std.script`.
- **capabilities[]:** the `script.<name>` pattern family + `script.raw`.
- **providers[]:** `exec` (subprocess adapter, native-only).
- **profiles:** `dev` (may bind `script.raw` + manifest entries), `hosted`
  (manifest entries only; `script.raw` never bindable).

The std lock exemption re-keys to "manifest embedded in this binary" in the
shared M5 slice (S6), which owns the `std-construct-authorization.maude`
re-model; this design's SC1 is the script-specific hard-off model, distinct
from that.

## Static checks

M8 tier 2: hand-coded core checks, named here as this package's demands
(packages bring data, never code). No new generic check engine.

1. **Hard-off:** any `exec` form without `use std.script` →
   `security.script_disabled` (both profiles).
2. **Hosted raw:** raw string exec under `--exec-profile hosted` → error
   suggesting the capability form (shipped, main.rs:3168-3242) — REWRITTEN
   onto the AST/IR, replacing the fragile line-scan over rule bodies
   (main.rs:3196-3208); M8 names this rewrite as owned by this slice.
3. **Manifest resolution:** capability name absent from a supplied manifest →
   check error listing declared capabilities (shipped; keep).
4. **`with` typing:** `with <binding>` requires a typed record binding —
   closing the recorded gap where lowering accepts arbitrary text
   (rule_lowering.rs:2627-2645).
5. **Control-plane deny-list at check time:** manifest argv whose executable
   basename is `whip` (or a future denied list) is a manifest load error,
   mirroring the shipped runtime refusal so operators learn at pin time.

## Information-flow face

Posture per DR-0029 ("Decision"): `std.script` exports no `@tool`, so it
carries no `information_flow` package-contract attestation; its crossings are
first-party effect crossings the consumer's own governance sees directly.

- **Egress:** the typed stdin record crosses to an operator-pinned process —
  crossing identity `exec:script.<name>`. Authority to cross is exactly the
  bound capability; there is no address beyond the pinned argv, so the
  capability id IS the egress address.
- **Ingress:** stdout re-enters as typed facts (C3). Integrity posture:
  *pinned provenance, untrusted content* — evidence records the executing
  hash (shipped), but the output value is external data and defaults to
  low-integrity exactly as file imports and inbound messages do; no label
  laundering through the typed-parse step. Per-capability operator label
  grants (the `grant channel` analogue) are deferred (below).
- **Secrets:** `env:` refs resolve at spawn and never enter facts or evidence
  (shipped, main.rs:23571-23591).
- **Raw dev exec** is an ungoverned crossing by definition; its containment
  is the demotion (dev-profile-only capability) — recorded, not labeled.
- Turn-plane `command run` grants remain owned by the owned-harness surface
  and its governance-envelope check (harness_tools.rs:2062-2132); monotone
  narrowing per M3 applies: turn grants ⊆ store-bound `script.*`.

## Model (model-first)

`models/maude/script-hard-off.maude`, landed BEFORE the enforcement slices,
covering the five properties of "Modeling notes" in
`spec/script-capabilities.md`:

- **hard-off soundness** — no reachable transition spawns a process from a
  configuration lacking (import ∧ operator authority): NoSolution searches
  from a no-import soup and a no-manifest-entry soup (negative fixtures carry
  a `RESIDUAL:Cfg` soup variable, per house Maude discipline);
- **fail-closed** — mismatched bytes never spawn under a capability name;
- **no-injection** — no author-controlled string reaches argv;
- **provenance** — every spawn records the executing hash;
- **plane separation** — the composite: authoring plane cannot cause
  execution of unpinned bytes.

Coverage AND bite gates apply.

## v1 implementation slices

Each independently gateable under the per-piece review discipline.

- **SC1 — model.** `script-hard-off.maude` as above. Gate: coverage + bite in
  check-rule-coverage; no code change.
- **SC2 — package identity.** DEPENDS ON substrate slice S6 (the shared
  embedded-manifest mechanism — see Dependencies); SC2 does not build that
  machinery, it supplies std.script's data: embedded manifest (library,
  contract, capability family, provider, profiles, `typed_effect_call`
  construct row) + the `exec` reserved_keywords addition + its privilege
  row + ContractRegistry registration on `use std.script`. Gate: manifest
  validates through the third-party pipeline (passable because the
  construct row carries the package-authorable `typed_effect_call` class);
  drift test manifest-vs-compiled-truth; registry snapshot test.
- **SC3 — check-time hard-off + AST lint rewrite.** Layer 1 errors; line-scan
  lint replaced; repo examples using `exec` gain `use std.script`. Gate:
  check-error fixtures (raw + capability, no-import), span-exactness tests,
  existing hosted-check tests stay green.
- **SC4 — runtime backstop + raw demotion.** `script.raw` capability on raw
  lowering; import-conditional seeding for the whole `script.*` family;
  blocked surfacing as `security.script_disabled`. Gate: forged-IR test — a
  hand-registered IR containing exec effects with no import/no manifest is
  blocked at admission with NO spawn (marker-file proof, the
  soft_middle.rs:1508 pattern); hosted raw-exec-blocked-at-store test;
  dev-loop test (import + allowlist ⇒ raw runs).
- **SC5 — static-check hardening.** `with` typed-record enforcement +
  check-time deny-list + `raw` key reservation. Gate: negative fixtures per
  check.
- **SC6 — operator surface polish.** Structured mismatch evidence fields
  (expected_sha256/actual_sha256, replacing message-text-only) +
  `whip script list|verify` (read-only hash recheck against pins). Gate:
  evidence snapshot test; CLI integration test.

Ordering: S6 (external substrate, constitution build order) → SC1 → SC2 →
{SC3, SC4} → SC5 → SC6. SC1 is code-free and may land before or in parallel
with S6; SC2 depends on S6; SC4 depends on SC2 (seeding keys off the
registered library); SC3/SC4 are independently landable after it.

## Spec amendments

- **`spec/script-capabilities.md`, "Enforcement":** replace the abstract
  "not imported, not installed, or disabled by profile/policy" hard-off
  clause with the two-layer design (check gate + import-conditional seeding
  at the admission gate) and cite this document as the package contract.
- **`spec/script-capabilities.md`, "Tiers":** dev raw exec is no longer
  "behind the env allowlist" alone — it requires `use std.script` +
  dev-profile `script.raw` binding; the allowlist is a secondary prefix
  filter.
- **`spec/script-capabilities.md`, "Static checks":** the not-imported rule
  moves from aspiration to shipped-by-SC3; add the check-time deny-list item.
- **`spec/script-capabilities.md`, "The manifest":** the key `raw` is
  reserved (manifest load error) and manifest keys must match
  `[a-z_][a-z0-9_]*` — the key-grammar rule this design introduces in
  Surface amends the manifest section.
- **`spec/script-capabilities.md`, evidence-recording prose:** hash-mismatch
  evidence gains structured `expected_sha256`/`actual_sha256` fields (SC6),
  replacing the message-text-only shape the spec currently describes.
- **`spec/workflow-testing.md`, "script_run outcomes":** the `disabled`
  outcome maps to admission-blocked `security.script_disabled` (no run
  record), distinguishing it from `denied` (bound but policy-refused) — one
  clarifying sentence, no vocabulary change.
- No amendment to `spec/error-handling.md` or `spec/verification.md` — both
  already carry the `security.script_disabled` contract this design
  implements.

## Deferred with cause

- **DO-plane exec.** Cause: no subprocess on wasm (DR-0033 Decision 7).
  Re-entry: DO tracker Phase 8 container-sidecar compute plane; this
  package's admission backstop already travels with the store lift.
- **Per-capability output label grants (IFC).** Cause: no consumer yet
  demands trusted script output; default low-integrity is safe. Re-entry:
  first workflow needing high-integrity script facts; design mirrors
  `grant channel` labels.
- **`whip script pin` / re-pin-with-diff tooling.** Cause: C9e's human
  confirmation is procedural today (hand-edited operator JSON) and manifest
  changes are rare by design; SC6's `verify` covers drift detection.
  Re-entry: the harness-curator flow (LLM-curated manifests) landing.
- **Test-harness `stub script` / `script_run` outcomes.** Cause: spec-only
  surface ("script_run" table in `spec/workflow-testing.md`); v1 fixtures are
  env/manifest-controlled tests. Re-entry: first workflow-test suite stubbing
  a script capability.
- **Per-script argv schemas, signed manifests, network fetch, keychain
  secret handles.** Cause: unchanged from "Out of scope (v1)" in
  `spec/script-capabilities.md`; single-host hash pinning suffices.
  Re-entry: multi-host distribution work.
- **`exec.command` → `script.exec` kind rename.** Cause: not in M4's decided
  rename set; adding a fourth rename slice mid-campaign buys no security.
  Re-entry: if a later campaign renames, it rides the M4 pattern (one-way,
  store-open guard, map-dedup'd regeneration).
