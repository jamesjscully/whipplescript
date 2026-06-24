# Construct Interoperability Examples

Status: draft examples

These examples are aspirational. They are not current syntax guarantees. The
goal is to keep the construct grammar grounded in concrete workflows where
multiple packages must interoperate through typed resources, projections,
events, effects, capabilities, providers, and terminal outputs.

## 1. Tracker + Memory + Agent

```whip
use std.tracker
use std.memory
use std.agent

tracker backlog { provider github }
memory pool project_memory { provider builtin }

agent coder {
  provider codex
  profile repo-writer
}

rule implement_ready_issue
  when backlog has ready issue as issue
  when coder is available
=> {
  recall from project_memory for issue as context

  after context succeeds as memory {
    tell coder with context memory "Resolve {{ issue.title }}"
  }
}
```

Interop pressure: tracker provides `Projection<Issue>`, memory consumes the
issue, recall provides `EffectHandle<MemoryContext>`, and agent context is
explicit.

## 2. Schedule + Tracker

```whip
use std.time
use std.tracker

signal triage.tick {
  scheduled_at time
}

source clock as daily_triage {
  every weekday at 09:00
  timezone "America/New_York"
  missed coalesce

  observe as tick
  emit triage.tick {
    scheduled_at tick.scheduled_at
  }
}

tracker backlog { provider linear }

rule file_daily_triage_issue
  when triage.tick as tick
=> {
  create issue in backlog {
    title "Daily triage"
    body "Review ready and blocked issues."
  } as created
}
```

Interop pressure: `std.time` contributes a `clock` source provider,
explicitly maps clock observations into signal facts, and tracker creation
consumes the signal-driven rule context.

## 3. Ingress + Tracker + Agent

```whip
use std.ingress
use std.tracker
use std.agent

signal github.issue_labeled {
  issue_id string
  label string
  title string
  body string
}

source http as github_webhook {
  signal github.issue_labeled
  path "/github/issues"
  auth hmac

  observe as delivery
  emit github.issue_labeled {
    issue_id delivery.body.issue_id
    label delivery.body.label
    title delivery.body.title
    body delivery.body.body
  }
}

tracker backlog { provider github }
agent triager { provider codex profile repo-reader }

rule triage_bug_label
  when github.issue_labeled as e where e.label == "bug"
  when triager is available
=> {
  link e to backlog issue e.issue_id as issue

  after issue succeeds as tracked {
    tell triager "Triage {{ tracked.title }}"
  }
}
```

Interop pressure: `std.ingress` contributes an `http` source provider,
explicitly maps the delivery observation into a typed signal, tracker links the
signal payload to an issue resource, and agent work consumes the typed issue.

## 4. Memory + Schema Coercion + Tracker

```whip
use std.tracker
use std.memory
use std.agent
use std.coercion

tracker backlog { provider github }
memory pool project_memory { provider builtin }
agent coder { provider codex profile repo-writer }

coerce RouteIssue(issue Issue, memory MemoryContext) -> IssueRoute

rule route_issue
  when backlog has ready issue as issue
=> {
  recall from project_memory for issue as memory

  after memory succeeds as m {
    decide RouteIssue(issue, m) as route
  }

  after route succeeds as route_decision {
    case route_decision {
      NeedsHuman as h => { comment issue { body h.reason } }
      ReadyForAgent as ready => {
        tell coder with context ready.context "Implement {{ issue.title }}"
      }
    }
  }
}
```

Interop pressure: multiple package outputs feed one typed schema-coercion
effect, and the typed output routes back into tracker or agent operations.

## 5. Agent + Script + Tracker + Messaging

```whip
use std.tracker
use std.script
use std.messaging
use std.agent

tracker backlog { provider github }

script run_tests {
  command "scripts/test.sh"
  output TestReport
}

channel ops {
  provider slack
  destination "#builds"
}

agent coder { provider codex profile repo-writer }

rule implement_and_validate
  when backlog has ready issue as issue
  when coder is available
=> {
  claim issue as lease

  after lease succeeds {
    tell coder "Fix {{ issue.title }}" as turn
  }

  after turn succeeds {
    run run_tests as tests
  }

  after tests succeeds as report where report.ok {
    finish issue { summary report.summary }
  }

  after tests succeeds as report where !report.ok {
    send via ops {
      text "Tests failed: {{ report.summary }}"
    } as sent
  }
}
```

Interop pressure: tracker lease gates agent work, script returns typed output,
messaging consumes failed validation output, and tracker finish consumes
successful validation output.

## 6. Ingress + Schedule + Tracker + Messaging

```whip
use std.ingress
use std.time
use std.tracker
use std.messaging

signal deploy.failed {
  service string
  env string
  error string
}

signal ops.digest_due {
  scheduled_at time
}

source http as deploy_webhook {
  path "/deploy"
  observe as delivery
  emit deploy.failed {
    service delivery.body.service
    env delivery.body.env
    error delivery.body.error
  }
}

source clock as quiet_hours_digest {
  every day at 08:00
  timezone "America/New_York"
  missed coalesce

  observe as tick
  emit ops.digest_due {
    scheduled_at tick.scheduled_at
  }
}

tracker incidents { provider jira }
channel oncall { provider pagerduty }

rule record_deploy_failure
  when deploy.failed as failure
=> {
  create issue in incidents {
    title "Deploy failed: {{ failure.service }}"
    body failure.error
    label failure.env
  } as incident
}

rule page_on_digest
  when ops.digest_due
  when incidents has open critical issue as issue
=> {
  send via oncall {
    text "{{ issue.title }}"
  } as page
}
```

Interop pressure: ingress and schedule are both `source` providers that produce
typed signal facts; tracker consumes one signal and provides projections for
messaging.

## 7. Two Trackers + Memory

```whip
use std.tracker
use std.memory

tracker github_backlog {
  provider github
  repo "org/app"
}

tracker linear_backlog {
  provider linear
  team "platform"
}

memory pool project_memory { provider builtin }

rule mirror_high_priority_issue
  when github_backlog has ready issue as gh_issue
  when gh_issue.labels contains "p0"
=> {
  recall from project_memory for gh_issue as context

  after context succeeds as memory {
    create issue in linear_backlog {
      title gh_issue.title
      body "{{ gh_issue.body }}\n\nContext:\n{{ memory.summary }}"
      external_ref gh_issue.id
    } as linear_issue
  }
}
```

Interop pressure: multiple instances of the same construct family must remain
namespaced, and packages need a common or adaptable issue interface.

## Modeling Targets

These examples suggest the next formal targets:

```text
Projection<T> feeds rule bindings.
EffectHandle<T> feeds after-success bindings.
Operation<I, O> can consume another package's terminal output only when types match.
Multiple resources of the same construct family are disambiguated by name.
SignalSource<T> produces ordinary signal facts, not direct rule execution.
Every lowered operation still requires runtime package/provider authorization.
```

## Interface Graph Coverage

The examples should become package-interoperability fixtures over these graph
checks:

```text
1. tracker -> memory -> agent:
   Projection<backlog, Issue> provides Value<Issue>; recall consumes
   Value<Issue> and returns EffectHandle<MemoryContext>; agent tell consumes
   explicit Value<MemoryContext>.

2. schedule -> tracker:
   source clock provides SignalSource<triage.tick>; runtime appends a signal
   fact; tracker create consumes the signal-bound Value<T>.

3. ingress -> tracker -> agent:
   source http validates and maps an external observation into a signal fact;
   tracker link consumes Value<T.issue_id> and produces typed issue output for
   the agent.

4. memory + schema coercion -> tracker/agent:
   coercion input fields require Value<Issue> and Value<MemoryContext>; branch
   payloads must be assignable to the operation that consumes them.

5. tracker lease -> agent -> script -> tracker/messaging:
   every handoff crosses an EffectHandle<T> / TerminalOutput<T> edge; failed
   script output cannot satisfy the tracker finish operation.

6. ingress + schedule + tracker + messaging:
   both ingress and schedule contribute source providers that produce ordinary
   typed signal facts; neither package can directly fire `page_on_digest`.

7. two trackers + memory:
   `github_backlog` and `linear_backlog` remain distinct Resource identities;
   bare `Issue` compatibility is not enough to choose the target tracker.
```

Negative fixtures should cover the paired failures: ambiguous bare projection
with two trackers, feeding `MemoryContext` where `Issue` is required, using raw
provider JSON as terminal output, missing runtime provider binding after a
valid compile, and a schedule/ingress source that attempts direct rule
execution.

## Formal Coverage

The abstract Maude suite
`models/maude/tests/construct-interop-examples.maude` gives each example one
positive composition search and one paired negative search. These checks model
the typed interface graph, not full package implementations.

Covered formally:

```text
typed projections become rule bindings
durable signal payloads become rule bindings
terminal success outputs become typed values
operations require compatible typed inputs before they are ready
two-input operations require all declared inputs
same-family tracker resources remain distinct by resource identity
signal sources cannot directly fire rules
missing typed links block downstream operations
```

Not yet covered formally:

```text
concrete tracker, memory, agent, script, messaging, schedule, and ingress
  package lowering
provider-specific behavior
guards and branch predicates inside these workflows
resource-specific lifecycle semantics such as leases, claims, reservations, and
  finish/close transitions
```
