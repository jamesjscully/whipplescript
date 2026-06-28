/-
WhippleScript IFC algebra — machine-checked core (Lean 4, no Mathlib).

The Rust `Envelope::can_act` (crates/whipplescript-cli/src/ifc.rs) and the Maude
`canAct` models only ASSERT, in comments, that acts-for is a reflexive-transitive
preorder with `public` as its bottom, and that the integrity axis is the dual of
the confidentiality axis. This module PROVES those facts about the exact order the
checker decides — so the assertions become theorems, not faith.
-/
namespace Whipple

variable {P : Type}

/-- One acts-for step under policy `d` (the delegation edges) with bottom `pub`:
    `p` steps to `q` when `q` is the public bottom (everyone acts-for public) or
    there is a delegation edge `p → q` and `p` is not itself the powerless bottom.
    The `p ≠ pub` guard mirrors the Rust `if p == PUBLIC { return false }`: the
    bottom holds no authority, so edges out of it are dead. -/
def Step (d : List (P × P)) (pub : P) (p q : P) : Prop :=
  q = pub ∨ (p ≠ pub ∧ (p, q) ∈ d)

/-- Reflexive-transitive closure of a relation (defined here to stay Mathlib-free,
    so the proof layer builds offline with a pinned toolchain). -/
inductive Star (r : P → P → Prop) : P → P → Prop where
  | refl (a : P) : Star r a a
  | head {a b c : P} : r a b → Star r b c → Star r a c

/-- `ActsFor d pub p q` : `p` acts-for `q` — the order `can_act` decides. -/
def ActsFor (d : List (P × P)) (pub : P) (p q : P) : Prop :=
  Star (Step d pub) p q

theorem star_trans {r : P → P → Prop} {a b c : P}
    (h1 : Star r a b) (h2 : Star r b c) : Star r a c := by
  induction h1 with
  | refl _ => exact h2
  | head hab _ ih => exact Star.head hab (ih h2)

/-- Acts-for is reflexive. -/
theorem actsFor_refl (d : List (P × P)) (pub p : P) : ActsFor d pub p p :=
  Star.refl p

/-- Acts-for is transitive. With `actsFor_refl`, acts-for is a preorder — the
    property the closure was only assumed to have. -/
theorem actsFor_trans (d : List (P × P)) (pub : P) {p q r : P}
    (h1 : ActsFor d pub p q) (h2 : ActsFor d pub q r) : ActsFor d pub p r :=
  star_trans h1 h2

/-- `public` is the bottom: everyone acts-for it. -/
theorem public_is_bottom (d : List (P × P)) (pub p : P) : ActsFor d pub p pub :=
  Star.head (Or.inl rfl) (Star.refl pub)

/-- A single step out of `public` can only land on `public` (the guard kills
    edges out of the bottom). -/
theorem step_from_public {d : List (P × P)} {pub q : P}
    (h : Step d pub pub q) : q = pub := by
  rcases h with h | ⟨hne, _⟩
  · exact h
  · exact absurd rfl hne

/-- Helper: a path that starts at `public` ends at `public`. Both endpoints are
    generalized so `induction` builds the right motive. -/
theorem star_pub_sink {d : List (P × P)} {pub : P} :
    ∀ {p q : P}, Star (Step d pub) p q → p = pub → q = pub := by
  intro p q h
  induction h with
  | refl _ => intro hp; exact hp
  | @head a b c hab _ ih =>
    intro hp; subst hp
    exact ih (step_from_public hab)

/-- `public` acts-for nothing but itself — the dual of `public_is_bottom`. So the
    bottom truly holds no authority, matching the Rust early-return. -/
theorem public_acts_for_only_public {d : List (P × P)} {pub q : P}
    (h : ActsFor d pub pub q) : q = pub :=
  star_pub_sink h rfl

/-- A two-axis label: a reader (confidentiality) authority — who may read — and an
    integrity (vouching) authority — who may have influenced this. Mirrors
    `reader_authority` and `integrity_authority`. -/
structure Label (P : Type) where
  reader : P
  integ : P

/-- Safe confidentiality flow `src → sink`: every reader of the sink is cleared for
    the source, i.e. `sink.reader` acts-for `src.reader`. This is exactly the
    negation of the Rust `leaks` (declassify hatch aside). -/
def FlowConf (d : List (P × P)) (pub : P) (src sink : Label P) : Prop :=
  ActsFor d pub sink.reader src.reader

/-- Safe integrity flow `src → sink`: the source is trusted enough for the sink,
    i.e. `src.integ` acts-for `sink.integ`. The negation of the Rust `injects`. -/
def FlowInteg (d : List (P × P)) (pub : P) (src sink : Label P) : Prop :=
  ActsFor d pub src.integ sink.integ

/-- Exchange the two axes of a label. -/
def swapAxes (l : Label P) : Label P := ⟨l.integ, l.reader⟩

/-- THE DUALITY: an integrity flow is a confidentiality flow with the axes
    swapped and the endpoints reversed — the precise sense in which `injects`
    reuses `can_act` with swapped argument order. Holds definitionally. -/
theorem integ_is_conf_dual (d : List (P × P)) (pub : P) (src sink : Label P) :
    FlowInteg d pub src sink ↔ FlowConf d pub (swapAxes sink) (swapAxes src) :=
  Iff.rfl

/-- The fail-closed sticky boundary, proven: a public-readable sink can receive a
    confidential flow ONLY from a public-readable source. Confidential data cannot
    reach a public sink without a declassify — there is no laundering via the
    order. -/
theorem conf_to_public_needs_public_source (d : List (P × P)) (pub : P)
    (src sink : Label P) (hsink : sink.reader = pub) (h : FlowConf d pub src sink) :
    src.reader = pub := by
  unfold FlowConf at h
  rw [hsink] at h
  exact public_acts_for_only_public h

end Whipple
