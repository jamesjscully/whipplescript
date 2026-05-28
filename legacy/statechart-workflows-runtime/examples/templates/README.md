# Workflow Templates

These are copyable `.whip` starting points. They are intentionally small
and use native local-agent harnessing plus built-in file-backed adapter
shortcuts where useful.

## simple-agent-supervisor.whip

Starts one worker when an `idle` observation says work remains, then reports
typed `finished` completions to a director thread.

Useful local commands:

```sh
store=/tmp/whippletree-template.sqlite
harness=/tmp/whippletree-template-harness.json

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

whip validate examples/templates/simple-agent-supervisor.whip \
  --policy examples/policies/local-file-backed.policy.json \
  --json

whip run examples/templates/simple-agent-supervisor.whip \
  --store "$store" \
  --policy examples/policies/local-file-backed.policy.json \
  --event idle \
  --payload '{"activeRuns":0,"unfinishedItems":1}' \
  --json

whip overview examples/templates/simple-agent-supervisor.whip \
  --store "$store" \
  --policy examples/policies/local-file-backed.policy.json

whip harness once examples/templates/simple-agent-supervisor.whip \
  --store "$store" \
  --config "$harness" \
  --json

whip harness run examples/templates/simple-agent-supervisor.whip \
  --store "$store" \
  --config "$harness" \
  --drive-workflow \
  --max-iterations 10 \
  --json

whip run examples/templates/simple-agent-supervisor.whip \
  --store "$store" \
  --policy examples/policies/local-file-backed.policy.json \
  --json
```

The first `overview` should report the worker as active. After the final
`run`, `overview` should report that the workflow is idle with no queued events
or active invocations. `whip harness status ... --json` shows recent
native invocations, completions, and harness events.
