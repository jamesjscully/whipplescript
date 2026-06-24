# Loft Integration

Status: historical adapter context

Direction note: this file predates the proposed `std.tracker` package direction
being workshopped in
[`decision-records/0002-work-tracker-package.md`](decision-records/0002-work-tracker-package.md).
Read this as historical adapter context, not the current product boundary.

Current authoring should prefer the `std.tracker` issue surface described in
the decision record. This file records the older Loft-specific integration
shape and provider effect contract for historical context only; examples here
are not normative for the current language surface.

Loft is a separate kernel. It owns project work truth:

```text
issues
dependencies
ready-work derivation
semantic conflicts
Git-friendly immutable transaction history
local issue leases
```

WhippleScript owns orchestration truth:

```text
programs
instances
events
facts
effects
provider runs
agent lifecycle
```

Loft is core integration for WhippleScript because local issue/work coordination is
the default serious use case. It should remain usable without WhippleScript.

## Capability Surface

WhippleScript talks to Loft through a registered capability:

```text
loft.ready
loft.show
loft.claim
loft.renew
loft.release
loft.note
loft.transition
loft.evidence
loft.resource_intent
loft.complete
loft.fail
loft.conflicts
```

The first implementation calls the `loft` CLI with `--json` using the v0.1
Loft spec. A daemon or library API may follow.

## Rule Shape

Author-facing sketch:

```whipplescript
requires capability loft

rule implement_ready_issue
  when {
    loft has ready issue as issue
    worker is available
  }
=> {
  claim issue with loft as claim

  after claim succeeds {
    tell worker """markdown
    Implement this Loft issue:

    {{ claim.issue.title }}

    {{ claim.issue.body }}
    """
  }
}
```

Lowered effects:

```text
loft.claim(issue)
loft.claim(issue) --succeeds--> agent.tell(worker, prompt)
```

The agent turn should not start unless the Loft claim succeeds. The triggered
turn follows the core agent contract: a record-once terminal plus evidence, with
in-turn activity recorded as evidence rather than rule-matchable facts (Proposal
A; see [`admission-and-idempotency.md`](admission-and-idempotency.md) and
[`agent-harness.md`](agent-harness.md)).

## Claim Effect Contract

Source syntax:

```whipplescript
claim issue with loft as claim
```

Lowered effect:

```text
kind: loft.claim
binding: claim
```

Input:

```text
issue_id
issue_version?
actor
lease_duration?
command_id
expect_heads?
idempotency_key
```

Success output:

```text
LoftClaimSucceeded {
  claim_id
  issue
  lease_expires_at?
  command_id
  evidence_refs
}
```

Failure output:

```text
LoftClaimFailed {
  issue_id
  reason
  recoverable
  current_status?
  current_actor?
  conflicts?
  command_id?
  evidence_refs
}
```

Timeout output:

```text
LoftClaimTimedOut {
  issue_id
  timeout
  recoverable
  evidence_refs
}
```

Typing rules:

```whipplescript
after claim succeeds {
  // claim : LoftClaimSucceeded
}

after claim fails {
  // claim : LoftClaimFailed
}

after claim completes {
  // claim : LoftClaimSucceeded | LoftClaimFailed | LoftClaimTimedOut
}
```

`claim.issue` is available only in the success branch. Failure branches must use
failure fields such as `claim.issue_id`, `claim.reason`, and `claim.conflicts`.

## Lease Relationship

```text
Loft lease = this actor is working on issue X
WhippleScript effect lease = this worker is running effect Y
```

These are different leases. Do not collapse them.

## Idempotency

WhippleScript effect IDs should map to Loft `command_id` values. Retrying an
WhippleScript effect must not create duplicate Loft transactions.

## Failure Semantics

- If `loft.claim` fails, no agent turn is enqueued.
- If the agent turn fails, policy decides whether to release the issue, renew
  the lease, add a note, or ask a human.
- If Loft has semantic conflicts, ready work excludes conflicted issues and
  WhippleScript may pause or ask for human resolution.
- Expired Loft leases do not imply durable issue failure.

## CLI Contract

The v0 WhippleScript adapter follows the Loft v0.1 agent-facing CLI:

```bash
loft show iss_abc --json
loft claim --ready --actor agent-a --ttl 1800s --json
loft claim iss_abc --actor agent-a --ttl 1800s --json
loft renew lea_abc --ttl 1800s --json
loft release lea_abc --json
loft note iss_abc "message" --lease-id lea_abc --json
loft set iss_abc status in_progress --lease-id lea_abc --json
loft evidence add iss_abc --lease-id lea_abc --kind whipplescript.trace --artifact REF --json-data trace-summary.json --json
loft set iss_abc resource_intent '{"reads":[],"writes":[]}' --lease-id lea_abc --json
loft complete lea_abc --reason done --json
loft fail lea_abc --note "tests failed" --release --json
```

WhippleScript keeps a command ID on the provider request for durable evidence and
future idempotency. The current Loft CLI does not accept `--command-id`, so
the command adapter omits it by default; `CommandLoftClient::pass_command_id`
exists for later Loft builds or alternate providers that support that flag.

## Fixture Source Of Truth

WhippleScript should add the Loft repository as `vendor/loft` once the Loft
v0.1 spec and conformance fixtures are tracked in that repository. Until then,
`scripts/check-real-providers.sh` accepts `WHIPPLESCRIPT_LOFT_REPO=/path/to/loft`
and verifies that `spec/loft-v0.1.md` is tracked and the repo is clean before
declaring real-provider readiness.

When the Loft repo is ready, use:

```sh
scripts/add-loft-submodule.sh /path/to/loft vendor/loft
```

The helper refuses to add a local repo unless `spec/loft-v0.1.md`,
`fixtures/whipplescript/v0.1/manifest.json`, and every manifest-listed fixture are
tracked and the Loft worktree is clean.

To stage WhippleScript's local compatibility fixtures into a local Loft repo for
review, use:

```sh
scripts/stage-loft-fixtures.sh /path/to/loft
```

That helper writes `fixtures/whipplescript/v0.1/manifest.json` plus every
manifest-listed fixture into the Loft repo and prints the Loft-side status.
It does not commit those files.

To produce a reviewable patch for the Loft repo instead of committing there,
use:

```sh
scripts/export-loft-source-patch.sh /path/to/loft
```

The default patch path is `target/loft-source-fixtures.patch`. The exporter
stages compatibility fixtures first, refuses to overwrite an existing patch
unless `WHIPPLESCRIPT_OVERWRITE_LOFT_PATCH=1` is set, and refuses to disturb staged
Loft changes under the spec or fixture paths.

`scripts/check-loft-fixtures.sh` validates the manifest and fixture files
from `WHIPPLESCRIPT_LOFT_FIXTURE_DIR`, `vendor/loft`, or the local compatibility
copy under `examples/loft-fixtures/v0.1`. The Rust fixture test asserts the
coverage tags in the Loft manifest, including rich issue shape,
`issue_status`, lease-scoped mutations, structured evidence, resource intent,
lifecycle complete/fail, retryable error details, and partial lifecycle
recovery.

To summarize the current Loft handoff state and next commands, run:

```sh
scripts/loft-handoff-report.sh /path/to/loft
```

The default Loft repo path is `WHIPPLESCRIPT_LOFT_REPO` or `../loft`. The default
report path is `target/loft-handoff-report.md`.

For local validation before Loft is installed globally, run:

```sh
scripts/check-local-loft-cli.sh ../loft
```

That command installs the local Loft checkout into `target/loft-cli-venv`,
uses an isolated temporary Loft workspace, exercises the CLI lifecycle, and
then runs WhippleScript's real Loft smoke with an `WHIPPLESCRIPT_LOFT_CLI` wrapper
pointing at that workspace.

WhippleScript expects source-of-truth Loft conformance fixtures at:

```text
vendor/loft/fixtures/whipplescript/v0.1/
```

Until the Loft submodule publishes those fixtures, WhippleScript carries local
compatibility fixtures at:

```text
examples/loft-fixtures/v0.1/
```

Required fixture files:

```text
claim-ready-succeeded.json
claim-already-leased-failed.json
renew-succeeded.json
release-succeeded.json
note-succeeded.json
transition-succeeded.json
```

The script-level fixture contract lives in `scripts/loft-fixtures-lib.sh` and
is shared by submodule addition, fixture conformance, submodule readiness,
fixture staging, and real-provider readiness checks.

To preflight a local Loft repo before using it as a source fixture repo, run:

```sh
scripts/check-loft-source-repo.sh /path/to/loft
```

That command verifies the Loft repo has a clean worktree plus tracked spec and
fixture files.

Run fixture conformance with:

```sh
scripts/check-loft-fixtures.sh
```

The script prefers `WHIPPLESCRIPT_LOFT_FIXTURE_DIR`, then submodule fixtures, then
the local compatibility fixtures. Set `WHIPPLESCRIPT_LOFT_FIXTURE_DIR` to force a
specific fixture source. Set `WHIPPLESCRIPT_REQUIRE_LOFT_FIXTURES=1` in CI to fail
when no fixture source is available. Set
`WHIPPLESCRIPT_REQUIRE_LOFT_SUBMODULE_FIXTURES=1` when CI must prove the
source-of-truth `vendor/loft` fixture path exists instead of using local
compatibility fixtures.

After adding the submodule, run:

```sh
scripts/check-loft-submodule-readiness.sh
```

That command verifies `vendor/loft` is a registered submodule, contains the
tracked Loft spec and fixture files, has a clean worktree, and passes strict
fixture conformance.

## Resource Intent

Loft issues may carry optional resource intent metadata:

```text
reads:  [resource_id...]
writes: [resource_id...]
```

Loft should not own any external resource graph. It may store resource intent so
schedulers or providers can avoid obvious conflicts before an agent writes code.
