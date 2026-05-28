# Docket Integration

Status: draft

Docket is a separate kernel. It owns project work truth:

```text
issues
dependencies
ready-work derivation
semantic conflicts
Git-friendly immutable transaction history
local issue leases
```

Armature owns orchestration truth:

```text
programs
instances
events
facts
effects
provider runs
agent lifecycle
```

Docket is core integration for Armature because local issue/work coordination is
the default serious use case. It should remain usable without Armature.

## Capability Surface

Armature talks to Docket through a registered capability:

```text
docket.ready
docket.claim
docket.renew
docket.release
docket.show
docket.note
docket.setStatus
docket.close
docket.conflicts
```

The first implementation can call the `docket` CLI with `--json`. A daemon or
library API may follow.

## Rule Shape

Author-facing sketch:

```armature
requires capability docket

rule implement_ready_issue
  when docket has ready issue as issue
  when worker is available
=> {
  claim issue with docket as claim

  after claim succeeds {
    tell worker """
    Implement this Docket issue:

    {{ claim.issue.title }}

    {{ claim.issue.body }}
    """
  }
}
```

Lowered effects:

```text
docket.claim(issue)
docket.claim(issue) --succeeds--> agent.tell(worker, prompt)
```

The agent turn should not start unless the Docket claim succeeds.

## Claim Effect Contract

Source syntax:

```armature
claim issue with docket as claim
```

Lowered effect:

```text
kind: docket.claim
binding: claim
```

Input:

```text
issue_id
issue_version?
claimant
lease_duration?
command_id
idempotency_key
```

Success output:

```text
DocketClaimSucceeded {
  claim_id
  issue
  lease_expires_at?
  command_id
  evidence_refs
}
```

Failure output:

```text
DocketClaimFailed {
  issue_id
  reason
  recoverable
  current_status?
  current_claimant?
  conflicts?
  command_id?
  evidence_refs
}
```

Timeout output:

```text
DocketClaimTimedOut {
  issue_id
  timeout
  recoverable
  evidence_refs
}
```

Typing rules:

```armature
after claim succeeds {
  // claim : DocketClaimSucceeded
}

after claim fails {
  // claim : DocketClaimFailed
}

after claim completes {
  // claim : DocketClaimSucceeded | DocketClaimFailed | DocketClaimTimedOut
}
```

`claim.issue` is available only in the success branch. Failure branches must use
failure fields such as `claim.issue_id`, `claim.reason`, and `claim.conflicts`.

## Lease Relationship

```text
Docket lease = this actor is working on issue X
Armature effect lease = this worker claimed effect Y
Thoth lease = this actor may write governed resource Z
```

These are different leases. Do not collapse them.

## Idempotency

Armature effect IDs should map to Docket `command_id` values. Retrying an
Armature effect must not create duplicate Docket transactions.

## Failure Semantics

- If `docket.claim` fails, no agent turn is enqueued.
- If the agent turn fails, policy decides whether to release the issue, renew
  the lease, add a note, or ask a human.
- If Docket has semantic conflicts, ready work excludes conflicted issues and
  Armature may pause or ask for human resolution.
- Expired Docket leases do not imply durable issue failure.

## Thoth Bridge

Docket issues may carry Thoth resource intent metadata:

```text
reads:  [resource_id...]
writes: [resource_id...]
```

Docket should not own Thoth's resource graph. It may store resource intent so
schedulers can avoid obvious conflicts before an agent writes code.
