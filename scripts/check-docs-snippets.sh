#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

WHIP=(cargo run --quiet --manifest-path "$ROOT/Cargo.toml" -p whipplescript --)

"$ROOT/scripts/check-docs-quickstart.sh" >/dev/null
"$ROOT/scripts/check-docs-examples.sh" >/dev/null

cat > "$TMPDIR/tutorial-triage.whip" <<'WHIP'
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

rule request_signoff
  when TriagedTicket as ticket where ticket.severity == "high"
=> {
  askHuman as signoff choices ["approve", "reject"] """markdown
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

assert count(Ticket where status == "open") == 0
assert count(TriagedTicket) == 2
WHIP

"${WHIP[@]}" check "$TMPDIR/tutorial-triage.whip" >/dev/null

printf 'docs snippets check passed\n'
