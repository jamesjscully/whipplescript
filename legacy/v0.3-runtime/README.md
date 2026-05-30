# WhippleScript v0.3 Runtime

This directory contains the previous WhippleScript task/service/script runner.

It has been moved out of the active workspace because the current project is
being rebuilt around native `.whip` statechart workflows, validated
WorkflowIR, typed effects, durable event queues, and trusted Rust adapters.

The v0.3 runtime may still be mined for:

- Rust CLI and daemon structure
- process and log capture
- event/run terminology where it remains accurate
- tests and packaging patterns
- migration and compatibility adapter ideas

It should not define the new workflow language, runtime semantics, permission
model, CLI surface, or implementation plan.
