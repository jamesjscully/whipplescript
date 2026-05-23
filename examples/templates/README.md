# Workflow Templates

These are copyable `.armature` starting points. They are intentionally small
and use the built-in file-backed adapter shortcuts where possible.

## simple-agent-supervisor.armature

Starts one worker when an `idle` observation says work remains, then reports
typed `finished` completions to a director thread.

Useful local commands:

```sh
store=/tmp/armature-template.sqlite
agents=/tmp/armature-template-agents.json

armature validate examples/templates/simple-agent-supervisor.armature \
  --agent-file "$agents" \
  --policy examples/policies/local-file-backed.policy.json \
  --json

armature run examples/templates/simple-agent-supervisor.armature \
  --store "$store" \
  --agent-file "$agents" \
  --policy examples/policies/local-file-backed.policy.json \
  --event idle \
  --payload '{"activeRuns":0,"unfinishedItems":1}' \
  --json

armature overview examples/templates/simple-agent-supervisor.armature \
  --store "$store" \
  --agent-file "$agents" \
  --policy examples/policies/local-file-backed.policy.json

armature emit examples/templates/simple-agent-supervisor.armature \
  --store "$store" \
  --agent-file "$agents" \
  --event finished \
  --payload '{"id":"run-1","name":"worker-1","status":"succeeded","stdoutTail":"","stderrTail":"","exitCode":0}' \
  --json

armature run examples/templates/simple-agent-supervisor.armature \
  --store "$store" \
  --agent-file "$agents" \
  --policy examples/policies/local-file-backed.policy.json \
  --json
```

The first `overview` should report the worker as active. After the final
`run`, `overview` should report that the workflow is idle with no queued events
or active invocations. The agent JSON file should contain an invocation record,
a completion record, and a director message.
