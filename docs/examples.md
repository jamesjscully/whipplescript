# Examples

Use this page to choose which checked workflow to open next.

| Example | Shows | Credentials needed? | Start here when... |
| --- | --- | --- | --- |
| [`minimal-noop.whip`](../examples/minimal-noop.whip) | Smallest fact/rule workflow. | No. | You want to see the minimum source shape. |
| [`multi-agent-bounded-concurrency.whip`](../examples/multi-agent-bounded-concurrency.whip) | Multiple logical agents, capacity, and handoff shape. | No for `check`; fixture/local validation only for runtime experiments. | You want to understand agent routing and capacity. |
| [`provider-language-e2e.whip`](../examples/provider-language-e2e.whip) | Multi-provider routing, fixture agent turns, typed review, and assertions. | No. | You want the best first end-to-end example. |
| [`human-review.whip`](../examples/human-review.whip) | Human inbox request and answer flow. | No. | You want to mix automation with manual decisions. |
| [`codex-poem-coerce-review.whip`](../examples/codex-poem-coerce-review.whip) | Agent work followed by typed review. | Fixture for local validation; real provider setup is experimental. | You want a compact agent-plus-review workflow. |
| [`multi-provider-poem-review.whip`](../examples/multi-provider-poem-review.whip) | Codex/Claude/Pi-style logical providers with shared review policy. | Fixture for local validation; real provider setup is experimental. | You want to compare provider routing patterns. |
| [`loft-worker-with-review.whip`](../examples/loft-worker-with-review.whip) | Loft issue claim, agent work, BAML-style review, and human fallback. | Advanced/experimental for real Loft/BAML use. | You want the shape of a work-tracker integration. |
| [`plugin-memory.whip`](../examples/plugin-memory.whip) | Plugin import and capability-call shape. | No for source checks; provider behavior depends on plugin binding. | You want to see how plugins appear in source. |
| [`revision-ticket-v1.whip`](../examples/revision-ticket-v1.whip) and [`revision-ticket-v2.whip`](../examples/revision-ticket-v2.whip) | In-flight workflow revision. | No. | You want to test `whip revise`. |

## Recommended Path

1. Run the [Quickstart](quickstart.md) with `minimal-noop.whip`.
2. Follow the [Tutorial](tutorial.md) with `provider-language-e2e.whip`.
3. Open `multi-agent-bounded-concurrency.whip` to study a compact multi-agent
   shape.
4. Try `human-review.whip` if your workflow needs approval gates.
5. Read [Providers And Plugins](providers.md) before attempting real providers.

## Commands

Check any example:

```sh
whip check examples/provider-language-e2e.whip
```

Run fixture-backed examples with `dev`:

```sh
whip --store .whipplescript/examples.sqlite \
  dev examples/provider-language-e2e.whip \
  --provider fixture \
  --until idle \
  --json
```

Inspect with the returned `instance_id`:

```sh
whip --store .whipplescript/examples.sqlite status <instance_id>
whip --store .whipplescript/examples.sqlite facts <instance_id>
whip --store .whipplescript/examples.sqlite effects <instance_id>
```
