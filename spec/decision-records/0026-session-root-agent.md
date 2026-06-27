# DR-0026: The session root agent — master loop, policy envelope, and stream protocol

Status: accepted 2026-06-26 (design). Builds on the owned brokered harness
([DR-0024](0024-owned-brokered-agent-harness.md)) and workflows-as-tools
([DR-0025](0025-workflows-as-agent-tools.md)). It defines a **new top-level
entry point** — an interactive *session root* agent that authors, runs, and
observes whips on the operator's behalf — and locks how that root is bounded
(a signed **policy envelope** it cannot self-widen) and how external clients
observe it (a cursor-tailed **session-event stream** over the existing durable
log). It governs **only** the root tier; the broader information-flow /
non-interference research direction is explicitly out of scope and noted as a
separate thread at the end.

## Problem

Today the entry point is inverted from where the leverage is. A human authors a
`.whip`, runs `whip dev`, and the program orchestrates agents. The owned harness
(DR-0024) made *in-workflow* agents first-class — brokered, bounded leaves held
to I1–I3. But the human is still the only thing that writes and runs whips.

The opportunity the owned harness opens: run the WhippleScript harness *as the
top-level interface to the model*. The shape:

```text
operator <-> session root agent --writes / runs / observes--> whips
                   |                                            |
            (the one loop allowed to                    each whip --> sub-agents
             spawn arbitrary whips)                      (owned harness, DR-0024)
```

Two things make this more than "another agent." First, the root is the **one
loop with authority to generate and run arbitrary whips** — that authority is
precisely what every *in-workflow* agent is denied (DR-0024 I3 forbids a leaf
from directing orchestration or escalating its authority). Concentrating
arbitrary-spawn at a single, governed root is what keeps the rest of the tree
bounded. Second, a root that authors and runs code is the **sharpest capability
one can hand a model**, so the question "what bounds the root, and who may widen
that bound?" is the load-bearing design problem, not an afterthought.

This record answers three things: where the root lives, what bounds it, and how
the outside world watches it.

## Decision

### D1 — The session root is a special root whip, not host code

The session agent runs **inside** the whip runtime as a long-lived root
instance, reusing the DR-0024 brokered loop. The host process is thin: it boots
the root, bridges operator I/O to the root's inbox, and tails the event stream
for subscribers. The intelligence and the durable record live in a whip.

Why inside, not a separate host-level agent loop:

```text
session root = a root whip            session root = host/daemon code
------------------------------        ------------------------------------
conversation is durable, resumable,   top layer is ephemeral; its reasoning
inspectable — crash and resume        lives outside the store unless re-plumbed
the session for free
root's turns + tool calls are          a second event source to reconcile with
evidence in the SAME stream apps       the whip stream
subscribe to (one stream)
ONE harness (DR-0024 reused)           TWO agent loops that drift — the exact
                                       thing the owned-harness consolidation
                                       was meant to end
```

The decisive factor is **single-harness discipline**: we just consolidated onto
the owned brokered harness; a parallel top-level loop in Rust would be a second
harness that diverges. The cost accepted: the root is fundamentally
non-convergent (interactive, unbounded, human-driven). That is correct for a
*root* and is why the root can never itself be a `@tool` (DR-0025) — it is the
opposite of convergent.

### D2 — Relationship to DR-0024: I1 preserved, I2 N/A, I3 relocated

The root is an owned-harness loop, so it inherits the invariants — but a *root*
is not a *leaf*, and the difference is exactly the spawn authority:

```text
DR-0024 invariant   at the session root
-----------------   ------------------------------------------------------------
I1 brokered exec    PRESERVED and load-bearing. The root model never touches FS /
                    shell / network directly; whip brokers every tool. This is the
                    single enforcement point for the policy envelope (D3).
I2 turn is a leaf   DOES NOT APPLY. The root is a root, not a leaf. It is the one
                    place orchestration-directing tools (author whip, spawn whip,
                    observe stream) are granted.
I3 no self-         RELOCATED, not dropped. The root MAY direct orchestration —
   escalation       that is its job — but it MAY NOT widen its own authority. The
                    boundary moves from a static lease/counter to the policy
                    envelope (D3): only a human-signed policy change can widen it.
                    Same spirit (no self-escalation), different boundary.
```

The continuity worth stating outright: **I3's "the loop cannot widen its own
authority" holds at the root too** — the root simply has more *base* authority
(arbitrary spawn) and a different escalation boundary (signed policy + human
approval) than a leaf (static envelope). No agent in the system, root or leaf,
can escalate itself.

### D3 — Bounding the root: a signed policy envelope the root cannot self-widen (governance Option A)

Governance is **Option A: no second agent.** Three considered options:

```text
(A) human + signed policy artifact + escalation-over-`human.ask`   <-- CHOSEN
(B) (A) + an ephemeral, tool-less LLM chat that drafts policy diffs for signature
(C) a standing second "governance" agent loop
```

(C) is rejected for v1: a standing policy-writing agent moves the crown jewels
rather than eliminating them — jailbreaking *it* becomes the escalation path —
and it only holds under a hard untrusted-input-isolation invariant we are not
ready to commit to. (B) is a trivial later add (it produces only a diff a human
signs) and is recorded as a deferred hook. (A) delivers the full
privilege-separation *property* with no new standing capability holder.

The mechanism, all reuse-of-existing-grain (signed package locks, attestation,
the capability / grant system, `exec` allow-lists):

```text
1. Policy envelope = a signed, versioned artifact. It declares the root
   session's authority: permitted exec commands, writable file regions, egress
   destinations + providers, budget caps (counter), and the arbitrary-whip-spawn
   toggle. It is the root's lease/counter/file-store/capability envelope, made
   explicit and signed.

2. Enforcement lives in the KERNEL, never in an agent. Every brokered tool the
   root requests (I1) is checked against the envelope at execution. An LLM cannot
   be trusted to sandbox itself; the deterministic substrate does it.

3. The root cannot modify the envelope. Modification is a human-authorized
   action that produces a NEW signed version.

4. Escalation is reactive and human-in-the-loop. The root hits a denied
   capability -> the request surfaces as a `human.ask` escalation to the operator
   on the session stream -> the operator approves -> the approval is appended as a
   signed grant (new envelope version) -> the kernel now permits it. No agent
   writes policy; the human signs, the kernel enforces.
```

The property this buys, by construction: **the root cannot broaden its own
envelope.** A confused or injected root can only operate within the signed
bound; escalation requires a human signature. This is privilege separation
without a second agent.

Precise wording on "no egress" (it bites): for *any* agent loop, the model call
itself is network egress to the provider endpoint. The envelope therefore
governs **tool/exec/fetch egress to arbitrary destinations**, never the provider
round-trip — an envelope that forbade the provider call would describe something
that cannot think.

### D4 — Arbitrary children are spawn-and-observe, distinct from `workflow.invoke`

The root's "run a whip" is **not** DR-0025 `workflow.invoke`, and the two must
stay distinct mechanisms (no premature unification):

```text
workflow.invoke (DR-0025)            spawn-and-observe (this record)
------------------------------       ------------------------------------------
pre-declared @tool, attested,        an arbitrary whip the root just authored
convergence-checked, typed I/O       — not pre-declared, may itself be interactive
typed tool result crosses up         observed via the event stream, not a return
held to the convergence invariant    NOT convergence-checked (D5)
```

A spawned child compiles (`whip check` returns diagnostics to the root as a tool
result — the lint/LSP/diagnostic surface *is* this feedback loop) and runs as a
**child instance sharing the durable store**, so its events flow into the same
stream the root watches. The root observes; it does not get a typed return.

### D5 — No convergence guarantee at the root, and that is not a hole

Convergence (DR-0025) governs *only* the `workflow.invoke` tool path; arbitrary
spawned children do not use that path, so declining the guarantee there punches
no hole in anything `workflow.invoke` offered. The property that *does* hold:
each individual whip remains **bounded by design** (it terminates, is
capacity-bounded). The root composes individually-bounded whips arbitrarily; the
count is operator-driven, finite, and small. **Root = unbounded orchestration of
individually-bounded whips** — internally consistent, and exactly the
root-vs-leaf authority split of D2.

### D6 — Observation: a versioned session-event stream over the durable log (protocol, not TUI)

Positioning: expose a **session + stream protocol**; do **not** build a polished
TUI (a thin stdio reference client only, to prove the protocol). The mechanism
leans entirely on what exists:

```text
1. Source of truth = the existing durable fact / evidence store. It is already an
   append-only event log. We do NOT build a separate message bus — that would
   duplicate the store's durability.

2. Subscription = a cursor-based tail over the log (precedent: the OTLP export
   emit-once cursor). Because the log is durable, subscriptions are REPLAYABLE
   and RESUMABLE: a client tracks a cursor, disconnects, reconnects, replays from
   where it left off — at-least-once, for free. No ephemeral bus gives this.

3. Public contract = a versioned session-event schema (`session_event_v0`, to be
   specified, sibling to `package_contract_v0`). Raw facts are internal and too
   low-level; subscribers see a CURATED projection: turn started, tool called,
   child spawned, child completed/failed, ask pending, output produced,
   escalation requested/granted.

4. Transport = thin adapters over (3). TUI, websocket, stdio-JSONL, RPC are all
   projections of the same cursor feed; v0 ships only the stdio-JSONL reference
   client. The durable log IS the "native pub/sub format."

5. Input authority. Multi-client READ is N subscribers on the log. Multi-client
   WRITE (answering the root, sending new instructions) goes through the inbox
   (`human.ask` / inbound message), which already exists; escalation approvals
   (D3) are one kind of inbox write.
```

## What this record does not decide

Explicitly open, sequenced as later steps (modeled-first per standing
discipline):

```text
- session_event_v0 schema. The exact curated event set, topic/filter model, and
  delivery/cursor semantics. The real design work behind D6; its own spec doc +
  report-schema, like package_contract_v0.

- child execution mechanism: in-process (shared store, one runtime — cleaner
  unified stream) vs `whip dev` subprocess (isolation, blast-radius — needs
  cross-process event bridging). Leaning in-process + a session-level budget
  governor, but the one genuine technical fork, left for its own step.

- policy-envelope scope: per-session-root vs a machine-level baseline that
  per-session envelopes may only NARROW (never widen). v1 assumes one envelope
  per session root; the machine-baseline ceiling is a recorded hook.

- formal model + lifecycle. TLA+ for session boot / escalation / crash-resume of
  the root; Maude for any rule-surface or envelope-enforcement check (e.g. the
  bite: a root tool call outside the signed envelope MUST be rejected; a root
  attempt to mutate its own envelope MUST be rejected).
```

## Deferred capabilities (recorded hooks, not v1)

```text
governance draft-chat (Option B)  An ephemeral, tool-less, no-arbitrary-egress
                                  LLM chat whose ONLY output is a proposed policy
                                  diff the operator signs. Adds NL policy authoring
                                  with no standing capability holder. Trivial add
                                  once D3's signed-artifact + signature flow exists.

machine-baseline policy ceiling   A machine-wide envelope that per-session
                                  envelopes may only narrow; escalation within a
                                  session cannot exceed the baseline without a
                                  higher-privilege baseline change.

richer transports                 websocket / RPC adapters over the same cursor
                                  feed, beyond the v1 stdio-JSONL reference client.
```

## Relationship to the information-flow research thread (out of scope here)

A separate, larger research direction — **provable mitigation of prompt-injection
and data-exfiltration via a typed information-flow lattice** (Biba integrity for
anti-injection, Bell–LaPadula confidentiality for anti-exfiltration; separated
agent contexts as label regions; typed declassify/endorse boundaries proved at
compile time) — is being explored **model-first in Maude before any DR**. This
record deliberately does **not** depend on it.

How they compose when that thread matures: the policy envelope (D3) is the
**coarse outer boundary** on the root's authority; the label system would later
add the **fine inner structure** that proves *which data may flow to which sink*
inside and between whips. Coarse capability gating now; provable flow control as
a future, separately-modeled layer.

## Consequences

What downstream work must preserve, in one place:

- The session root is a whip (D1); one harness, no parallel top-level loop.
- I1 holds at the root and is the policy-enforcement point; I2 is N/A; I3's
  no-self-escalation holds, relocated to the signed envelope (D2).
- Enforcement is in the kernel, never an agent. The root cannot widen its own
  envelope; only a human-signed policy change can (D3, Option A — no second
  agent).
- Arbitrary spawned children are spawn-and-observe over the shared store, kept
  distinct from typed `workflow.invoke` (D4); no convergence guarantee at the
  root, with each child still individually bounded (D5).
- Observation is a cursor-tailed, versioned, curated session-event projection
  over the existing durable log — protocol not TUI, no separate message bus (D6).
- The information-flow lattice is a separate research thread; the envelope is the
  coarse boundary it will later refine, not replace.
