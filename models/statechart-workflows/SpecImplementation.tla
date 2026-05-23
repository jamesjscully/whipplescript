------------------------ MODULE SpecImplementation ------------------------
EXTENDS Naturals, FiniteSets, Sequences

\* Hand-written Phase 1 model for the spec implementation workflow.
\*
\* This is intentionally an abstract WorkflowIR/runtime model, not a parser
\* model. Coerce outputs are represented by nondeterministic transitions:
\* worker/quality completions may pass or fail, and the director may choose to
\* start worker work, start quality work, ask a human, wait, or finish.

CONSTANTS WorkItems, MaxWorkers, MaxQuality

ASSUME WorkItems # {}
ASSUME MaxWorkers > 0
ASSUME MaxQuality > 0

VARIABLES
  phase,
  itemStatus,
  started,
  humanReviews,
  failureCount,
  lastEffect,
  lastTarget

vars ==
  << phase, itemStatus, started, humanReviews, failureCount, lastEffect,
     lastTarget >>

Phases == {"watching", "choosing", "done", "blocked"}

Statuses ==
  { "todo",
    "worker_active",
    "ready_for_quality",
    "quality_active",
    "done",
    "blocked",
    "human_review" }

VisibleStatuses == Statuses

DeclaredEffects ==
  { "none",
    "idleNudge",
    "coerceChooseNextStep",
    "coerceClassifyRun",
    "startWorker",
    "startQuality",
    "planMarkReadyForQuality",
    "planMarkDone",
    "planMarkBlocked",
    "sendDirector",
    "askHuman" }

DeclaredTargets == {"none", "worker", "quality", "director", "human", "plan"}

ActiveWorkers ==
  { item \in WorkItems : itemStatus[item] = "worker_active" }

ActiveQuality ==
  { item \in WorkItems : itemStatus[item] = "quality_active" }

Unfinished ==
  { item \in WorkItems : itemStatus[item] # "done" }

RunnableWorkerItems ==
  { item \in WorkItems : itemStatus[item] = "todo" }

RunnableQualityItems ==
  { item \in WorkItems : itemStatus[item] = "ready_for_quality" }

BlockedItems ==
  { item \in WorkItems :
      itemStatus[item] = "blocked" \/ itemStatus[item] = "human_review" }

Init ==
  /\ phase = "watching"
  /\ itemStatus = [item \in WorkItems |-> "todo"]
  /\ started = {}
  /\ humanReviews = {}
  /\ failureCount = 0
  /\ lastEffect = "none"
  /\ lastTarget = "none"

SetEffect(effect, target) ==
  /\ lastEffect' = effect
  /\ lastTarget' = target

IdleNudge ==
  /\ phase = "watching"
  /\ ActiveWorkers = {}
  /\ ActiveQuality = {}
  /\ Unfinished # {}
  /\ phase' = "choosing"
  /\ UNCHANGED << itemStatus, started, humanReviews, failureCount >>
  /\ SetEffect("idleNudge", "none")

StartWorker(item) ==
  /\ phase = "choosing"
  /\ item \in RunnableWorkerItems
  /\ Cardinality(ActiveWorkers) < MaxWorkers
  /\ phase' = "watching"
  /\ itemStatus' = [itemStatus EXCEPT ![item] = "worker_active"]
  /\ started' = started \cup {item}
  /\ UNCHANGED << humanReviews, failureCount >>
  /\ SetEffect("startWorker", "worker")

StartQuality(item) ==
  /\ phase = "choosing"
  /\ item \in RunnableQualityItems
  /\ Cardinality(ActiveQuality) < MaxQuality
  /\ phase' = "watching"
  /\ itemStatus' = [itemStatus EXCEPT ![item] = "quality_active"]
  /\ UNCHANGED << started, humanReviews, failureCount >>
  /\ SetEffect("startQuality", "quality")

AskHumanForBlocked(item) ==
  /\ phase = "choosing"
  /\ item \in BlockedItems
  /\ phase' = "watching"
  /\ itemStatus' = [itemStatus EXCEPT ![item] = "human_review"]
  /\ humanReviews' = humanReviews \cup {item}
  /\ UNCHANGED << started, failureCount >>
  /\ SetEffect("askHuman", "human")

WaitForExternalProgress ==
  /\ phase = "choosing"
  /\ RunnableWorkerItems = {}
  /\ RunnableQualityItems = {}
  /\ Unfinished # {}
  /\ phase' = "watching"
  /\ UNCHANGED << itemStatus, started, humanReviews, failureCount >>
  /\ SetEffect("sendDirector", "director")

FinishWorkflow ==
  /\ phase = "choosing"
  /\ Unfinished = {}
  /\ phase' = "done"
  /\ UNCHANGED << itemStatus, started, humanReviews, failureCount >>
  /\ SetEffect("none", "none")

WorkerCompletes(item) ==
  /\ phase = "watching"
  /\ item \in ActiveWorkers
  /\ phase' = "watching"
  /\ itemStatus' = [itemStatus EXCEPT ![item] = "ready_for_quality"]
  /\ UNCHANGED << started, humanReviews, failureCount >>
  /\ SetEffect("planMarkReadyForQuality", "plan")

WorkerFails(item) ==
  /\ phase = "watching"
  /\ item \in ActiveWorkers
  /\ phase' = "watching"
  /\ itemStatus' = [itemStatus EXCEPT ![item] = "blocked"]
  /\ failureCount' = failureCount + 1
  /\ UNCHANGED << started, humanReviews >>
  /\ SetEffect("planMarkBlocked", "plan")

QualityPasses(item) ==
  /\ phase = "watching"
  /\ item \in ActiveQuality
  /\ phase' = "choosing"
  /\ itemStatus' = [itemStatus EXCEPT ![item] = "done"]
  /\ UNCHANGED << started, humanReviews, failureCount >>
  /\ SetEffect("planMarkDone", "plan")

QualityFails(item) ==
  /\ phase = "watching"
  /\ item \in ActiveQuality
  /\ phase' = "watching"
  /\ itemStatus' = [itemStatus EXCEPT ![item] = "human_review"]
  /\ humanReviews' = humanReviews \cup {item}
  /\ failureCount' = failureCount + 1
  /\ UNCHANGED << started >>
  /\ SetEffect("askHuman", "human")

TerminalStutter ==
  /\ phase \in {"done", "blocked"}
  /\ UNCHANGED vars

Next ==
  \/ IdleNudge
  \/ \E item \in WorkItems : StartWorker(item)
  \/ \E item \in WorkItems : StartQuality(item)
  \/ \E item \in WorkItems : AskHumanForBlocked(item)
  \/ WaitForExternalProgress
  \/ FinishWorkflow
  \/ \E item \in WorkItems : WorkerCompletes(item)
  \/ \E item \in WorkItems : WorkerFails(item)
  \/ \E item \in WorkItems : QualityPasses(item)
  \/ \E item \in WorkItems : QualityFails(item)
  \/ TerminalStutter

Spec ==
  Init /\ [][Next]_vars /\ WF_vars(IdleNudge)

TypeOK ==
  /\ phase \in Phases
  /\ itemStatus \in [WorkItems -> Statuses]
  /\ started \subseteq WorkItems
  /\ humanReviews \subseteq WorkItems
  /\ failureCount \in Nat
  /\ lastEffect \in DeclaredEffects
  /\ lastTarget \in DeclaredTargets

MaxActiveWorker ==
  Cardinality(ActiveWorkers) <= MaxWorkers

MaxActiveQuality ==
  Cardinality(ActiveQuality) <= MaxQuality

NoDoneWithUnfinishedWork ==
  phase = "done" => Unfinished = {}

StartedItemsAreVisible ==
  \A item \in started : itemStatus[item] \in VisibleStatuses

ActiveItemsWereStarted ==
  \A item \in WorkItems :
    itemStatus[item] \in
      {"worker_active", "ready_for_quality", "quality_active", "done",
       "blocked", "human_review"}
      => item \in started

QualityOnlyAfterWorker ==
  \A item \in WorkItems :
    itemStatus[item] \in {"ready_for_quality", "quality_active", "done"}
      => item \in started

HumanReviewsAreVisible ==
  humanReviews \subseteq
    { item \in WorkItems : itemStatus[item] \in {"human_review", "done"} }

DeclaredEffectsOnly ==
  /\ lastEffect \in DeclaredEffects
  /\ lastTarget \in DeclaredTargets

====
