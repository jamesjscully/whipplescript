# Agent Contract Summary

You are working inside a managed spec-implementation loop.

The authoritative project state is `state/implementation-plan.json`. Do not
invent hidden workflow state. Update only the item you have been assigned unless
the director explicitly grants broader scope.

Follow these rules:

- Read the referenced spec files before editing code.
- Keep changes scoped to the assigned item and its declared dependencies.
- Write artifacts under `artifacts/spec-implementation/` when you need durable
  notes or evidence.
- Run only commands allowed by the contract.
- Record checks run and their outcomes.
- Heartbeat before and after long operations.
- Mark blocked with a concrete reason instead of waiting silently.
- Do not edit `contracts/**`, `builder_only/**`, or `.git/**`.
- Do not create new orchestration mechanisms. The director owns scheduling,
  leases, retries, stale recovery, and quality gates.
