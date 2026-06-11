# Tutorial: build a triage workflow

In this tutorial you write a workflow from an empty file and run it to a
completed instance. The workflow seeds two bug tickets, has an agent propose
a triage plan for each, asks a human to sign off on the high-severity one,
and completes or fails depending on the answer.

Along the way you will use most of the language: typed facts, a seed table,
an agent, effect sequencing with `after`, a human review gate, guards, and
workflow terminals. The fixture provider stands in for a real agent, so no
credentials are needed.

[Install the CLI](install.md) first if you have not.

## 1. Declare the data

Create `triage.whip`. Start with the workflow's name, its terminal
contracts, and the fact types it works over:

```whip
workflow TicketTriage

output result TriageDecision
failure error TriageBlocked

class Ticket {
  id string
  title string
  severity string
  status string
}

class TriagedTicket {
  id string
  title string
  severity string
  plan string
  status "triaged"
}

class TriageDecision {
  decision string
  decidedBy string
}

class TriageBlocked {
  reason string
}
```

Two things to note:

- `output` and `failure` declare what this workflow produces when it ends.
  Every workflow must end (or be explicitly tagged as a service); the checker
  enforces this below.
- `status "triaged"` is a literal type: a `TriagedTicket` can only ever have
  that status. Literal fields are the idiomatic way to model small state
  machines.

## 2. Declare the agent and seed data

```whip
agent triager {
  provider fixture
  profile "repo-reader"
  capacity 1
}

table tickets as Ticket [
  {
    id "T-31"
    title "Login returns 500 on empty password"
    severity "high"
    status "open"
  }
  {
    id "T-32"
    title "Typo in footer copyright"
    severity "low"
    status "open"
  }
]
```

The agent binds to the `fixture` provider for now; switching to a real
provider later changes this one line, not the rules. The table seeds two
`Ticket` facts when the instance starts.

## 3. Write the triage rule

```whip
rule triage_open_ticket
  when Ticket as ticket where ticket.status == "open"
  when triager is available
=> {
  tell triager as turn """markdown
  Suggest an owner and a fix plan for this ticket:

  {{ ticket.title }} (severity: {{ ticket.severity }})
  """

  after turn succeeds as triaged {
    done ticket -> record TriagedTicket {
      id ticket.id
      title ticket.title
      severity ticket.severity
      plan triaged.summary
      status "triaged"
    }
  }
}
```

This is the core pattern of the language:

- The `when` clauses say what the rule waits for: an open ticket, and free
  capacity on the agent.
- `tell` does not call the agent. It records a durable `agent.tell` effect;
  a worker executes it later through the provider.
- `after turn succeeds as triaged` runs when the effect completes
  successfully — minutes or days later — with the turn's output bound to
  `triaged`. Output is only visible inside the `after` block.
- `done ticket -> record ...` consumes the open ticket and records its
  replacement in the same atomic commit, so a ticket can never be both open
  and triaged.

## 4. Let the checker catch the missing ending

Check what you have so far:

```sh
whip check triage.whip
```

```text
error: workflow `TicketTriage` has no rule that reaches `complete` or `fail`
  = help: add a rule that runs `complete <output> { ... }` or `fail <failure> { ... }`,
          or tag the workflow `@service` if it intentionally runs forever
```

The checker is right: tickets get triaged, and then nothing ever finishes
the workflow. Add the human gate and the two endings.

## 5. Add the human approval gate

```whip
rule request_signoff
  when TriagedTicket as ticket where ticket.severity == "high"
=> {
  askHuman as signoff """markdown
  {{ ticket.title }} was triaged with this plan:

  {{ ticket.plan }}

  Approve or reject the plan.
  """
}

rule approve_plan
  when human answered signoff as answer where answer.choice == "approve"
=> {
  complete result {
    decision answer.choice
    decidedBy answer.answered_by
  }
}

rule reject_plan
  when human answered signoff as answer where answer.choice == "reject"
=> {
  fail error {
    reason "triage plan rejected"
  }
}
```

`askHuman` creates a `human.ask` effect that surfaces as an inbox item. When
someone answers it, the runtime records a fact that `human answered ...`
rules match; the answer payload carries `choice`, `text`, and `answered_by`.

Finally, add assertions — executable claims about the finished run that
`dev` evaluates for you:

```whip
assert count(Ticket where status == "open") == 0
assert count(TriagedTicket) == 2
```

Check again; it should pass and print the compiled rule graph:

```sh
whip check triage.whip
```

## 6. Run it

```sh
mkdir -p .whipplescript
whip --store .whipplescript/triage.sqlite \
  dev triage.whip --provider fixture --until idle
```

`dev` reports both assertions passing, and `status` shows the instance is
still `running` — it is waiting for the sign-off:

```sh
whip --store .whipplescript/triage.sqlite status <instance_id>
whip --store .whipplescript/triage.sqlite inbox
```

```text
key_... instance=ins_... severity=normal created=...
  Login returns 500 on empty password was triaged with this plan: ...
```

Only the high-severity ticket produced an inbox item; T-32 was triaged
without ceremony.

## 7. Answer and finish

Answer the review, then step the instance so the decision rules see it:

```sh
whip --store .whipplescript/triage.sqlite inbox answer <item_id> --choice approve --by alice
whip --store .whipplescript/triage.sqlite step <instance_id> --program triage.whip
whip --store .whipplescript/triage.sqlite status <instance_id>
```

```text
instance ins_... completed
```

The durable record of the whole run is now queryable:

```sh
whip --store .whipplescript/triage.sqlite facts <instance_id>
```

```text
TriagedTicket          {"id":"T-31","plan":"...","severity":"high","status":"triaged",...}
TriagedTicket          {"id":"T-32","plan":"...","severity":"low","status":"triaged",...}
agent.turn.completed   {"agent":"triager","provider":"fixture","status":"completed",...}
agent.turn.completed   {"agent":"triager","provider":"fixture","status":"completed",...}
human.answer.received  {"answer":{"answered_by":"alice","choice":"approve",...},...}
```

`effects` shows the two agent turns and the human ask, all `completed`;
`trace --check` verifies the lifecycle conforms to the runtime model.

## 8. Try the failure path

Run a fresh instance and reject the plan:

```sh
whip --store .whipplescript/triage.sqlite \
  dev triage.whip --provider fixture --until idle
whip --store .whipplescript/triage.sqlite inbox
whip --store .whipplescript/triage.sqlite inbox answer <item_id> --choice reject --by alice
whip --store .whipplescript/triage.sqlite step <instance_id> --program triage.whip
whip --store .whipplescript/triage.sqlite status <instance_id>
```

```text
instance ins_... failed
```

The `reject_plan` rule executed `fail error { ... }`. Both instances — one
completed, one failed — coexist in the same store with full histories.

## Where to go next

- Swap `provider fixture` for a real provider once you have one configured:
  [providers & plugins](providers.md).
- Add a typed model review with `coerce` instead of trusting the turn
  summary: see [`examples/queue-worker-with-review.whip`](../examples/queue-worker-with-review.whip).
- Read the [manual](manual.md) for authoring guidance — retries, branching,
  composition — and the [language reference](language-reference.md) for the
  full construct list.
