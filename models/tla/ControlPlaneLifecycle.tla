---- MODULE ControlPlaneLifecycle ----
EXTENDS Naturals, Sequences, FiniteSets

\* This model captures the durable control-plane lifecycle independently of
\* any particular Whippletree source program. Maude models local rule/effect
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
  \* @type: Seq(Str);
  recoveryLog,
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
  cancelled,
  \* @type: Bool;
  recovering

vars ==
  << eventLog, recoveryLog, effects, runs, runEffect, leases, terminalEffects,
     projectionCursor, paused, cancelled, recovering >>

EffectStatuses ==
  {"queued", "blocked", "claimed", "running", "completed",
   "failed", "timed_out", "cancelled"}

RunStatuses ==
  {"none", "claimed", "running", "completed", "failed", "timed_out"}

LeaseStatuses ==
  {"none", "active", "expired"}

Init ==
  /\ eventLog = << >>
  /\ recoveryLog = << >>
  /\ effects = [e \in Effects |-> "queued"]
  /\ runs = [r \in Runs |-> "none"]
  /\ runEffect = [r \in Runs |-> CHOOSE e \in Effects : TRUE]
  /\ leases = [e \in Effects |-> "none"]
  /\ terminalEffects = {}
  /\ projectionCursor = 0
  /\ paused = FALSE
  /\ cancelled = FALSE
  /\ recovering = FALSE

AppendEvent(ev) ==
  /\ ev \in Events
  /\ ~recovering
  /\ eventLog' = Append(eventLog, ev)
  /\ UNCHANGED << effects, runs, runEffect, leases, terminalEffects,
                  projectionCursor, paused, cancelled, recoveryLog, recovering >>

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
  /\ ~recovering
  /\ Claimable(e)
  /\ runs[r] = "none"
  /\ effects' = [effects EXCEPT ![e] = "claimed"]
  /\ runs' = [runs EXCEPT ![r] = "claimed"]
  /\ runEffect' = [runEffect EXCEPT ![r] = e]
  /\ UNCHANGED << eventLog, recoveryLog, leases, terminalEffects, projectionCursor,
                  paused, cancelled >>

StartRun(r) ==
  /\ r \in Runs
  /\ ~recovering
  /\ runs[r] = "claimed"
  /\ effects[runEffect[r]] = "claimed"
  /\ effects' = [effects EXCEPT ![runEffect[r]] = "running"]
  /\ runs' = [runs EXCEPT ![r] = "running"]
  /\ leases' = [leases EXCEPT ![runEffect[r]] = "active"]
  /\ UNCHANGED << eventLog, recoveryLog, runEffect, terminalEffects, projectionCursor,
                  paused, cancelled >>

CompleteRun(r, ev) ==
  /\ r \in Runs
  /\ ev \in Events
  /\ ~recovering
  /\ runs[r] = "running"
  /\ runEffect[r] \notin terminalEffects
  /\ effects' = [effects EXCEPT ![runEffect[r]] = "completed"]
  /\ runs' = [runs EXCEPT ![r] = "completed"]
  /\ leases' = [leases EXCEPT ![runEffect[r]] = "none"]
  /\ terminalEffects' = terminalEffects \cup {runEffect[r]}
  /\ eventLog' = Append(eventLog, ev)
  /\ UNCHANGED << recoveryLog, runEffect, projectionCursor, paused, cancelled, recovering >>

FailRun(r, ev) ==
  /\ r \in Runs
  /\ ev \in Events
  /\ ~recovering
  /\ runs[r] = "running"
  /\ runEffect[r] \notin terminalEffects
  /\ effects' = [effects EXCEPT ![runEffect[r]] = "failed"]
  /\ runs' = [runs EXCEPT ![r] = "failed"]
  /\ leases' = [leases EXCEPT ![runEffect[r]] = "none"]
  /\ terminalEffects' = terminalEffects \cup {runEffect[r]}
  /\ eventLog' = Append(eventLog, ev)
  /\ UNCHANGED << recoveryLog, runEffect, projectionCursor, paused, cancelled, recovering >>

ExpireLease(e, ev) ==
  /\ e \in Effects
  /\ ev \in Events
  /\ ~recovering
  /\ effects[e] = "running"
  /\ leases[e] = "active"
  /\ e \notin terminalEffects
  /\ effects' = [effects EXCEPT ![e] = "queued"]
  /\ leases' = [leases EXCEPT ![e] = "expired"]
  /\ eventLog' = Append(eventLog, ev)
  /\ UNCHANGED << recoveryLog, runs, runEffect, terminalEffects, projectionCursor,
                  paused, cancelled >>

DeriveProjection ==
  /\ ~recovering
  /\ projectionCursor < Len(eventLog)
  /\ projectionCursor' = projectionCursor + 1
  /\ UNCHANGED << eventLog, recoveryLog, effects, runs, runEffect, leases, terminalEffects,
                  paused, cancelled, recovering >>

PauseInstance ==
  /\ ~recovering
  /\ ~paused
  /\ paused' = TRUE
  /\ UNCHANGED << eventLog, recoveryLog, effects, runs, runEffect, leases, terminalEffects,
                  projectionCursor, cancelled, recovering >>

ResumeInstance ==
  /\ ~recovering
  /\ paused
  /\ ~cancelled
  /\ paused' = FALSE
  /\ UNCHANGED << eventLog, recoveryLog, effects, runs, runEffect, leases, terminalEffects,
                  projectionCursor, cancelled, recovering >>

CancelInstance(ev) ==
  /\ ev \in Events
  /\ ~recovering
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
  /\ UNCHANGED << recoveryLog, runs, runEffect, leases, projectionCursor, recovering >>

StartRecovery ==
  /\ ~recovering
  /\ recovering' = TRUE
  /\ recoveryLog' = eventLog
  /\ UNCHANGED << eventLog, effects, runs, runEffect, leases, terminalEffects,
                  projectionCursor, paused, cancelled >>

FinishRecovery ==
  /\ recovering
  /\ eventLog' = recoveryLog
  /\ projectionCursor' = Len(recoveryLog)
  /\ recovering' = FALSE
  /\ UNCHANGED << recoveryLog, effects, runs, runEffect, leases, terminalEffects,
                  paused, cancelled >>

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
  \/ StartRecovery
  \/ FinishRecovery

Spec ==
  Init /\ [][Next]_vars

ClaimAny(e) ==
  \E r \in Runs : ClaimEffect(e, r)

StartAny(e) ==
  \E r \in Runs :
    /\ runEffect[r] = e
    /\ StartRun(r)

ProviderTerminalOrRecovered(e) ==
  \E r \in Runs, ev \in Events :
    /\ runEffect[r] = e
    /\ \/ CompleteRun(r, ev)
       \/ FailRun(r, ev)
       \/ ExpireLease(e, ev)

FairSpec ==
  /\ Spec
  /\ WF_vars(DeriveProjection)
  /\ WF_vars(FinishRecovery)
  /\ \A e \in Effects :
       /\ WF_vars(ClaimAny(e))
       /\ WF_vars(StartAny(e))
       /\ WF_vars(ProviderTerminalOrRecovered(e))

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

RecoveryDoesNotReorderEventLog ==
  recovering => eventLog = recoveryLog

EffectTerminalOrNotRunning(e) ==
  \/ effects[e] \in {"blocked", "claimed"}
  \/ e \in terminalEffects
  \/ paused
  \/ cancelled
  \/ recovering

ClaimableEffectEventuallyRunsOrStops(e) ==
  [](Claimable(e) => <>(effects[e] = "running" \/ EffectTerminalOrNotRunning(e)))

RunningEffectEventuallyTerminalsOrRecovers(e) ==
  [](effects[e] = "running" /\ leases[e] = "active" =>
      <>(e \in terminalEffects \/ effects[e] = "queued" \/ cancelled \/ recovering))

ProjectionEventuallyCatchesUp ==
  [](~recovering /\ projectionCursor < Len(eventLog) =>
      <>(projectionCursor = Len(eventLog) \/ recovering))

RecoveryEventuallyFinishes ==
  [](recovering => <>~recovering)

LivenessGoals ==
  /\ \A e \in Effects : ClaimableEffectEventuallyRunsOrStops(e)
  /\ \A e \in Effects : RunningEffectEventuallyTerminalsOrRecovers(e)
  /\ ProjectionEventuallyCatchesUp
  /\ RecoveryEventuallyFinishes

TypeOk ==
  /\ eventLog \in Seq(Events)
  /\ recoveryLog \in Seq(Events)
  /\ effects \in [Effects -> EffectStatuses]
  /\ runs \in [Runs -> RunStatuses]
  /\ runEffect \in [Runs -> Effects]
  /\ leases \in [Effects -> LeaseStatuses]
  /\ terminalEffects \subseteq Effects
  /\ projectionCursor \in Nat
  /\ paused \in BOOLEAN
  /\ cancelled \in BOOLEAN
  /\ recovering \in BOOLEAN

====
