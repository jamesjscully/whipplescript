# Lean proof layer — the IFC algebra, machine-checked

Maude and TLA+ give fast, bounded design feedback (they check concrete fixtures).
They do **not** prove universal algebraic facts. After the report-vs-check bug
(2026-06-27) showed that "we have formal models" had been quietly meaning "we have
bounded smoke tests," this layer proves the **load-bearing algebra** the Rust
checker and the Maude models only asserted in comments.

It is deliberately **Mathlib-free** and pinned to an installed toolchain
(`lean-toolchain` → v4.30.0), so it builds **offline and hermetically** in the
gate — and, crucially, the acts-for order is proven about **our own** closure, not
imported from a library (importing `Relation.ReflTransGen` would not answer
"is *our* `can_act` the preorder").

## What is proven

`Whipple/ActsFor.lean` — the order:
- `actsFor_refl`, `actsFor_trans` → acts-for is a **preorder**.
- `public_is_bottom`, `public_acts_for_only_public` → `public` is its **bottom**
  and holds **no authority** (mirroring the Rust `if p == PUBLIC { return false }`).
- `integ_is_conf_dual` → integrity is confidentiality with the axes swapped and
  endpoints reversed — the exact sense in which `injects` reuses `can_act`.
- `conf_to_public_needs_public_source` → the **fail-closed sticky boundary**:
  confidential data cannot reach a public sink without a declassify.

`Whipple/Decide.lean` — the algorithm is the order:
- `fuel_sound` + `actsFor_complete` + `canAct_iff` → the bounded forward search
  the Rust `Envelope::can_act` and the Maude `reach` run is **sound and complete**
  against `ActsFor`. It decides **exactly** the preorder — no more, no less.

`Whipple/Boundary.lean` — the bug class, fixed at the type level:
- `consumer_relies_on_genuine`, `no_verified_for_tampered` → a `Verified` artifact
  is constructible only with a genuineness proof, so **every** consumer is gated by
  construction. This is the discipline the report-vs-check bug violated; the Rust
  `VerifiedEnvelope` refactor (pending) is its realization.

## Trust base

`#print axioms Whipple.canAct_iff` → `[propext, Quot.sound]` (Lean's standard core;
no `sorryAx`, not even `Classical.choice`). The bottom and boundary theorems depend
on **no axioms at all**.

## Map to the implementation

| Lean | Rust (`crates/whipplescript-cli/src/ifc.rs`) | Maude |
| --- | --- | --- |
| `ActsFor` / `canAct_iff` | `Envelope::can_act` | `canAct` / `reach` |
| `FlowConf` (= `¬ leaks`) | `Envelope::leaks` | `infoflow-confidentiality` |
| `FlowInteg` (= `¬ injects`) | `Envelope::injects` | `infoflow-integrity` |
| `Verified` boundary | `gov::SignedEnvelope::verify` (→ `VerifiedEnvelope`, pending) | `subworkflow-attestation` |

## Running

```sh
scripts/check-lean-models.sh   # builds + rejects any sorry/admit/axiom/native_decide
```

## Not yet proven here (next obligations)

- The reader/writer **sets** (a single up-set per resource is modeled; multi-reader
  meet/join lattice is future).
- **NMIF / robust declassification** (a downgrade not influenced by an attacker).
- Durable **label carriage** across persistence/instance boundaries (I-IFC7) — that
  is a transition-system property; the natural home is Veil (Lean) or TLA+, not this
  algebraic core.
