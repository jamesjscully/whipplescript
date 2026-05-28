# Statechart Workflow Architecture

Status: design proposal

Statechart workflows should be implemented as the new primary Whippletree product
surface for this track. The old Whippletree daemon/task implementation may be
reused opportunistically, but it should not define the conceptual model.

The new core is:

```text
native .whip source files
validated workflow IR
durable event queues
append-only transition/effect logs
trusted Rust interpreter
typed effects
typed synchronous value calls
runtime status
optional formal verification
```

Storage should be SQLite-backed from the first runtime slice. Durable queues and
logs are not an implementation detail to bolt on later.

The system exists to provide constrained operational meaning:

```text
statechart authoring
statechart validation
durable workflow execution
capability-aware effects
generated BAML artifacts for coerce declarations
workflow status/debugging
formal model generation
optional verification gates
```

The workflow runtime must not support arbitrary TypeScript or shell as workflow
logic.

The workflow expression language is deliberately small. BAML-compatible types
are schemas, not a commitment to implement a general-purpose data language.
The supported expression primitives are defined in
[expression-primitives.md](expression-primitives.md). Complex ranking,
summarization, parsing, multimodal work, and domain policy should be expressed
as typed `coerce` functions or adapter capabilities.

## Candidate Architectures

### A. Minimal Interpreter First

Build the workflow system directly around a custom parser, normalized IR, static
validator, and durable interpreter.

```text
.whip source
  -> logos lexer
  -> rowan lossless syntax tree
  -> typed AST / lowering
  -> static validator
  -> workflow IR
  -> BAML artifact generation
  -> durable interpreter
  -> adapters
  -> workflow status
```

Strengths:

- fastest path to a usable workflow runtime
- tight fit with Whippletree's existing runtime objects
- small number of moving parts
- easy to run locally

Risks:

- semantics can drift if the IR is not defined early
- verification can become a retrofit if delayed too long
- implementation pressure may encourage ad hoc escape hatches

### B. Verification-First Compiler

Define the IR and formal semantics first, then build the runtime around that
model.

```text
workflow IR schema
  -> formal transition semantics
  -> hand-written model
  -> generated model target
  -> validator
  -> interpreter
```

Strengths:

- forces crisp semantics before runtime complexity accumulates
- makes capability and concurrency invariants explicit
- reduces risk of building a language that cannot be verified

Risks:

- slower path to a running product loop
- formal tooling can dominate the early implementation cycle
- Veil or another backend may impose constraints before authoring ergonomics are
  proven

### C. XState/SCXML-Compatible Frontend

Use existing statechart vocabulary or interchange formats while still executing
with a restricted Rust interpreter.

```text
.whip source
  -> statechart frontend
  -> XState/SCXML-shaped IR
  -> custom validator
  -> custom interpreter
  -> optional visualizer
```

Strengths:

- borrows mature statechart concepts
- can align with existing visualization and documentation language
- easier for some users to recognize

Risks:

- XState's native execution model assumes JavaScript guards/actions
- SCXML is broad and may carry more surface area than needed
- compatibility may constrain security choices

## Selected Direction

The recommended architecture is:

**Minimal interpreter first, with verification-shaped semantics from day one.**

That means:

- use the native `.whip` statechart DSL as the initial authoring format
- define a normalized IR before implementing the interpreter
- write a hand-authored formal model before runtime behavior hardens
- keep the runtime small and effect-based
- avoid arbitrary user code inside the interpreter
- make every language construct lowerable to a transition-system model
- add generated verification after the runtime skeleton proves useful

The goal is not to make formal tooling block every early edit. The goal is to
use formal modeling as design backpressure before the implementation goes too
far down a path with ambiguous semantics.

## System Components

### 1. Whippletree Source Parser

The parser reads one workflow source file and produces a lossless rowan syntax
tree plus typed lowering diagnostics. This follows the newer BAML language
tooling pattern: a simple lexer, a hand-written recursive-descent parser with
recovery, a lossless syntax tree, then semantic lowering.

Responsibilities:

- tokenize source with `logos`
- build a rowan syntax tree that preserves trivia and malformed tokens
- parse machine metadata, data declarations, agents, capabilities, BAML-shaped
  `class`/`enum`/`coerce` declarations, states, transitions, actions, and
  invariants
- preserve useful source spans for diagnostics
- reject malformed syntax
- avoid evaluating user code

Non-responsibilities:

- capability enforcement
- runtime execution
- BAML model calls
- filesystem or network effects beyond reading the source file and declared
  includes, if includes are later allowed

The parser owns source spans and syntax only. Guard expressions, interpolations,
action normalization, and invariants lower according to
[source-to-ir.md](source-to-ir.md).

### 2. BAML Artifact Generator

The generator turns Whippletree `class`, `enum`, and `coerce` declarations into
normal generated BAML artifacts.

Responsibilities:

- generate BAML enum/class/function declarations
- validate that workflow calls reference declared coerce functions
- expose coerce input/output schemas to the workflow validator
- make coerce outputs available as typed values in the IR
- write generated `baml_src` artifacts that can be served by `baml-cli serve`

Non-responsibilities:

- using BAML as a control-flow runtime
- allowing BAML to return arbitrary executable plans
- requiring generated TypeScript clients for normal `coerce` execution

### 3. Workflow IR

The IR is the central contract between parser, validator, runtime, adapters, and
verification generators.

The IR should be serializable and stable enough to inspect. The initial shape is
defined in [workflow-ir.md](workflow-ir.md). At minimum, it contains:

```text
workflow metadata
declared agents
declared capabilities
declared events
typed data schema
coerce function schemas
states
transitions
guards
actions
invariants
source spans
```

The source syntax may evolve, but the IR should remain small and explicit.

### 4. Static Validator

The validator checks that the IR is executable and safe enough to start.

Responsibilities:

- validate state graph structure
- validate event and payload references
- validate action schemas
- validate capability references
- validate concurrency limits
- validate BAML call schemas
- validate failure paths and default blocked behavior
- validate invariants are known or expressible in the supported invariant
  subset

The validator should produce diagnostics that are useful to coding agents, not
just compiler authors.

### 5. Durable Interpreter

The interpreter executes validated IR. It processes one workflow transition at a
time and records all state changes durably.

Responsibilities:

- load workflow state
- admit events
- evaluate guards
- perform deterministic data updates
- record intended effects
- commit transitions
- dispatch effects idempotently
- record effect outcomes
- expose workflow status

The interpreter must not execute arbitrary workflow-authored host code.

### 6. Effect Registry

Effects are the only way a workflow changes the outside world.

Initial effects:

```text
send       send a message to an existing agent or thread
start      begin native harness-managed agent work
coerce     call BAML to produce typed data from input
assign     update workflow-local durable data
askHuman   create a visible human-review obligation
raise      publish a typed event
```

Native local agent effects such as `start worker { ... }` and
`send director "..."` persist to the SQLite agent ledger and are claimed by the
local harness. Adapter-backed capability effects such as `plan.snapshot()` or
`plan.markDone(...)` may be
registered by adapters. They are not free-form workflow code.

`coerce` and adapter value operations are synchronous typed value calls, not
workflow control-flow authority. They produce values that the statechart may
branch on. Any resulting `start`, `send`, `askHuman`, `raise`, or adapter write
is still an explicit Whippletree effect with its own schema and capability checks.
For local agents, `start`/`send` are native ledger writes rather than adapter
dispatch.
The v1 `coerce` backend is BAML HTTP via `baml-cli serve`; TypeScript codegen is
not part of the selected execution path.

Each effect has:

- a schema
- required capabilities
- idempotency behavior
- failure categories
- model-generation semantics

The effect category and transaction behavior are defined in
[effects.md](effects.md).

### 7. Contract And Capability Resolver

The resolver combines workflow declarations with repo, workstream, thread, and
agent policies.

Responsibilities:

- load declared contracts
- resolve inherited capabilities
- reject capability escalation
- verify target agent/session sandboxes can satisfy requested effects
- expose capability facts to validators and model generators

This is the critical bridge to enterprise and un-tie-style environments.
The resolution algorithm and local/team/enterprise modes are defined in
[policy.md](policy.md).

### 8. Native Harness And Adapter Layer

The native harness connects local agent effects to provider processes. Adapters
connect explicitly external effects and observations to real systems.

Likely adapters:

```text
native command/Codex/Claude/Pi harness providers
un-tie thread/session adapter
human review adapter
filesystem state adapter, scoped to declared files
external process adapter, if explicitly enabled
legacy Whippletree adapter, if compatibility is needed
```

Harness providers and adapters should be narrow and capability-checked. They
are trusted runtime code, not workflow-authored code.

Harness provider authority is resolved through profiles, not raw provider names
in workflow logic. A workflow agent may request a semantic profile such as
`research` or `repo-writer`; the harness policy maps that profile to concrete
provider commands, filesystem/network posture, environment allowlists, timeout,
and enforcement mode. See [harness-profiles.md](harness-profiles.md).

### 8a. Coerce Execution Layer

The coerce execution layer evaluates BAML-backed synchronous value calls.

Supported backend modes:

```text
external BAML HTTP server supplied with --baml-url
managed local BAML HTTP server launched with baml-cli serve --from <baml_src>
```

The first implementation should support the external server mode before managed
process supervision. Managed mode can reuse process/logging lessons from the
legacy runtime, but the legacy runtime should not own `coerce` semantics.

The coerce layer is intentionally below the language boundary:

- it receives typed JSON inputs
- it returns typed JSON outputs
- Rust validates the output against WorkflowIR schemas
- it cannot enqueue events or dispatch workflow effects directly
- effectful host access must be represented as adapter capabilities, not hidden
  `coerce` behavior

### 9. Status And Diagnostics

Workflow status is a product feature, not an afterthought.

The status view should show:

```text
workflow state
workflow data snapshot or redacted summary
latest transition
pending events
ignored events
active effects
active invocations
latest BAML calls
blocked reason
current effect failures
current blockers
recent failures history
invariant/check status
```

`blocked reason` is reserved for explicit durable blockers. Implemented v0
mostly reports stuck work through current blockers, current effect failures,
policy blockers, queued events, active invocations, and the current coerce
failure rather than creating a hidden blocked state. Historical recent failures
and latest coerce failures remain visible for audit after retries or later
successful coerce calls.

Durable timers are a planned extension. They are not part of the implemented v0
DSL surface; `after` remains reserved in the grammar.

The strongest user-facing improvement over scripts is that stuck workflows must
be legible.

### 10. Formal Model Generator

The generator lowers validated IR into a formal transition-system artifact.

Initial targets may include hand-authored TLA+/Apalache or Veil models. The
long-term target should be generated verification artifacts from the same IR the
runtime executes.

The generator models:

- states
- events
- guards
- data fields relevant to invariants
- active invocation counters
- capability facts
- coerce outputs as nondeterministic schema-valid values
- effect failure categories

It does not model:

- prompts
- LLM internals
- arbitrary file contents
- host implementation details
- BAML HTTP server internals

### 11. Durable Event Queue

The durable event queue is the interpreter's input boundary. It owns event
ordering, statuses, retry/recovery behavior, dedupe policy, fanout semantics,
and retention. Timer admission is reserved for a later runtime slice.

The queue semantics are defined in [event-queue.md](event-queue.md).
The SQLite storage model is defined in [storage.md](storage.md).
