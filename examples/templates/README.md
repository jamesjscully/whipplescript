# Workflow Templates

These are copyable `.armature` starting points. They are intentionally small
and use native local-agent harnessing plus built-in file-backed adapter
shortcuts where useful.

## simple-agent-supervisor.armature

Starts one worker when an `idle` observation says work remains, then reports
typed `finished` completions to a director thread.

Useful local commands:

```sh
store=/tmp/armature-template.sqlite
harness=/tmp/armature-template-harness.json

cat >"$harness" <<'JSON'
{
  "agents": {
    "worker": {
      "provider": "command",
      "command": ["sh", "-c", "printf 'worker complete'"]
    }
  }
}
JSON

armature validate examples/templates/simple-agent-supervisor.armature \
  --policy examples/policies/local-file-backed.policy.json \
  --json

armature run examples/templates/simple-agent-supervisor.armature \
  --store "$store" \
  --policy examples/policies/local-file-backed.policy.json \
  --event idle \
  --payload '{"activeRuns":0,"unfinishedItems":1}' \
  --json

armature overview examples/templates/simple-agent-supervisor.armature \
  --store "$store" \
  --policy examples/policies/local-file-backed.policy.json

armature harness once examples/templates/simple-agent-supervisor.armature \
  --store "$store" \
  --config "$harness" \
  --json

armature harness run examples/templates/simple-agent-supervisor.armature \
  --store "$store" \
  --config "$harness" \
  --drive-workflow \
  --max-iterations 10 \
  --json

armature run examples/templates/simple-agent-supervisor.armature \
  --store "$store" \
  --policy examples/policies/local-file-backed.policy.json \
  --json
```

The first `overview` should report the worker as active. After the final
`run`, `overview` should report that the workflow is idle with no queued events
or active invocations. `armature harness status ... --json` shows recent
native invocations, completions, and harness events.
