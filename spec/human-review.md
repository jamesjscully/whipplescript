# Human Review And Inbox

Status: historical/current implementation note, target design superseded

Direction note: this file describes the current `askHuman` / inbox surface.
The target package design moves this use case under
[`std.messaging`](messaging.md) plus typed [`std.ingress`](event-ingress.md)
signals. There should not be a long-term `std.human` or `std.inbox` package
abstraction unless a later design finds a non-messaging semantic core.

Human review is core because restricted automation needs a clear escape hatch.
`askHuman` must create visible work for a person, not a hidden pending effect.

## Core Effects

```text
human.ask
human.notify
human.approve
```

`human.ask` creates an inbox item:

```text
question_id
instance_id
created_by_rule
prompt
choices?
freeform_allowed
severity
related_effects
related_artifacts
status
```

## Inbox CLI

```sh
whip inbox
whip inbox show <question>
whip inbox answer <question> --choice <choice>
whip inbox answer <question> --text <text>
whip inbox dismiss <question>
```

Answers append events to the target instance. Rules consume those events like
any other external observation.

## UX Requirements

The inbox should answer:

```text
what needs me?
why does it need me?
what happens if I answer yes/no?
which issue/resource/agent/run is involved?
what evidence should I inspect?
```

The first implementation can be CLI-only. A hosted/UI surface can come later.
