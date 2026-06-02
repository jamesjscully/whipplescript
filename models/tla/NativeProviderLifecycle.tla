---- MODULE NativeProviderLifecycle ----
EXTENDS Naturals, Sequences, FiniteSets

\* Focused native-provider lifecycle fixture. The broad control-plane model
\* covers claim/run/recovery ordering; this model isolates provider evidence,
\* cancellation acknowledgement, and required artifact-capture failure.

CONSTANTS
  \* @type: Set(Str);
  Effects,
  \* @type: Set(Str);
  Events

VARIABLES
  \* @type: Str -> Str;
  effectStatus,
  \* @type: Set(Str);
  cancelRequested,
  \* @type: Set(Str);
  cancelAcknowledged,
  \* @type: Set(Str);
  providerEvidence,
  \* @type: Set(Str);
  artifactCaptureFailed,
  \* @type: Set(Str);
  terminalAppended,
  \* @type: Seq(Str);
  eventLog

vars ==
  << effectStatus, cancelRequested, cancelAcknowledged, providerEvidence,
     artifactCaptureFailed, terminalAppended, eventLog >>

EffectStatuses ==
  {"running", "completed", "failed", "cancelled"}

TerminalStatuses ==
  {"completed", "failed", "cancelled"}

Init ==
  /\ effectStatus = [e \in Effects |-> "running"]
  /\ cancelRequested = {}
  /\ cancelAcknowledged = {}
  /\ providerEvidence = {}
  /\ artifactCaptureFailed = {}
  /\ terminalAppended = {}
  /\ eventLog = << >>

AppendEvent(ev) ==
  /\ ev \in Events
  /\ eventLog' = Append(eventLog, ev)

RequestCancellation(e, ev) ==
  /\ e \in Effects
  /\ ev \in Events
  /\ effectStatus[e] = "running"
  /\ e \notin terminalAppended
  /\ cancelRequested' = cancelRequested \cup {e}
  /\ AppendEvent(ev)
  /\ UNCHANGED << effectStatus, cancelAcknowledged, providerEvidence,
                  artifactCaptureFailed, terminalAppended >>

AcknowledgeCancellation(e, ev) ==
  /\ e \in Effects
  /\ ev \in Events
  /\ effectStatus[e] = "running"
  /\ e \in cancelRequested
  /\ e \notin terminalAppended
  /\ cancelAcknowledged' = cancelAcknowledged \cup {e}
  /\ AppendEvent(ev)
  /\ UNCHANGED << effectStatus, cancelRequested, providerEvidence,
                  artifactCaptureFailed, terminalAppended >>

RecordProviderEvidence(e, ev) ==
  /\ e \in Effects
  /\ ev \in Events
  /\ effectStatus[e] = "running"
  /\ e \notin terminalAppended
  /\ providerEvidence' = providerEvidence \cup {e}
  /\ AppendEvent(ev)
  /\ UNCHANGED << effectStatus, cancelRequested, cancelAcknowledged,
                  artifactCaptureFailed, terminalAppended >>

RecoverProviderTerminal(e, ev) ==
  /\ e \in Effects
  /\ ev \in Events
  /\ effectStatus[e] = "running"
  /\ e \in providerEvidence
  /\ e \notin terminalAppended
  /\ effectStatus' = [effectStatus EXCEPT ![e] = "completed"]
  /\ terminalAppended' = terminalAppended \cup {e}
  /\ AppendEvent(ev)
  /\ UNCHANGED << cancelRequested, cancelAcknowledged, providerEvidence,
                  artifactCaptureFailed >>

CompleteProvider(e, ev) ==
  /\ e \in Effects
  /\ ev \in Events
  /\ effectStatus[e] = "running"
  /\ e \notin artifactCaptureFailed
  /\ e \notin terminalAppended
  /\ effectStatus' = [effectStatus EXCEPT ![e] = "completed"]
  /\ terminalAppended' = terminalAppended \cup {e}
  /\ AppendEvent(ev)
  /\ UNCHANGED << cancelRequested, cancelAcknowledged, providerEvidence,
                  artifactCaptureFailed >>

RequiredArtifactCaptureFailure(e, ev) ==
  /\ e \in Effects
  /\ ev \in Events
  /\ effectStatus[e] = "running"
  /\ e \notin terminalAppended
  /\ artifactCaptureFailed' = artifactCaptureFailed \cup {e}
  /\ effectStatus' = [effectStatus EXCEPT ![e] = "failed"]
  /\ terminalAppended' = terminalAppended \cup {e}
  /\ AppendEvent(ev)
  /\ UNCHANGED << cancelRequested, cancelAcknowledged, providerEvidence >>

CancelAfterAcknowledgement(e, ev) ==
  /\ e \in Effects
  /\ ev \in Events
  /\ effectStatus[e] = "running"
  /\ e \in cancelAcknowledged
  /\ e \notin terminalAppended
  /\ effectStatus' = [effectStatus EXCEPT ![e] = "cancelled"]
  /\ terminalAppended' = terminalAppended \cup {e}
  /\ AppendEvent(ev)
  /\ UNCHANGED << cancelRequested, cancelAcknowledged, providerEvidence,
                  artifactCaptureFailed >>

Next ==
  \/ \E e \in Effects, ev \in Events : RequestCancellation(e, ev)
  \/ \E e \in Effects, ev \in Events : AcknowledgeCancellation(e, ev)
  \/ \E e \in Effects, ev \in Events : RecordProviderEvidence(e, ev)
  \/ \E e \in Effects, ev \in Events : RecoverProviderTerminal(e, ev)
  \/ \E e \in Effects, ev \in Events : CompleteProvider(e, ev)
  \/ \E e \in Effects, ev \in Events : RequiredArtifactCaptureFailure(e, ev)
  \/ \E e \in Effects, ev \in Events : CancelAfterAcknowledgement(e, ev)

Spec ==
  Init /\ [][Next]_vars

EventSeqOk(seq) ==
  \A i \in 1..Len(seq) : seq[i] \in Events

TypeOk ==
  /\ effectStatus \in [Effects -> EffectStatuses]
  /\ cancelRequested \subseteq Effects
  /\ cancelAcknowledged \subseteq Effects
  /\ providerEvidence \subseteq Effects
  /\ artifactCaptureFailed \subseteq Effects
  /\ terminalAppended \subseteq Effects
  /\ EventSeqOk(eventLog)

TerminalSetMatchesStatus ==
  \A e \in Effects :
    (e \in terminalAppended) <=> effectStatus[e] \in TerminalStatuses

CancellationAcknowledgementDoesNotFabricateTerminal ==
  \A e \in Effects :
    (/\ e \in cancelAcknowledged
     /\ e \notin terminalAppended)
      => effectStatus[e] = "running"

RequiredArtifactFailurePreventsSuccess ==
  \A e \in Effects :
    e \in artifactCaptureFailed => effectStatus[e] # "completed"

NoDuplicateTerminalOutcome ==
  \A e \in Effects :
    e \in terminalAppended => Cardinality({effectStatus[e]} \cap TerminalStatuses) = 1

ProviderEvidenceRecoveryIsTerminalWhenCompleted ==
  \A e \in Effects :
    (/\ e \in providerEvidence
     /\ effectStatus[e] = "completed")
      => e \in terminalAppended

ConstInit ==
  /\ Effects = {"effectA", "effectB"}
  /\ Events = {"eventA", "eventB"}

SafetyInvariants ==
  /\ TypeOk
  /\ TerminalSetMatchesStatus
  /\ CancellationAcknowledgementDoesNotFabricateTerminal
  /\ RequiredArtifactFailurePreventsSuccess
  /\ NoDuplicateTerminalOutcome
  /\ ProviderEvidenceRecoveryIsTerminalWhenCompleted

====
