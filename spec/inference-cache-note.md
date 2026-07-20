# Inference-cache research spike — findings note

**Status: research finding, 2026-07-19.** v0.2 milestone, Cluster C
(`spec/v0.2-milestone-tracker.md`). Verdict: whip has solid *foundational*
prompt-cache support already; the wins are two specific, bounded gaps —
**(G2) measure cache economics** and **(G1) cache the growing conversation** —
not a from-scratch build.

## Objective (narrow)

Maximize the **provider-side prompt-cache hit rate** across a workflow's model
calls — the lever that cuts both **cost** (Anthropic bills cache *reads* at ~0.1×
input, cache *writes* at ~1.25×; OpenAI ~0.5× cached; local KV reuse is free) and
**latency**. Every provider rewards the same thing: a **stable, byte-identical
prompt prefix** across turns, plus — for Anthropic — an explicit `cache_control`
breakpoint at the end of each stable region. Not the objective: raw token count.

## Current state — whip is well-founded (verified)

- **Deterministic context assembler** → a byte-stable `[tools, system]` prefix
  (`context_assembly.rs:16,129` — "same bundle set yields byte-identical output
  regardless of insertion order"). This is the precondition for any caching.
- **Anthropic:** one `cache_control: {type: ephemeral}` breakpoint at the end of
  the `system` block (`harness_model.rs:495`). Because Anthropic's cache order is
  tools → system → messages, this one breakpoint caches the whole
  **tools + system** prefix. Correct as far as it goes.
- **OpenAI / compat:** sends `prompt_cache_key` = the resume-stable per-effect run
  id (`harness_model.rs:68,369`); OpenAI's automatic prefix caching then applies.
- **Raw `usage` is stored verbatim** in run metadata (`harness_model.rs:768` etc.),
  so provider cache fields survive on disk even though nothing reads them yet.

## Gaps (prioritized wins)

### G2 — cache economics are invisible to the pricing/observability layer  *(do first: measure + correctness)*
`TurnUsage::from_usage_json` (`improve.rs:104`) parses only `input_tokens` /
`output_tokens` / `total_tokens`; it **discards** `cache_read_input_tokens` /
`cache_creation_input_tokens` (Anthropic) and `prompt_tokens_details.cached_tokens`
(OpenAI). `PriceTable::cost_micros` then prices *all* input at the full rate.
Consequences: (a) whip cannot report its own cache-hit rate — you can't optimize
what you don't measure; (b) spend is **wrong whenever caching works** (cache reads
should price ~0.1× on Anthropic) — this directly undercuts the `--spend-cap`
accuracy just hardened (see [[project-v02-milestone]] unpriced-model work). Fix:
capture the cache token fields into `TurnUsage`, add a `std.cache_hit` builtin
gauge (cache_read / total_input), and apply cache-read/creation multipliers in
`cost_micros`. Small, self-contained, fixes a real correctness gap.

### G1 — no moving breakpoint on the conversation prefix  *(the actual optimization)*
whip caches only the static `[tools, system]` prefix; the **conversation**
(messages) appends *after* the sole breakpoint and is re-processed **uncached
every turn**. In the owned harness's multi-turn tool-use loop (tool call ↔ tool
result, often large results), this is the dominant re-processed cost. Anthropic
allows **up to 4 breakpoints**; whip uses 1. Fix: on each turn, also mark
`cache_control: ephemeral` on the **last content block of the previous turn**, so
the growing conversation prefix caches incrementally (standard Anthropic pattern).
Net-positive for any loop that turns over within the 5-min TTL (the 1.25× write on
the delta is repaid by 0.1× reads of the whole prior prefix). **Anthropic-specific**
— OpenAI/compat already cache the growing prefix automatically, so this needs no
OpenAI change beyond keeping the prefix stable.

### G3 — no explicit byte-stability guard on the cached prefix  *(cheap guard)*
The assembler is deterministic by construction (sorted bundles) but there is no
test asserting the cached prefix is byte-identical across two assemblies with
volatile inputs (timestamps, per-instance ids, skill/tool ordering). One
regression test would lock the precondition that all caching depends on. Low
effort, high insurance.

## Recommendation & sequencing

1. **G2 first** — capture cache tokens + `std.cache_hit` gauge + cache-aware
   pricing. Foundational (measure before optimize) *and* fixes spend correctness.
2. **G1 next** — the moving-breakpoint optimization, now measurable via G2's gauge.
3. **G3** — a byte-stability guard test alongside either.

All three are small and self-contained (comparable to the openai-generic pass),
not a from-scratch subsystem. Recommend building G2+G1; G1 is the cost win, G2 is
the correctness+observability that makes it (and the spend cap) honest.

## Caveats

- No live cache-metric measurement was run (Anthropic needs creds; Ollama/tinyllama
  don't expose cache economics). Findings are a code audit against documented
  provider caching mechanics (Anthropic explicit breakpoints; OpenAI automatic
  prefix; vLLM/KV prefix reuse), which is sufficient to locate the gaps; G2 is
  itself the instrumentation that would make live measurement possible.
