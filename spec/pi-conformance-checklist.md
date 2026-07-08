# Pi-conformance checklist — owned-harness behavior deltas

Status: extraction DONE 2026-07-07 (un-tie tracker Phase 3, v0.3). Source:
pi-mono @ `351efc828b6fc5250fa50d6b32b20b0f0cb22cb4` (packages/coding-agent +
packages/agent) vs. the whip owned harness (`harness_tools.rs`,
`harness_loop.rs`, `harness_model.rs`). System prompt, skills, project
instructions, and compaction are OWNED by the context-assembly tracker
(closed) and excluded here. Verdicts: **PORT** (build the delta), **KEEP**
(whip's behavior is deliberately different/better — documented divergence),
**N/A** (pi has nothing to conform to).

## 1. Tool ergonomics

| Area | pi | whip today | Verdict |
|---|---|---|---|
| read truncation | line-based: 2000 lines OR 50KB, head-truncate, continuation notices `[Showing lines A–B of TOTAL. Use offset=N to continue.]`, first-line-exceeds-limit fallback message | byte cap only (50KB middle-truncate + recall footer) | **PORT** line-window + continuation notices (keep the recall footer for the byte overflow) |
| read binary/size guard | image MIME detection (jpg/png/gif/webp/bmp) → image blocks; no other guard | none — `read_to_string`, non-UTF-8 errors raw | **PORT** a binary/size guard (clean error for non-UTF-8; images → §6) |
| edit robustness | arg tolerance (`edits` as JSON string, legacy top-level oldText/newText), BOM strip + LF-normalize/restore, overlap detection, fuzzy match, rich error strings | exact-match + unique check only | **PORT** arg tolerance + BOM/LF normalization + overlap detection. Fuzzy match: defer (whip's exact-match errors are informative; fuzz hides mistakes) |
| bash output | tail-truncate to 2000 lines/50KB, full output → temp file, no default timeout, streamed updates | middle-truncate + content-addressed **recall** tool, 30s default timeout | **KEEP** — recall is strictly better than temp files (durable, replayable); default timeout is safer. Documented divergence |
| bash stderr | merged with stdout | merged with stdout | conformant |
| grep | ripgrep regex, `context` lines, glob filter, 500-char line cap, 100-match limit, `.gitignore` | substring only, 100-match limit | **PORT** regex + `context` + per-line cap (via the `regex` crate; keep the walker — no rg dependency) |
| find / ls | fd-backed glob, 1000 / 500 limits, `.gitignore` | glob walker, same 1000 / 500 limits | conformant (gitignore awareness: defer) |
| fetch/browser | none in the harness (MCP-only) | none (web-fetch has its own design note) | **N/A** here |
| tracker todos | none built-in | list/add/update_todo | whip extension, keep |

## 2. Turn lifecycle

| Area | pi | whip | Verdict |
|---|---|---|---|
| step bound | none (runs until no tool calls) | `max_steps` hard bound → TimedOut | **KEEP** (bounded loops are the whip stance) |
| provider retry | session auto-retry on retryable errors (rate-limit/overloaded/server): 3 attempts, exp backoff 2s/4s/8s; context-overflow excluded (compaction's job) | none (only the overflow front-trim) | **PORT** bounded retry on retryable provider errors in the brokered loop |
| tool-calls-on-length | provider `stopReason=length` fails the calls with a re-issue message | not handled distinctly | **PORT** (small): treat truncated tool calls as informative errors |
| streaming | token streaming events | none | **KEEP** — no token streaming (settled 2026-07-04) |

## 3. Abort semantics

pi: one AbortController per run; model stream and every tool observe it;
partial assistant message with `stopReason: "aborted"` persists in the thread;
tools settle before release (fs ops finish, bash process tree killed).

whip: **no mid-turn cancellation on the owned loop at all** (DR-0035 B3
covered the delegated adapters only).

**PORT**: cooperative cancel in the brokered loop — check the durable
`effect_cancellation_requests` surface between model steps and before tool
dispatch; settle as `Cancelled` with the transcript checkpointed (the partial
conversation persists, mirroring pi). In-flight tool completes before the
check (whip tools are synchronous — the natural settle-before-release).

## 4. Thread continuation (the chat-shaped instance)

pi: append-only session tree (`leafId` head); every message persisted on
`message_end`; a new user prompt rebuilds context from the tree and continues;
model/thinking/tool changes are tree entries; branching = moving the leaf.

whip: transcripts persist per **effect** (crash-recovery resume only); every
`tell` is a fresh single turn.

**PORT (v1)**: thread continuation keyed by `(instance, agent)` — an agent
declares `thread continue` (Managed-only knob, default `fresh` preserves
current semantics); a new tell to a threaded agent seeds `resume_from` from
the agent's latest completed-turn transcript and appends the new user message.
The instance event log is the tree (append-only already); branching = v0.4
(needs workspace branches, per the tracker fork).

## 5. Error surfacing

| Area | pi | whip | Verdict |
|---|---|---|---|
| tool errors → model | thrown → `isError:true` tool result, verbatim text | `ToolStatus::Error` + text, same shape | conformant |
| exit codes | `Command exited with code N` + output | `command exited with status N` + combined output | conformant |
| provider error cap | errorMessage on the assistant message | 300-char excerpt → turn summary | conformant (different plumbing, same info) |
| `is_error` on OpenAI wire | n/a (pi-ai handles) | **dropped** — `function_call_output` carries only output | **PORT** (small): prefix `error: ` marker into the output text when is_error, so OpenAI models see failure |

## 6. Multimodal

pi: `ImageContent {type:"image", data(base64), mimeType}` blocks; user
messages carry `[text, ...images]`; the read tool returns image blocks
(resize to 2000×2000 / 4.5MB inline); tool results may carry images;
non-vision models get an omission note.

whip: **no image support anywhere** — `ChatMessage` is all-String; the
agent-turn input is a prompt string.

**PORT (v1)**: `ImageBlock {media_type, data_base64}`; the agent-turn effect
input accepts `images: [...]` alongside `prompt`; `ChatMessage::User` gains
image blocks; the Anthropic wire emits `image` source blocks (OpenAI:
`input_image`). The read-tool image path + resizing: defer (needs an image
codec dependency; the effect-surface half is the v0.3 commitment).

## Build order (deltas, per-piece)

1. Owned-loop cooperative cancel (§3) — the safety-relevant one.
2. Provider auto-retry + length-stop handling (§2).
3. read line-window/notices + binary guard; grep regex+context; edit
   robustness; OpenAI is_error marker (§1, §5).
4. Thread continuation `thread continue` (§4).
5. Multimodal input blocks (§6).

Workbench event projection is tracked separately (same tracker phase): the
existing evidence/event stream (`agent.turn.tool_requested`,
`agent.turn.brokered.tool_result`, dev_stream_v0 schema) is the base; the
projection stabilizes it for external UIs.
