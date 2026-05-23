# Statechart Workflow Architecture

Status: design proposal

Statechart workflows should be implemented as the new primary Armature product
surface for this track. The old Armature daemon/task implementation may be
reused opportunistically, but it should not define the conceptual model.

The new core is:

```text
native .armature source files
validated workflow IR
durable event queues
append-only transition/effect logs
trusted Rust interpreter
typed effects
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

## Candidate Architectures

### A. Minimal Interpreter First

Build the workflow system directly around a custom parser, normalized IR, static
validator, and durable interpreter.

```text
.armature source
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
- tight fit with Armature's existing runtime objects
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
.armature source
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

- use the native `.armature` statechart DSL as the initial authoring format
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

### 1. Armature Source Parser

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

The generator turns Armature `class`, `enum`, and `coerce` declarations into
normal generated BAML artifacts.

Responsibilities:

- generate BAML enum/class/function declarations
- validate that workflow calls reference declared coerce functions
- expose coerce input/output schemas to the workflow validator
- make coerce outputs available as typed values in the IR

Non-responsibilities:

- using BAML as a control-flow runtime
- allowing BAML to return arbitrary executable plans

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
start      begin asynchronous external work
coerce     call BAML to produce typed data from input
assign     update workflow-local durable data
askHuman   create a visible human-review obligation
raise      publish a typed event
sleep      create a durable timer
stop       enter a terminal state
```

Adapter-backed capability effects such as `plan.snapshot()` or
`plan.markDone(...)` may be
registered by adapters. They are not free-form workflow code.

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

### 8. Adapter Layer

Adapters connect effects and observations to real systems.

Likely adapters:

```text
un-tie thread/session adapter
BAML adapter
human review adapter
filesystem state adapter, scoped to declared files
external process adapter, if explicitly enabled
legacy Armature adapter, if compatibility is needed
```

Adapters should be narrow and capability-checked. They are trusted runtime code,
not workflow-authored code.

### 9. Status And Diagnostics

Workflow status is a product feature, not an afterthought.

The status view should show:

```text
workflow state
latest transition
pending events
ignored events
active effects
active invocations
next timers
latest BAML calls
blocked reason
recent failures
invariant/check status
```

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

### 11. Durable Event Queue

The durable event queue is the interpreter's input boundary. It owns event
ordering, statuses, retry/recovery behavior, dedupe policy, fanout semantics,
timer admission, and retention.

The queue semantics are defined in [event-queue.md](event-queue.md).
The SQLite storage model is defined in [storage.md](storage.md).
