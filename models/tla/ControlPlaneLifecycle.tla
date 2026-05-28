---- MODULE ControlPlaneLifecycle ----
EXTENDS Naturals, Sequences, FiniteSets

\* This model captures the durable control-plane lifecycle independently of
\* any particular Armature source program. Maude models local rule/effect
\* rewrites; this TLA+ model tracks asynchronous runtime actions, leases,
\* recovery, pause/resume, cancellation, and event-log ordering.

CONSTANTS
  \* @type: Set(Str);
  Effects,
  \* @type: Set(Str);
  Runs,
  \* @type: Set(Str);
  Events,
  \* @type: Set(<<Str, Str, Str>>);
  Dependencies

VARIABLES
  \* @type: Seq(Str);
  eventLog,
  \* @type: Str -> Str;
  effects,
  \* @type: Str -> Str;
  runs,
  \* @type: Str -> Str;
  runEffect,
  \* @type: Str -> Str;
  leases,
  \* @type: Set(Str);
  terminalEffects,
  \* @type: Int;
  projectionCursor,
  \* @type: Bool;
  paused,
  \* @type: Bool;
  cancelled

vars ==
  << eventLog, effects, runs, runEffect, leases, terminalEffects,
     projectionCursor, paused, cancelled >>

EffectStatuses ==
  {"queued", "blocked", "claimed", "running", "completed",
   "failed", "timed_out", "cancelled"}

RunStatuses ==
  {"none", "claimed", "running", "completed", "failed", "timed_out"}

LeaseStatuses ==
  {"none", "active", "expired"}

Init ==
  /\ eventLog = << >>
  /\ effects = [e \in Effects |-> "queued"]
  /\ runs = [r \in Runs |-> "none"]
  /\ runEffect = [r \in Runs |-> CHOOSE e \in Effects : TRUE]
  /\ leases = [e \in Effects |-> "none"]
  /\ terminalEffects = {}
  /\ projectionCursor = 0
  /\ paused = FALSE
  /\ cancelled = FALSE

AppendEvent(ev) ==
  /\ ev \in Events
  /\ eventLog' = Append(eventLog, ev)
  /\ UNCHANGED << effects, runs, runEffect, leases, terminalEffects,
                  projectionCursor, paused, cancelled >>

\* @type: (<<Str, Str, Str>>) => Str;
DepUpstream(d) == d[1]

\* @type: (<<Str, Str, Str>>) => Str;
DepPredicate(d) == d[2]

\* @type: (<<Str, Str, Str>>) => Str;
DepDownstream(d) == d[3]

PredicateSatisfied(d) ==
  \/ /\ DepPredicate(d) = "succeeds"
     /\ effects[DepUpstream(d)] = "completed"
  \/ /\ DepPredicate(d) = "fails"
     /\ effects[DepUpstream(d)] \in {"failed", "timed_out"}
  \/ /\ DepPredicate(d) = "completes"
     /\ effects[DepUpstream(d)] \in {"completed", "failed", "timed_out", "cancelled"}

EffectHasUnsatisfiedDeps(e) ==
  \E d \in Dependencies :
    /\ DepDownstream(d) = e
    /\ ~PredicateSatisfied(d)

Claimable(e) ==
  /\ ~paused
  /\ ~cancelled
  /\ effects[e] = "queued"
  /\ ~EffectHasUnsatisfiedDeps(e)

ClaimEffect(e, r) ==
  /\ e \in Effects
  /\ r \in Runs
  /\ Claimable(e)
  /\ runs[r] = "none"
  /\ effects' = [effects EXCEPT ![e] = "claimed"]
  /\ runs' = [runs EXCEPT ![r] = "claimed"]
  /\ runEffect' = [runEffect EXCEPT ![r] = e]
  /\ UNCHANGED << eventLog, leases, terminalEffects, projectionCursor,
                  paused, cancelled >>

StartRun(r) ==
  /\ r \in Runs
  /\ runs[r] = "claimed"
  /\ effects[runEffect[r]] = "claimed"
  /\ effects' = [effects EXCEPT ![runEffect[r]] = "running"]
  /\ runs' = [runs EXCEPT ![r] = "running"]
  /\ leases' = [leases EXCEPT ![runEffect[r]] = "active"]
  /\ UNCHANGED << eventLog, runEffect, terminalEffects, projectionCursor,
                  paused, cancelled >>

CompleteRun(r, ev) ==
  /\ r \in Runs
  /\ ev \in Events
  /\ runs[r] = "running"
  /\ runEffect[r] \notin terminalEffects
  /\ effects' = [effects EXCEPT ![runEffect[r]] = "completed"]
  /\ runs' = [runs EXCEPT ![r] = "completed"]
  /\ leases' = [leases EXCEPT ![runEffect[r]] = "none"]
  /\ terminalEffects' = terminalEffects \cup {runEffect[r]}
  /\ eventLog' = Append(eventLog, ev)
  /\ UNCHANGED << runEffect, projectionCursor, paused, cancelled >>

FailRun(r, ev) ==
  /\ r \in Runs
  /\ ev \in Events
  /\ runs[r] = "running"
  /\ runEffect[r] \notin terminalEffects
  /\ effects' = [effects EXCEPT ![runEffect[r]] = "failed"]
  /\ runs' = [runs EXCEPT ![r] = "failed"]
  /\ leases' = [leases EXCEPT ![runEffect[r]] = "none"]
  /\ terminalEffects' = terminalEffects \cup {runEffect[r]}
  /\ eventLog' = Append(eventLog, ev)
  /\ UNCHANGED << runEffect, projectionCursor, paused, cancelled >>

ExpireLease(e, ev) ==
  /\ e \in Effects
  /\ ev \in Events
  /\ effects[e] = "running"
  /\ leases[e] = "active"
  /\ e \notin terminalEffects
  /\ effects' = [effects EXCEPT ![e] = "queued"]
  /\ leases' = [leases EXCEPT ![e] = "expired"]
  /\ eventLog' = Append(eventLog, ev)
  /\ UNCHANGED << runs, runEffect, terminalEffects, projectionCursor,
                  paused, cancelled >>

DeriveProjection ==
  /\ projectionCursor < Len(eventLog)
  /\ projectionCursor' = projectionCursor + 1
  /\ UNCHANGED << eventLog, effects, runs, runEffect, leases, terminalEffects,
                  paused, cancelled >>

PauseInstance ==
  /\ ~paused
  /\ paused' = TRUE
  /\ UNCHANGED << eventLog, effects, runs, runEffect, leases, terminalEffects,
                  projectionCursor, cancelled >>

ResumeInstance ==
  /\ paused
  /\ ~cancelled
  /\ paused' = FALSE
  /\ UNCHANGED << eventLog, effects, runs, runEffect, leases, terminalEffects,
                  projectionCursor, cancelled >>

CancelInstance(ev) ==
  /\ ev \in Events
  /\ ~cancelled
  /\ cancelled' = TRUE
  /\ paused' = TRUE
  /\ eventLog' = Append(eventLog, ev)
  /\ effects' =
       [e \in Effects |->
         IF effects[e] \in {"queued", "blocked", "claimed"}
         THEN "cancelled"
         ELSE effects[e]]
  /\ terminalEffects' =
       terminalEffects \cup {e \in Effects : effects[e] \in {"queued", "blocked", "claimed"}}
  /\ UNCHANGED << runs, runEffect, leases, projectionCursor >>

Next ==
  \/ \E ev \in Events : AppendEvent(ev)
  \/ \E e \in Effects, r \in Runs : ClaimEffect(e, r)
  \/ \E r \in Runs : StartRun(r)
  \/ \E r \in Runs, ev \in Events : CompleteRun(r, ev)
  \/ \E r \in Runs, ev \in Events : FailRun(r, ev)
  \/ \E e \in Effects, ev \in Events : ExpireLease(e, ev)
  \/ DeriveProjection
  \/ PauseInstance
  \/ ResumeInstance
  \/ \E ev \in Events : CancelInstance(ev)

Spec ==
  Init /\ [][Next]_vars

EveryRunReferencesEffect ==
  \A r \in Runs : runEffect[r] \in Effects

NoRunWithoutRunningEffect ==
  \A r \in Runs :
    runs[r] = "running" => effects[runEffect[r]] = "running"

NoClaimedRunWithoutClaimedEffect ==
  \A r \in Runs :
    runs[r] = "claimed" => effects[runEffect[r]] = "claimed"

NoClaimableEffectHasUnsatisfiedDeps ==
  \A e \in Effects :
    Claimable(e) => ~EffectHasUnsatisfiedDeps(e)

NoNewEffectfulWorkWhilePaused ==
  paused => \A e \in Effects : ~Claimable(e)

NoTerminalEffectLeavesTerminalSet ==
  \A e \in terminalEffects :
    effects[e] \in {"completed", "failed", "timed_out", "cancelled"}

ProjectionCursorWithinLog ==
  projectionCursor <= Len(eventLog)

TypeOk ==
  /\ eventLog \in Seq(Events)
  /\ effects \in [Effects -> EffectStatuses]
  /\ runs \in [Runs -> RunStatuses]
  /\ runEffect \in [Runs -> Effects]
  /\ leases \in [Effects -> LeaseStatuses]
  /\ terminalEffects \subseteq Effects
  /\ projectionCursor \in Nat
  /\ paused \in BOOLEAN
  /\ cancelled \in BOOLEAN

====
