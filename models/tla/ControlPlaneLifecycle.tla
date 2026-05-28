---- MODULE ControlPlaneLifecycle ----
EXTENDS Naturals, Sequences, FiniteSets

\* Skeleton only. This model will validate durable runtime lifecycle behavior:
\*
\* - no run starts unless its effect is claimable
\* - dependency predicates gate downstream effects
\* - recovery preserves per-instance event order
\* - retry reuses effect identity unless a new attempt is created
\*
\* The concrete action definitions should be added after the runtime store
\* fields stabilize.

CONSTANTS Effects, Runs, Events

VARIABLES
  eventLog,
  effects,
  deps,
  runs,
  runEffect,
  leases,
  projectionCursor

vars == << eventLog, effects, deps, runs, runEffect, leases, projectionCursor >>

Init ==
  /\ eventLog = << >>
  /\ effects = [e \in Effects |-> "queued"]
  /\ deps = {}
  /\ runs = [r \in Runs |-> "none"]
  /\ runEffect = [r \in Runs |-> CHOOSE e \in Effects : TRUE]
  /\ leases = {}
  /\ projectionCursor = 0

PredicateSatisfied(d) ==
  \/ /\ d.predicate = "succeeds"
     /\ effects[d.upstream] = "completed"
  \/ /\ d.predicate = "fails"
     /\ effects[d.upstream] \in {"failed", "timed_out"}
  \/ /\ d.predicate = "completes"
     /\ effects[d.upstream] \in {"completed", "failed", "timed_out", "cancelled"}

EffectHasUnsatisfiedDeps(e) ==
  \E d \in deps :
    /\ d.downstream = e
    /\ ~PredicateSatisfied(d)

Claimable(e) ==
  /\ effects[e] = "queued"
  /\ ~EffectHasUnsatisfiedDeps(e)

NoRunWithoutClaimableEffect ==
  \A r \in Runs :
    runs[r] = "running" =>
      effects[runEffect[r]] = "running"

Next ==
  UNCHANGED vars

Spec ==
  Init /\ [][Next]_vars

====
