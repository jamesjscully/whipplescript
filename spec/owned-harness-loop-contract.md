# Owned harness — slice 1 loop contract

Status: design, 2026-06-24. The concrete, implementation-facing contract for
**slice 1** of [DR-0024](decision-records/0024-owned-brokered-agent-harness.md):
the brokered loop + file tools. This is DR-0024 "step 3" (event-stream +
projection contract) made concrete for slice 1's scope. Deferred to later slices:
bash/sandbox (3), budget/lease enforcement (2), tracker tools (4), full compaction
(5), resume-from-projection (6).

Grounded in the current code (file:line refs are to the 2026-06-24 tree).

## 1. Harness mode and selection

A new harness mode `owned` (provider kind `owned`, surface `owned`). It joins the
existing dispatch fork in `run_agent_effect`
(`crates/whipplescript-cli/src/main.rs:20847`) as a new
`provider_selection.kind == "owned"` arm, alongside `native-fixture` / `codex` /
`claude` / `pi` / `command`. Slice 1 selects it explicitly (e.g.
`--provider owned`, or an agent whose harness kind is `owned`); making it the
*default* is sequenced with the codex/claude-optional migration, not slice 1.

The owned arm constructs the brokered runner inputs (a model-client + a
tool-executor over a workspace) and calls a new kernel runner
`RuntimeKernel::run_brokered_agent_turn_with_metadata`, modeled on
`run_native_agent_turn_with_metadata` (`crates/whipplescript-kernel/src/lib.rs:835`).

No new public trait in slice 1 (one implementation; YAGNI). The runner takes the
generalized model-client (behind the existing `CoerceTransport` seam for test
fakes) and a concrete `ToolExecutor`.

## 2. The brokered loop

```text
run_brokered_agent_turn(execution, model_client, tools, workspace):
  start_run(...)                       # reuse: claims effect, opens run + lease
  record_skill_evidence(...)           # reuse
  observe(started)                     # lifecycle: derives the agent.turn fact gate

  context = assemble_initial_context(execution.input_json, tools.specs)
  loop up to MAX_STEPS:                # slice-1 hard bound (budget enforcement = slice 2)
    reply = model_client.next(context) # one model call (reuse coerce transport/creds)
    observe(model_request, redacted)   # EVIDENCE only
    if reply.is_final:                 # model emitted a terminal message, no tool calls
       break with Completed(reply.text, reply.structured?)
    for call in reply.tool_calls:
       observe(tool_requested{name, redacted args})        # EVIDENCE only
       result = tools.execute(call, workspace)             # KERNEL executes the tool
       observe(tool_result{name, status, redacted shape})  # EVIDENCE only
       context.append(tool_call, tool_result)
  # loop exhausted -> TimedOut (synthetic), mirrors native player lib.rs:939

  complete_brokered_agent_turn(terminal)   # reuse complete_native_agent_turn shape:
                                           # ONE terminal lifecycle event + ONE fact
```

The structural difference from the native player: where the native player
passively calls `adapter.next_event(run_id)` and records it, the brokered runner
**actively executes each requested tool** between model calls. Everything else —
run/lease lifecycle, evidence recording, redaction, terminal — is reused.

`MAX_STEPS` is a slice-1 safety bound only; the *governing* budget (`counter`) and
workspace (`lease`) enforcement is slice 2.

## 3. In-turn stream events (evidence-grade; no facts)

Reuse the existing `NativeAgentTurnObservation` machinery
(`crates/whipplescript-kernel/src/native_lifecycle.rs`) and its fact-gate
`derives_rule_matchable_fact()` (`native_lifecycle.rs:60`). The leaf invariant
(I2) is enforced by that gate: only terminal lifecycle kinds derive a fact.

```text
observation kind            event_type                         derives fact?
started                     agent.turn.started                 YES (one, at start)
model_request   (new)       agent.turn.brokered.model_request  no  (evidence)
tool_requested  (existing)  agent.turn.tool_requested          no  (evidence)
tool_result     (new)       agent.turn.brokered.tool_result    no  (evidence)
completed/failed/timed_out  agent.turn.<status>                YES (one, terminal)
```

- `model_request` and `tool_result` are added to the observation-kind enum as
  **evidence-only** kinds (return `false` from `derives_rule_matchable_fact`).
  `tool_requested` already exists as evidence-only and is reused for tool calls.
- Payloads cross the redaction boundary as **shape only** via `json_shape`
  (`native_lifecycle.rs:291`); any control-plane error string uses
  `redacted_provider_error_detail` (`provider.rs:584`, 300-char cap + scrub).
- `events.event_type` is untyped TEXT (`migrations/0001_runtime_store.sql`), so no
  schema migration; in-turn dedup uses the per-(instance,idempotency_key) index.

The terminal re-engages the keystone exactly as today: `complete_run` /
`fail_run_with_diagnostic` / `timeout_run_with_diagnostic` under the
`terminal_completion_idempotency_key`, producing exactly one `agent.turn.<status>`
fact (layer 3). Interior events never reach `derive_fact`.

## 4. File tool set (slice 1)

Pi-style shapes (decided). Each tool is a facade executed by the kernel through
the **file-store policy boundary** — reuse the existing file effect handlers
(`run_file_effect` / `run_file_write_effect`, `main.rs`) and the `file store`
read/write glob policy rather than touching the filesystem raw.

```text
tool   inputs                              executes as
read   {path, offset?, limit?}            file-store read (bounded slice + ref)
write  {path, content}                    file-store write (full overwrite)
edit   {path, edits:[{oldText,newText}]}  file-store read+write (exact replace)
grep   {pattern, path?, glob?, limit?}    file-store read (search)
find   {pattern, path?, limit?}           file-store read (glob)
ls     {path?, limit?}                    file-store read (list)
```

- Slice-1 access scope: a single workspace root for the turn (a concrete
  `cwd`/root). The construct-level question (reuse the `file store` declaration vs.
  a turn-scoped workspace grant) is a slice-2/governance decision; slice 1 wires
  a direct root and applies read/write separation so the structure is right.
- `edit` is exact string-replace; `oldText` must match a unique region of the
  current file; failure returns a tool-result error the model can retry
  (anti-idempotence is intended — DR-0024 boundary corollary).
- Tool output is truncated at record time to a bounded slice; the full output is
  recoverable by reference (the event id). Full compaction is slice 5; slice 1
  does naive bounded truncation only.

## 5. Model-client generalization

Generalize the coerce client (`coerce_native.rs` pure core + `coerce_runtime.rs`
transport) from single-shot structured output to a tool-use loop:

- **Input**: a messages list (not one prompt) + a tools list (the file-tool
  specs as JSON Schemas). Drop the forced `tool_choice` that coerce pins.
- **Output parse**: read assistant `tool_use` (Anthropic) / `function_call`
  (OpenAI Responses) blocks across turns; a turn with no tool calls and a final
  message ends the loop. The codex SSE assembler
  (`assemble_responses_sse`, `coerce_runtime.rs:411`) must also harvest the
  `output[]` function-call items, not just `output_text.delta`.
- **Reuse unchanged**: credential resolution + precedence
  (`resolve_credential_with_source`, env -> stored -> Codex OAuth),
  model resolution (`WHIPPLESCRIPT_*` -> `~/.codex/config.toml`), and
  `UreqCoerceTransport`. The brokered harness gets its own env namespace
  (`WHIPPLESCRIPT_HARNESS_*`) so it is configured independently of coerce.
- Structured final result (optional in slice 1): if the agent declares an output
  type, the last turn can still use the coerce structured-output path; otherwise
  the terminal summary is the model's final text.

## 6. Invariants to model (slice 1)

For the Maude model + test (and TLA where lifecycle-relevant; much is already
covered by `ControlPlaneLifecycle.tla`):

1. **Leaf-ness (I2):** no interior observation
   (`model_request`/`tool_requested`/`tool_result`) ever derives a rule-matchable
   fact. Bite: flip one interior kind's `derives_rule_matchable_fact` to true and
   the search for an interior-derived fact must become satisfiable.
2. **Exactly one terminal (layer 3):** a brokered turn reaches exactly one
   terminal lifecycle fact; bite via the `=>* … RESIDUAL:Cfg` two-terminal search
   expecting `No solution.` (cf. `tests/native-provider-lifecycle.maude:38`).
3. **Brokering (I1):** in the model, a tool-result observation can only follow a
   tool-requested observation mediated by a broker step — no tool effect appears
   without the kernel broker transition. Bite: a rule that emits a tool-result
   without the broker guard must be reachable only when the guard is dropped.

## 7. Out of scope for slice 1 (later slices)

- Budget (`counter`) and workspace (`lease`) *enforcement* with bite — slice 2.
- `bash` + sandbox + command classification — slice 3.
- Tracker tools (`list/add/update_todo`) — slice 4.
- Full compaction projection (truncate-by-ref, two-tier eviction, summarization,
  anti-thrashing) — slice 5.
- Resume-from-projection crash recovery — slice 6.
- Persistent PTY sessions, atomic-turn isolation — deferred (DR-0024).
