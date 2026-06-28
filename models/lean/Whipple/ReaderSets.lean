/-
Reader/writer SETS — the confidentiality label as a set of compartments.

The single-authority model (`Label.reader : P`) is the leaf case; a real
confidentiality label is a SET of reader authorities (compartments), and a party
may read a value only if it is cleared for EVERY compartment. This module proves
that set forms a join-semilattice with the empty set as bottom: union is the join
(combining data restricts to the readers common to both), so the label algebra the
checker will grow into is sound. Builds on `Whipple.ActsFor`; still Mathlib-free.
-/
import Whipple.ActsFor

namespace Whipple

variable {P : Type}

/-- A confidentiality label as a set of reader authorities (compartments). A party
    with authority `a` may read a value labeled `rs` iff it acts-for EVERY
    compartment — it must be cleared for all of them. -/
def canRead (d : List (P × P)) (pub : P) (a : P) (rs : List P) : Prop :=
  ∀ r ∈ rs, ActsFor d pub a r

/-- The empty label is the public bottom: everyone may read it (vacuously). -/
theorem canRead_nil (d : List (P × P)) (pub a : P) : canRead d pub a [] := by
  intro r hr
  cases hr

/-- The JOIN of two confidentiality labels is their union: a party may read the
    combination iff it may read BOTH parts. So union is the least upper bound —
    combining data restricts to the readers common to both inputs (the set form of
    the join box, I-IFC2). -/
theorem canRead_append (d : List (P × P)) (pub a : P) (rs ss : List P) :
    canRead d pub a (rs ++ ss) ↔ canRead d pub a rs ∧ canRead d pub a ss := by
  constructor
  · intro h
    exact ⟨fun r hr => h r (List.mem_append.mpr (Or.inl hr)),
           fun s hs => h s (List.mem_append.mpr (Or.inr hs))⟩
  · rintro ⟨h1, h2⟩ x hx
    rcases List.mem_append.mp hx with hr | hs
    · exact h1 x hr
    · exact h2 x hs

/-- Join is commutative w.r.t. readership: `rs ++ ss` and `ss ++ rs` have the same
    readers. (Idempotence and associativity follow likewise from `canRead_append`.) -/
theorem canRead_append_comm (d : List (P × P)) (pub a : P) (rs ss : List P) :
    canRead d pub a (rs ++ ss) ↔ canRead d pub a (ss ++ rs) := by
  rw [canRead_append, canRead_append, and_comm]

/-- Readership is MONOTONE in clearance: a higher-authority party reads at least as
    much. If `a` acts-for `b` and `b` may read `rs`, then `a` may read `rs` — so the
    acts-for order on principals lifts to the label order. -/
theorem canRead_mono (d : List (P × P)) (pub a b : P) (rs : List P)
    (hab : ActsFor d pub a b) (h : canRead d pub b rs) : canRead d pub a rs := by
  intro r hr
  exact actsFor_trans d pub hab (h r hr)

end Whipple
