# Legacy Armature Runtime

The existing task/service/script runner is legacy for the statechart workflow
track.

It may still provide useful implementation material:

- Rust workspace and CLI patterns
- process/log capture code
- event/run terminology where it remains accurate
- tests and packaging
- runtime inspection lessons

It should not define the new product model. New implementation work should start
from:

```text
native .armature DSL files
validated workflow IR
durable event queues
append-only transition/effect logs
trusted Rust interpreter
typed effects
status/check/prove UX
```

Compatibility with the old task/service model should be implemented only through
explicit adapters or migration tools.
