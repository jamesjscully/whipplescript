# The web fetch tool — Design Note (in-house, GET-only)

**Status: ACCEPTED (Jack, 2026-07-07) — BUILT 2026-07-10 (v0.4, native
owned harness: `web_tools.rs`, GET-only behind the central FetchGuard
with resolve-then-check + pinned connection + redirect re-entry, htmd
converter; §8 residuals that remain open: grant-naming settled as
`web { fetch search }` by the `command { run }` precedent, robots
posture, boilerplate-trim, PDF extraction, DO boxes).** Opened by Jack 2026-07-07 as the
fetch half of the network-tool policy discussion (search half:
`web-search-tool-design-note.md`, accepted the same day). Direction
settled: **built in-house** — the fetchkit dependency was dropped
2026-07-07; what its spike validated survives as requirements knowledge,
not as a dependency. This note designs the tool; build boxes + residuals
in §8.

## 1. Why a dedicated tool and not curl through the governed door

There is no free curl anywhere in whip's world — the in-isolate bash
tier has no network, the DO has no subprocess, native exec is behind the
allow-list — so any agent-visible HTTP is a facade over governed
machinery regardless. The dedicated tool is what shape that facade gets,
and the shape does real work (settled rationale, Jack 2026-07-07):

- **Capability granularity.** curl is *send*: arbitrary method, headers,
  body — a full exfiltration channel that no argv inspection can tame
  (the classifier-rollback lesson). `web_fetch` is structurally
  **GET-only with no request body**: the only egress is the URL string,
  which is exactly the narrow flow the IFC design already checks. This
  invariant is load-bearing; there is no v-next that adds POST to this
  tool — request-shaped HTTP is a different door with a different grant.
- **Central policy.** SSRF resolve-then-check, redirect confinement,
  private-range/metadata-IP denial, port limits — enforceable once,
  beneath the tool; unenforceable inside curl semantics.
- **Context economy.** HTML→markdown with boilerplate stripping is a
  10–50× token reduction with more signal; binary detection returns
  metadata instead of flooding the context; truncation is honest.
- **Evidence.** Tool calls are recorded stream events in the turn
  (DR-0024 loop contract); a replayed turn re-reads the record, never
  the network. curl inside an escalated exec would be an unrecorded
  side channel.
- **Familiarity.** Models are trained on WebFetch-shaped tools.

## 2. Tool surface

```jsonc
// web_fetch — GET-only, response-shaped.
input:  { "url": string,
          "max_bytes"?: number }          // policy-capped; default from policy
output: { "url": string,                  // final URL after confined redirects
          "status": number,
          "content_type"?: string,
          "content": string,              // markdown (html), text, or metadata line (binary)
          "truncated": boolean }
```

- v1 returns **raw markdown/text only** — no `prompt`-style model-side
  extraction (that is a summarizer/compaction concern; fork recorded in
  §8, not v1).
- Non-HTML text (`text/plain`, `text/markdown`, json, code) passes
  through unconverted. Binary content returns a metadata line
  (type/size/filename), never bytes.
- Errors are typed tool failures (blocked-by-policy, dns, timeout,
  too-large, http-status), distinguishable by the model.

## 3. Architecture — three small parts, all in-house seams

1. **`FetchGuard`** (policy): URL normalization; scheme allowlist
   (http/https); port policy (80/443 default); domain allow/block lists
   derived from the same workspace declarations as the search tool and
   sidecar egress (one discipline, three enforcement points);
   **unconditional** private/loopback/link-local/metadata denial —
   resolve-then-check with the connection pinned to the checked IP;
   each redirect hop re-enters the guard (hop limit; cross-host
   redirects re-checked, default-permitted within policy).
2. **`FetchTransport`**: native = `ureq` (the existing HTTP posture:
   threads + sync ureq; resolver override pins the checked IP; first-
   byte + total timeouts; streaming body read up to `max_bytes` with
   truncation, decompression-bomb capped). DO = the tool raises
   `NeedsHttp` on the DR-0033 machine and the TS shell fetches —
   the IP-level guard re-lands in the shell (the check must run where
   DNS happens); the URL-level guard runs wasm-side before the effect
   is raised.
3. **Converter**: **htmd** (Apache-2.0, html5ever-only dependency,
   passes the turndown.js test suite; maintained). Verified 2026-07-07:
   compiles clean to wasm32-unknown-unknown, 635 KB raw / 220 KB gz —
   fine against the size budget (bashkit composite left ~9 MB headroom).
   Post-conversion boilerplate trim (nav/footer link-density heuristic)
   is a small in-house pass, later refinement. Fallback option if the
   dependency ever sours: a hand-rolled streaming converter is
   ~1.7 k lines (fetchkit's, measured); not worth it while htmd holds.

"In-house" means the tool, guard, transport wiring, and policy are ours;
a pure-function converter dependency with one vetted parser dep is
consistent with that (it holds no authority and touches no I/O).

## 4. IFC and grants

- **URL = egress**, flow-checked like `send` (a URL string can encode
  anything); **response = low-integrity ingress** (taint-labeled;
  markdown conversion is context economy, *not* injection
  sanitization — the label is the mitigation).
- Grant surface: proposed **one `web` resource with verbs** —
  `with access to web { fetch }` / `{ search }` / both — so the
  search+fetch pair reads as one door with two narrow verbs (naming
  open, §8). Subagents get it only by delegation, per the workflow
  authority model. Owned-harness only.
- Fetch of a URL that arrived as low-integrity ingress (e.g. out of a
  search result or a fetched page) is expected and fine — the guard and
  the egress flow-check still apply; integrity of the *URL source* does
  not widen what the URL may reach.

## 5. Recording, replay, budgets

Per the DR-0024 loop contract: tool calls are **stream events
(evidence), never durable effects** — `web_fetch` results are recorded
in the turn stream and replay re-reads them; the network is touched
only on live execution. Budget: fetches count against the turn's
`counter` like other gated tools; per-fetch byte caps bound context
spend. No response cache in v1 (fetches are non-hermetic ingress);
conditional-GET revalidation (`ETag`/`Last-Modified`) is a later
economy layer if spend data argues for it.

## 6. Defaults (progressive rigor)

Zero-config posture: tool exposed when granted; guard active with
private-net denial (non-negotiable), no domain lists (open web),
2 MB default `max_bytes` (policy-overridable), 5 s first-byte / 30 s
total timeouts, 5-hop redirect limit, safeschemes http/https. Domain
allow/block lists engage as workspace declarations accrue — same
declarations the search tool and sidecar egress read.

## 7. What this is not

- Not a request tool: no POST/PUT, no custom headers, no cookies, no
  auth — all send-shaped, all a different door (an authenticated-API
  effect discussion that has not happened).
- Not a browser: no JS execution, no rendering; JS-only pages return
  what the HTML says (honest degradation). A rendering tier would be a
  Class-B/container concern if ever wanted.
- Not a crawler: one URL per call; no recursive fetch.

## 8. Open items

1. **Grant naming**: `web { fetch, search }` resource/verb shape vs two
   flat capabilities — settle with the capability-registry conventions.
2. **`prompt`/extraction parameter** (model-side digest of the fetched
   page): deferred — interacts with the summarizer/compaction machinery
   (context-assembly Phase 4), not fetch.
3. **robots.txt / crawl-etiquette posture**: single agent-initiated
   GETs are conventionally exempt from robots (it is not crawling), but
   decide and document; rate-limit per-host courtesy delay if usage
   data shows hammering.
4. **Boilerplate-trim pass** quality bar (link-density heuristic) and a
   conversion-quality corpus (shared with the search-note calibration).
5. **PDF/text extraction** for common binary types: later, likely a
   Class-A sidecar job (real tooling), returned through the same tool
   shape.
6. **In-isolate `curl` builtin** (bash note §9.6): if ever built, it is
   sugar over this exact guard+transport+recording machinery — never a
   second HTTP path.
7. DO build boxes: `NeedsHttp` executor wiring; TS-shell guard
   (IP check + redirect re-entry); wasm converter in the composite
   (size re-measure).

## 9. Settled vs. open

**Settled (Jack, 2026-07-07):** a dedicated fetch tool is justified over
curl-through-the-door (§1 rationale); **in-house build** (fetchkit
dropped; converter dependency = htmd per §3); GET-only invariant;
response-shaped surface (§2); IFC shape imported (URL egress /
low-integrity ingress).

**Open:** §8; ADR + build sequencing (owned-harness tool build home,
alongside the search tool in the v0.3 slice).

## 10. Relationships

- **`web-search-tool-design-note.md`** — the sibling half; search
  returns URLs, this reads them; shared domain-policy declarations and
  quality corpus; its §8.1 gap closes into this note.
- **`owned-harness-tool-surface.md`** — canonical tool-surface home;
  the network-tool policy discussion closed with acceptance of both
  halves (2026-07-07).
- **DR-0024 / `owned-harness-loop-contract.md`** — tool calls as stream
  events; brokered execution.
- **DR-0033 / `compute-plane-design-note.md`** — `NeedsHttp` on the DO;
  egress-allowlist discipline shared with sidecar containers.
- **`information-flow-surface.md` / `-governance.md`** — egress
  flow-check on URLs; ingress labeling.
- **`in-isolate-bash-design-note.md`** — future `curl` builtin rides
  this machinery (§8.6).
- **Execution-model posture** — native transport stays sync ureq on the
  capacity-bounded pool; no tokio.
