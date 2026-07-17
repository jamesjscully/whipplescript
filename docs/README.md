# WhippleScript Docs

WhippleScript coordinates durable work across agents, humans, packages, queues,
timers, scripts, and child workflows. The same durable kernel runs locally and
on the edge inside a Cloudflare Durable Object; see
[Runtime & operations](runtime-operations.md) for `whip deploy` and the
checkpoint/restore operator surface. The docs are written for two readers:
humans using the website and coding agents reading Markdown directly.

**Version scope:** the Markdown in this repository tracks the latest released
line of the CLI. Release users should read the docs from the matching Git
tag when exact CLI flags or JSON fields matter. The current published CLI
version is `0.1.0`; the implementation-stage label printed by `whip --help`
is an internal progress marker, not a separate compatibility version.
In-progress work on the next release line lands on a separate branch and is
not reflected here unless a page says otherwise.

If you are a coding agent, start with the local companion skill at
`skills/whipplescript-author/SKILL.md`
([web link](https://github.com/jamesjscully/whipplescript/blob/main/skills/whipplescript-author/SKILL.md)).
It contains the operational route map, feature selection table, canonical
patterns, command loop, and links into the deeper pages.

Design records and implementation trackers live in [`spec/`](https://github.com/jamesjscully/whipplescript/tree/main/spec/). They
are not required for normal authoring.

## Reading Paths

| Goal | Read |
| --- | --- |
| Install the CLI | [Install](install.md) |
| Install and run a checked workflow | [Quickstart](quickstart.md) |
| Learn the runtime nouns | [Concepts](concepts.md) |
| Build a real workflow from an empty file | [Tutorial](tutorial.md) |
| Choose the right authoring pattern | [Manual](manual.md) |
| Look up exact syntax | [Language reference](language-reference.md) |
| Pick a known-good example | [Examples](examples.md) |
| Run or inspect instances | [CLI reference](api-reference.md) |
| Consume machine-readable output | [JSON reference](json-reference.md) |
| Work with the Rust crates | [Rust API reference](rust-api.md) |
| Interpret errors | [Diagnostics guide](diagnostics.md) |
| Operate running workflows | [Runtime & operations](runtime-operations.md) |
| Configure agents, providers, and packages | [Providers & packages](providers.md) |
| Fix a failing first run | [Troubleshooting](troubleshooting.md) |
| Check stability and caveats | [Current state](current-state.md) |

## Agent Route

Use this route when the task is "write or fix a workflow"
(the [Agent Guide](agent-guide.md) covers the same route in more depth):

1. Read `skills/whipplescript-author/SKILL.md`.
2. Pick the closest checked example from [Examples](examples.md).
3. Use the [Manual](manual.md) for pattern choice.
4. Use the [Language reference](language-reference.md) only for exact grammar.
5. Validate with `whip check` and fixture-backed `whip dev`.
6. Inspect runtime state with `status`, `effects`, `runs`, `diagnostics`,
   `evidence`, and `trace --check` before changing prompts.

## Website

This directory is also a MkDocs site. From the repository root:

```sh
python3 -m pip install -r docs/requirements.txt
mkdocs serve
```

Check the site in strict mode with:

```sh
scripts/check-docs-site.sh
```

The Markdown files remain canonical so agents can navigate them without a site
build.

## Verification

Docs that present complete workflows or cataloged example commands are checked
by scripts:

```sh
scripts/check-docs-quickstart.sh
scripts/check-docs-examples.sh
scripts/check-docs-snippets.sh
```

Code block convention:

- `sh` blocks are runnable commands unless they contain placeholders such as
  `<instance>`.
- `whip` blocks are complete source only when introduced as a full file or
  covered by one of the scripts above.
- `json` and `text` blocks are output or schema/diagnostic shapes unless the
  surrounding text says to save them as input.
- `...` and angle-bracket placeholders mark illustrative fragments.

Complete workflow examples should either live in `examples/` or be covered by
one of the scripts above.
