# Trace Conformance

Status: first-pass contract

Runtime traces are the bridge between the executable formal models and the Rust
implementation. Every runtime integration test should be able to emit an
ordered trace and pass it through a conformance checker.

The initial Rust checker lives in:

```text
crates/whippletree-kernel/src/trace.rs
```

It validates these invariants:

```text
event sequence numbers are contiguous
dependencies reference known effects
effects are not claimed while paused or cancelled
effects are not claimed before dependency predicates are satisfied
runs start only for claimed effects
lease expiry marks the active run stale and makes the effect queueable again
terminal completions must come from the live run for the effect
terminal completions from stale runs are rejected
terminal completions reference known effects
terminal completions are not duplicated
cancelled instances cannot resume or start new runs
```

This checker is intentionally abstract. It does not know SQL table names,
provider-specific payloads, or source-language syntax. Runtime code should
lower concrete events into these trace records before conformance checking.

The checker should grow alongside the store and kernel. In particular, later
stages should add:

```text
lease renewal checks
retry attempt identity checks
blocked-by-policy and blocked-by-capacity traces
rule commit and projection trace checks
artifact/evidence reference checks
```
