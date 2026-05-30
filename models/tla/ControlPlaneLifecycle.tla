---- MODULE ControlPlaneLifecycle ----
EXTENDS Naturals, Sequences, FiniteSets

\* This model captures the durable control-plane lifecycle independently of
\* any particular WhippleScript source program. Maude models local rule/effect
\* rewrites; this TLA+ model tracks asynchronous runtime actions, leases,
\* recovery, pause/resume, cancellation, and event-log ordering.

CONSTANTS
  \* @type: Set(Str);
  Effects,
  \* @type: Set(Str);
  Runs,
  \* @type: Set(Str);
  Events,
  \* @type: Set(Str);
  Versions,
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
  completed,
  \* @type: Bool;
  failed,
  \* @type: Bool;
  recovering,
  \* @type: Str;
  activeVersion,
  \* @type: Int;
  revisionEpoch,
  \* @type: Str -> Str;
  effectVersion,
  \* @type: Set(Str);
  cancelRequested,
  \* @type: Seq(Str);
  revisionEvents

vars ==
  << eventLog, recoveryLog, effects, runs, runEffect, leases, terminalEffects,
     projectionCursor, paused, cancelled, completed, failed, recovering,
     activeVersion, revisionEpoch, effectVersion, cancelRequested,
     revisionEvents >>

RevisionVars ==
  << activeVersion, revisionEpoch, effectVersion, cancelRequested,
     revisionEvents >>

EffectStatuses ==
  {"queued", "blocked", "claimed", "running", "completed",
   "failed", "timed_out", "cancelled"}

RunStatuses ==
  {"none", "claimed", "running", "completed", "failed", "timed_out",
   "cancelled", "lease_expired"}

LeaseStatuses ==
  {"none", "active", "released", "expired"}

Init ==
  /\ eventLog = << >>
  /\ recoveryLog = << >>
  /\ effects = [e \in Effects |-> "queued"]
  /\ runs = [r \in Runs |-> "none"]
  /\ runEffect = [r \in Runs |-> CHOOSE e \in Effects : TRUE]
  /\ leases = [r \in Runs |-> "none"]
  /\ terminalEffects = {}
  /\ projectionCursor = 0
  /\ paused = FALSE
  /\ cancelled = FALSE
  /\ completed = FALSE
  /\ failed = FALSE
  /\ recovering = FALSE
  /\ activeVersion = "version1"
  /\ revisionEpoch = 0
  /\ effectVersion = [e \in Effects |-> "version1"]
  /\ cancelRequested = {}
  /\ revisionEvents = << >>

InstanceRunning ==
  /\ ~paused
  /\ ~cancelled
  /\ ~completed
  /\ ~failed

AppendEvent(ev) ==
  /\ ev \in Events
  /\ ~recovering
  /\ eventLog' = Append(eventLog, ev)
  /\ UNCHANGED << effects, runs, runEffect, leases, terminalEffects,
                  projectionCursor, paused, cancelled, completed, failed,
                  recoveryLog, recovering, RevisionVars >>

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
  /\ InstanceRunning
  /\ effects[e] = "queued"
  /\ e \notin cancelRequested
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
                  paused, cancelled, completed, failed, recovering,
                  RevisionVars >>

StartRun(r) ==
  /\ r \in Runs
  /\ ~recovering
  /\ InstanceRunning
  /\ runs[r] = "claimed"
  /\ effects[runEffect[r]] = "claimed"
  /\ effects' = [effects EXCEPT ![runEffect[r]] = "running"]
  /\ runs' = [runs EXCEPT ![r] = "running"]
  /\ leases' = [leases EXCEPT ![r] = "active"]
  /\ UNCHANGED << eventLog, recoveryLog, runEffect, terminalEffects, projectionCursor,
                  paused, cancelled, completed, failed, recovering,
                  RevisionVars >>

CompleteRun(r, ev) ==
  /\ r \in Runs
  /\ ev \in Events
  /\ ~recovering
  /\ runs[r] = "running"
  /\ runEffect[r] \notin terminalEffects
  /\ effects' = [effects EXCEPT ![runEffect[r]] = "completed"]
  /\ runs' = [runs EXCEPT ![r] = "completed"]
  /\ leases' = [leases EXCEPT ![r] = "released"]
  /\ terminalEffects' = terminalEffects \cup {runEffect[r]}
  /\ eventLog' = Append(eventLog, ev)
  /\ UNCHANGED << recoveryLog, runEffect, projectionCursor, paused, cancelled,
                  completed, failed, recovering, activeVersion, revisionEpoch,
                  effectVersion, cancelRequested, revisionEvents >>

FailRun(r, ev) ==
  /\ r \in Runs
  /\ ev \in Events
  /\ ~recovering
  /\ runs[r] = "running"
  /\ runEffect[r] \notin terminalEffects
  /\ effects' = [effects EXCEPT ![runEffect[r]] = "failed"]
  /\ runs' = [runs EXCEPT ![r] = "failed"]
  /\ leases' = [leases EXCEPT ![r] = "released"]
  /\ terminalEffects' = terminalEffects \cup {runEffect[r]}
  /\ eventLog' = Append(eventLog, ev)
  /\ UNCHANGED << recoveryLog, runEffect, projectionCursor, paused, cancelled,
                  completed, failed, recovering, activeVersion, revisionEpoch,
                  effectVersion, cancelRequested, revisionEvents >>

CancelRun(r, ev) ==
  /\ r \in Runs
  /\ ev \in Events
  /\ ~recovering
  /\ runs[r] = "running"
  /\ runEffect[r] \notin terminalEffects
  /\ effects' = [effects EXCEPT ![runEffect[r]] = "cancelled"]
  /\ runs' = [runs EXCEPT ![r] = "cancelled"]
  /\ leases' = [leases EXCEPT ![r] = "released"]
  /\ terminalEffects' = terminalEffects \cup {runEffect[r]}
  /\ eventLog' = Append(eventLog, ev)
  /\ UNCHANGED << recoveryLog, runEffect, projectionCursor, paused, cancelled,
                  completed, failed, recovering, activeVersion, revisionEpoch,
                  effectVersion, cancelRequested, revisionEvents >>

TimeoutRun(r, ev) ==
  /\ r \in Runs
  /\ ev \in Events
  /\ ~recovering
  /\ runs[r] = "running"
  /\ runEffect[r] \notin terminalEffects
  /\ effects' = [effects EXCEPT ![runEffect[r]] = "timed_out"]
  /\ runs' = [runs EXCEPT ![r] = "timed_out"]
  /\ leases' = [leases EXCEPT ![r] = "released"]
  /\ terminalEffects' = terminalEffects \cup {runEffect[r]}
  /\ eventLog' = Append(eventLog, ev)
  /\ UNCHANGED << recoveryLog, runEffect, projectionCursor, paused, cancelled,
                  completed, failed, recovering, activeVersion, revisionEpoch,
                  effectVersion, cancelRequested, revisionEvents >>

ExpireLease(r, ev) ==
  /\ r \in Runs
  /\ ev \in Events
  /\ ~recovering
  /\ runs[r] = "running"
  /\ effects[runEffect[r]] = "running"
  /\ leases[r] = "active"
  /\ runEffect[r] \notin terminalEffects
  /\ effects' = [effects EXCEPT ![runEffect[r]] = "queued"]
  /\ runs' = [runs EXCEPT ![r] = "lease_expired"]
  /\ leases' = [leases EXCEPT ![r] = "expired"]
  /\ eventLog' = Append(eventLog, ev)
  /\ UNCHANGED << recoveryLog, runEffect, terminalEffects, projectionCursor,
                  paused, cancelled, completed, failed, recovering,
                  RevisionVars >>

RetryEffect(e, ev) ==
  /\ e \in Effects
  /\ ev \in Events
  /\ ~recovering
  /\ InstanceRunning
  /\ effects[e] \in {"failed", "timed_out"}
  /\ effects' = [effects EXCEPT ![e] = "queued"]
  /\ terminalEffects' = terminalEffects \ {e}
  /\ eventLog' = Append(eventLog, ev)
  /\ UNCHANGED << recoveryLog, runs, runEffect, leases, projectionCursor,
                  paused, cancelled, completed, failed, recovering,
                  activeVersion, revisionEpoch, effectVersion, cancelRequested,
                  revisionEvents >>

DeriveProjection ==
  /\ ~recovering
  /\ projectionCursor < Len(eventLog)
  /\ projectionCursor' = projectionCursor + 1
  /\ UNCHANGED << eventLog, recoveryLog, effects, runs, runEffect, leases, terminalEffects,
                  paused, cancelled, completed, failed, recovering,
                  RevisionVars >>

PauseInstance ==
  /\ ~recovering
  /\ InstanceRunning
  /\ ~paused
  /\ paused' = TRUE
  /\ UNCHANGED << eventLog, recoveryLog, effects, runs, runEffect, leases, terminalEffects,
                  projectionCursor, cancelled, completed, failed, recovering,
                  RevisionVars >>

ResumeInstance ==
  /\ ~recovering
  /\ paused
  /\ ~cancelled
  /\ ~completed
  /\ ~failed
  /\ paused' = FALSE
  /\ UNCHANGED << eventLog, recoveryLog, effects, runs, runEffect, leases, terminalEffects,
                  projectionCursor, cancelled, completed, failed, recovering,
                  RevisionVars >>

CancelInstance(ev) ==
  /\ ev \in Events
  /\ ~recovering
  /\ ~completed
  /\ ~failed
  /\ ~cancelled
  /\ cancelled' = TRUE
  /\ paused' = TRUE
  /\ eventLog' = Append(eventLog, ev)
  /\ UNCHANGED << recoveryLog, effects, runs, runEffect, leases, terminalEffects,
                  projectionCursor, completed, failed, recovering,
                  RevisionVars >>

CompleteWorkflow(ev) ==
  /\ ev \in Events
  /\ ~recovering
  /\ InstanceRunning
  /\ completed' = TRUE
  /\ eventLog' = Append(eventLog, ev)
  /\ UNCHANGED << recoveryLog, effects, runs, runEffect, leases, terminalEffects,
                  projectionCursor, paused, cancelled, failed, recovering,
                  RevisionVars >>

FailWorkflow(ev) ==
  /\ ev \in Events
  /\ ~recovering
  /\ InstanceRunning
  /\ failed' = TRUE
  /\ eventLog' = Append(eventLog, ev)
  /\ UNCHANGED << recoveryLog, effects, runs, runEffect, leases, terminalEffects,
                  projectionCursor, paused, cancelled, completed, recovering,
                  RevisionVars >>

OldVersionEffect(e) ==
  effectVersion[e] # activeVersion

ActivateRevision(newVersion, ev) ==
  /\ newVersion \in Versions
  /\ ev \in Events
  /\ ~recovering
  /\ InstanceRunning
  /\ newVersion # activeVersion
  /\ activeVersion' = newVersion
  /\ revisionEpoch' = revisionEpoch + 1
  /\ revisionEvents' = Append(revisionEvents, ev)
  /\ eventLog' = Append(eventLog, ev)
  /\ UNCHANGED << recoveryLog, effects, runs, runEffect, leases, terminalEffects,
                  projectionCursor, paused, cancelled, completed, failed,
                  recovering, effectVersion, cancelRequested >>

TerminalCancelQueuedRevisionEffect(e, ev) ==
  /\ e \in Effects
  /\ ev \in Events
  /\ ~recovering
  /\ InstanceRunning
  /\ OldVersionEffect(e)
  /\ effects[e] \in {"queued", "blocked"}
  /\ e \notin terminalEffects
  /\ effects' = [effects EXCEPT ![e] = "cancelled"]
  /\ terminalEffects' = terminalEffects \cup {e}
  /\ eventLog' = Append(eventLog, ev)
  /\ UNCHANGED << recoveryLog, runs, runEffect, leases, projectionCursor,
                  paused, cancelled, completed, failed, recovering,
                  activeVersion, revisionEpoch, effectVersion, cancelRequested,
                  revisionEvents >>

RequestCancelEffect(e, ev) ==
  /\ e \in Effects
  /\ ev \in Events
  /\ ~recovering
  /\ InstanceRunning
  /\ OldVersionEffect(e)
  /\ effects[e] \in {"claimed", "running"}
  /\ e \notin terminalEffects
  /\ cancelRequested' = cancelRequested \cup {e}
  /\ eventLog' = Append(eventLog, ev)
  /\ UNCHANGED << recoveryLog, effects, runs, runEffect, leases, terminalEffects,
                  projectionCursor, paused, cancelled, completed, failed,
                  recovering, activeVersion, revisionEpoch, effectVersion,
                  revisionEvents >>

AcknowledgeCancelRun(r, ev) ==
  /\ r \in Runs
  /\ ev \in Events
  /\ ~recovering
  /\ runs[r] = "running"
  /\ runEffect[r] \in cancelRequested
  /\ runEffect[r] \notin terminalEffects
  /\ effects' = [effects EXCEPT ![runEffect[r]] = "cancelled"]
  /\ runs' = [runs EXCEPT ![r] = "cancelled"]
  /\ leases' = [leases EXCEPT ![r] = "released"]
  /\ terminalEffects' = terminalEffects \cup {runEffect[r]}
  /\ eventLog' = Append(eventLog, ev)
  /\ UNCHANGED << recoveryLog, runEffect, projectionCursor, paused, cancelled,
                  completed, failed, recovering, activeVersion, revisionEpoch,
                  effectVersion, cancelRequested, revisionEvents >>

IgnoreLateCancelAfterTerminal(e) ==
  /\ e \in Effects
  /\ e \in terminalEffects
  /\ e \in cancelRequested
  /\ cancelRequested' = cancelRequested \ {e}
  /\ UNCHANGED << eventLog, recoveryLog, effects, runs, runEffect, leases,
                  terminalEffects, projectionCursor, paused, cancelled,
                  completed, failed, recovering, activeVersion, revisionEpoch,
                  effectVersion, revisionEvents >>

StartRecovery ==
  /\ ~recovering
  /\ recovering' = TRUE
  /\ recoveryLog' = eventLog
  /\ UNCHANGED << eventLog, effects, runs, runEffect, leases, terminalEffects,
                  projectionCursor, paused, cancelled, completed, failed,
                  RevisionVars >>

FinishRecovery ==
  /\ recovering
  /\ eventLog' = recoveryLog
  /\ projectionCursor' = Len(recoveryLog)
  /\ recovering' = FALSE
  /\ UNCHANGED << recoveryLog, effects, runs, runEffect, leases, terminalEffects,
                  paused, cancelled, completed, failed, RevisionVars >>

Next ==
  \/ \E ev \in Events : AppendEvent(ev)
  \/ \E e \in Effects, r \in Runs : ClaimEffect(e, r)
  \/ \E r \in Runs : StartRun(r)
  \/ \E r \in Runs, ev \in Events : CompleteRun(r, ev)
  \/ \E r \in Runs, ev \in Events : FailRun(r, ev)
  \/ \E r \in Runs, ev \in Events : CancelRun(r, ev)
  \/ \E r \in Runs, ev \in Events : TimeoutRun(r, ev)
  \/ \E r \in Runs, ev \in Events : ExpireLease(r, ev)
  \/ \E e \in Effects, ev \in Events : RetryEffect(e, ev)
  \/ DeriveProjection
  \/ PauseInstance
  \/ ResumeInstance
  \/ \E ev \in Events : CancelInstance(ev)
  \/ \E ev \in Events : CompleteWorkflow(ev)
  \/ \E ev \in Events : FailWorkflow(ev)
  \/ \E newVersion \in Versions, ev \in Events : ActivateRevision(newVersion, ev)
  \/ \E e \in Effects, ev \in Events : TerminalCancelQueuedRevisionEffect(e, ev)
  \/ \E e \in Effects, ev \in Events : RequestCancelEffect(e, ev)
  \/ \E r \in Runs, ev \in Events : AcknowledgeCancelRun(r, ev)
  \/ \E e \in Effects : IgnoreLateCancelAfterTerminal(e)
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
       \/ CancelRun(r, ev)
       \/ TimeoutRun(r, ev)
       \/ ExpireLease(r, ev)
       \/ AcknowledgeCancelRun(r, ev)

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

TerminalEffectSetMatchesCurrentStatus ==
  \A e \in Effects :
    (e \in terminalEffects) <=> effects[e] \in {"completed", "failed", "timed_out", "cancelled"}

ProjectionCursorWithinLog ==
  projectionCursor <= Len(eventLog)

RecoveryDoesNotReorderEventLog ==
  recovering => eventLog = recoveryLog

NoActiveLeaseWithoutRunningRun ==
  \A r \in Runs :
    leases[r] = "active" => /\ runs[r] = "running"
                             /\ effects[runEffect[r]] = "running"

NoRunningRunWithoutActiveLease ==
  \A r \in Runs :
    runs[r] = "running" => /\ leases[r] = "active"
                            /\ effects[runEffect[r]] = "running"

NoTerminalRunHasActiveLease ==
  \A r \in Runs :
    runs[r] \in {"completed", "failed", "timed_out", "cancelled", "lease_expired"}
      => leases[r] # "active"

NoReleasedLeaseWithoutTerminalRun ==
  \A r \in Runs :
    leases[r] = "released" => runs[r] \in {"completed", "failed", "timed_out", "cancelled"}

ActiveLeaseForEffect(e) ==
  \E r \in Runs :
    /\ runEffect[r] = e
    /\ runs[r] = "running"
    /\ leases[r] = "active"

EffectTerminalOrNotRunning(e) ==
  \/ effects[e] \in {"blocked", "claimed"}
  \/ e \in terminalEffects
  \/ paused
  \/ cancelled
  \/ completed
  \/ failed
  \/ recovering

ClaimableEffectEventuallyRunsOrStops(e) ==
  [](Claimable(e) => <>(effects[e] = "running" \/ EffectTerminalOrNotRunning(e)))

RunningEffectEventuallyTerminalsOrRecovers(e) ==
  [](effects[e] = "running" /\ ActiveLeaseForEffect(e) =>
      <>(e \in terminalEffects \/ effects[e] = "queued" \/ cancelled \/ completed \/ failed \/ recovering))

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

EventSeqOk(seq) ==
  \A i \in 1..Len(seq) : seq[i] \in Events

TypeOk ==
  /\ EventSeqOk(eventLog)
  /\ EventSeqOk(recoveryLog)
  /\ EventSeqOk(revisionEvents)
  /\ effects \in [Effects -> EffectStatuses]
  /\ runs \in [Runs -> RunStatuses]
  /\ runEffect \in [Runs -> Effects]
  /\ leases \in [Runs -> LeaseStatuses]
  /\ terminalEffects \subseteq Effects
  /\ projectionCursor \in Nat
  /\ paused \in BOOLEAN
  /\ cancelled \in BOOLEAN
  /\ completed \in BOOLEAN
  /\ failed \in BOOLEAN
  /\ recovering \in BOOLEAN
  /\ activeVersion \in Versions
  /\ revisionEpoch \in Nat
  /\ effectVersion \in [Effects -> Versions]
  /\ cancelRequested \subseteq Effects

RevisionEpochMatchesEvents ==
  revisionEpoch = Len(revisionEvents)

CancelRequestIsNotTerminalByItself ==
  \A e \in cancelRequested :
    effects[e] = "cancelled" => e \in terminalEffects

NoConflictingInstanceTerminalStates ==
  /\ ~(cancelled /\ completed)
  /\ ~(cancelled /\ failed)
  /\ ~(completed /\ failed)

NoNewEffectfulWorkAfterTerminalInstance ==
  (cancelled \/ completed \/ failed) => \A e \in Effects : ~Claimable(e)

ConstInit ==
  /\ Effects = {"effectA", "effectB"}
  /\ Runs = {"runA", "runB"}
  /\ Events = {"eventA", "eventB"}
  /\ Versions = {"version1", "version2"}
  /\ Dependencies = {
       <<"effectA", "succeeds", "effectB">>,
       <<"effectA", "fails", "effectB">>,
       <<"effectA", "completes", "effectB">>
     }

SafetyInvariants ==
  /\ TypeOk
  /\ EveryRunReferencesEffect
  /\ NoRunWithoutRunningEffect
  /\ NoClaimedRunWithoutClaimedEffect
  /\ NoClaimableEffectHasUnsatisfiedDeps
  /\ NoNewEffectfulWorkWhilePaused
  /\ NoNewEffectfulWorkAfterTerminalInstance
  /\ NoConflictingInstanceTerminalStates
  /\ TerminalEffectSetMatchesCurrentStatus
  /\ ProjectionCursorWithinLog
  /\ RecoveryDoesNotReorderEventLog
  /\ RevisionEpochMatchesEvents
  /\ CancelRequestIsNotTerminalByItself
  /\ NoActiveLeaseWithoutRunningRun
  /\ NoRunningRunWithoutActiveLease
  /\ NoTerminalRunHasActiveLease
  /\ NoReleasedLeaseWithoutTerminalRun

====
