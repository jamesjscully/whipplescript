# 0004: Provider Compatibility for Tracker Packages

Status: proposed

## Decision

External systems should integrate through providers that advertise capabilities.
WhippleScript should normalize the orchestration-relevant subset and expose
provider differences explicitly rather than forcing GitHub, Linear, Jira, and
local providers into an identical model.

## Capability Shape

Providers should report operations as native, emulated, local-only, or
unsupported:

```text
claim
release
close
cancel
reopen
dependencies
comments
evidence
state_tokens
watch_changes
resource_intent
```

Rules can branch on capability where behavior matters.

## Rationale

GitHub, Linear, and Jira have overlapping concepts but different semantics.
For example, "claim" may be an atomic local transaction, a best-effort
check-then-assign flow, a status transition, or unavailable. Hiding this makes
workflows look portable while creating race conditions and surprising behavior.

The right target is portable intent with explicit capability disclosure.

## Consequences

- Builtin local providers should be the reference semantics.
- External providers can be useful even when incomplete.
- The language should make unsupported capabilities fail early at check time
  when a workflow requires them.
- Provider bindings own native mapping details such as Jira workflow states,
  Linear statuses, GitHub labels, and metadata placement.

## Risks

- Capability matrices can become noisy if exposed directly to authors.
- Emulated operations may still be too weak for some workflows.
- Sync conflict handling needs careful design before webhook/push sync.

## Open Questions

- What is the exact `requires capability` syntax for package/provider features?
- Should provider capabilities be compile-time, runtime, or both?
- What is the minimum viable GitHub provider that is honest about races?
