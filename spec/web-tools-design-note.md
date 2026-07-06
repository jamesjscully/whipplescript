# Web tools (fetch + search) — Design Note (fetchkit as the default engine)

**Status: DRAFT DESIGN NOTE (pre-ADR; needs research + refinement
passes).** Opened by Jack 2026-07-05 alongside the in-isolate bash note.
This is (part of) the **network-tool policy discussion** that the web
search item in `owned-harness-tool-surface.md` has been gated on since
the 2026-07-01 surface-hardening pass — that item's settled shape (IFC
egress/ingress, capability via `with access to`, owned-harness only) is
imported here as ground, not reopened. The engine candidate, **fetchkit**
(everruns, MIT, crates.io v0.4.1), was validated by spike 2026-07-05.

## 1. Two tools, one engine — and what fetchkit is not

The model-facing surface is two tools with familiar shapes:

- **`web_fetch`** — URL in, LLM-shaped content out (markdown by default;
  text/metadata-only for binaries). fetchkit provides this directly.
- **`web_search`** — query in, ranked results out. **fetchkit does not
  provide this** — verified against 0.4.1 source: its 13 specialized
  fetchers (github repo/issue/code, arxiv, wikipedia, stackoverflow,
  hackernews, docs sites, rss, youtube, twitter, package registries) are
  URL-aware *content* dispatchers, not query engines. A search tool needs
  a **search provider** in front; that fork is the main open research
  item (§6).

"fetchkit as the default search tool" therefore lands as: fetchkit is the
default **engine for the web surface** — all fetching, conversion,
SSRF/allowlist enforcement, and specialized-source handling — with the
search provider as a pluggable front that resolves queries to URLs+
snippets, whose full-content reads then go through fetchkit.

## 2. Spike results (2026-07-05)

- **Fetch → markdown works** (native): status/headers/content returned;
  conversion quality good (scripts stripped, headings/links/lists/code
  fences correct).
- **SSRF default-deny verified empirically**: cloud metadata
  (169.254.169.254), loopback, and RFC1918 probes all blocked out of the
  box (resolve-then-check DNS policy); `allow_prefixes` enforced
  (off-allowlist fetch refused). Body-size cap (10 MB, truncation),
  first-byte/body timeouts, port restrictions, blocked-host suffix rules
  all present as options.
- **The transport is pluggable**: `HttpTransport` is a single async
  `execute(TransportRequest) -> TransportResponse` trait; reqwest is only
  the default impl. This is the DO seam (§4).
- **The conversion half is pure**: `convert.rs` (1,656 lines,
  HTML→markdown/text, boilerplate stripping, metadata/heading/link
  extraction) depends on nothing but two small structs. Extracted and
  compiled to wasm32-unknown-unknown: **79 KB raw / 32 KB gz, output
  identical to native** in V8.
- **Whole-crate wasm: no** — hard tokio/reqwest deps refuse the target.
  Irrelevant for the DO (§4), matters only as upstream-PR shape.
- Extra: a `bot-auth` feature (Ed25519 request signing per
  draft-meunier-web-bot-auth) — relevant later for agent-identity
  headers; not v1.

## 3. IFC and governance (imported settled shape + mechanism map)

From the surface-hardening pass, unchanged: the **query/URL is an
egress** (flow-checked like `send` — a URL can exfiltrate anything that
can be string-encoded), the **result is a low-integrity ingress** (taint
source, like an inbound message; markdown conversion does *not* sanitize
prompt injection — the integrity label is the mitigation, not the
converter). The capability is grantable via `with access to`; a subagent
gets web tools only if delegated them; owned-harness only (command-backed
Claude/Codex keep their native web tools).

Mechanism map: fetchkit's SSRF layer is **mechanism beneath policy** —
private-network deny is unconditional (not operator-configurable off);
domain allow/block lists derive from workspace declarations the same way
sidecar egress allowlists derive from exec declarations
(compute-plane §6: designed deliberately, not inherited). Ports, body
caps, and redirect confinement are envelope parameters with
progressive-rigor defaults.

## 4. Architecture per runtime

- **Native harness**: fetchkit as-is (validated). The tool executor is a
  thin facade: options assembled from grants/policy, request out, labeled
  result in.
- **DO**: HTTP must ride the DR-0033 machine — the tool raises
  `NeedsHttp`, the TS shell fetches (Workers `fetch()` respects the
  platform's own egress story), the response comes back as an effect
  result; then the **wasm-side conversion half** (verified 32 KB gz)
  shapes it. Two routes to that half: (a) upstream PR feature-gating
  transport/tokio so `fetchkit` compiles `--no-default-features` on wasm
  (the `HttpTransport` trait says the seam is already theirs
  conceptually), or (b) vendor `convert.rs`+types (small, self-contained,
  MIT). Vendor now, PR in parallel; converge on (a).
  SSRF checks re-land in the TS shell for the DO path (the URL policy
  check runs wasm-side *before* the effect is raised; the
  resolve-then-check IP guard must run where DNS happens).
- **Search provider** (§6) is HTTP like any other — same seams both
  runtimes.

## 5. Effects, recording, and caching

Each web call is a whip **effect** with the standard idempotency-key
discipline: recorded, replay-served-from-record (a replayed turn never
re-fetches), spend-metered. Fetches are inherently non-hermetic —
they are **recorded ingress**, never Class-A-cacheable results; but
fetchkit surfaces `ETag`/`Last-Modified`, so a workspace-level fetch
cache with conditional revalidation is a natural economy layer (open:
retention + staleness policy; joins the versioned-workspace retention
question only loosely — this is a cache, not history).

## 6. The search-provider fork (main open research)

Options, deliberately not settled here:

1. **Hosted search API** (Brave, Exa, Tavily, Bing…): quality and
   simplicity; costs per-query money and an API key (secrets machinery
   P6); an external data processor for every query (IFC egress to a
   *fourth party* — must be visible in the grant, not folded silently
   into "web").
2. **Meta-search over public endpoints** (DDG html, SearXNG instance):
   no key, weaker quality/stability, ToS gray zones.
3. **Operator-configured provider trait** with no default: honest but
   violates progressive rigor (search should work at zero setup,
   degraded and tagged if need be).

Research items: provider quality/cost matrix for agent workloads; whether
a zero-config default exists that is ToS-clean; whether the provider
trait should be the same `HttpTransport`-shaped seam; result schema
(match the familiar title/url/snippet shape models are trained on).
Leaning: provider trait + one blessed zero-config default chosen by the
matrix, richer providers as configuration — but the matrix has to be
built first.

## 7. What the requirements pass must settle

1. Search provider fork (§6) — the gating item.
2. Tool schemas: match the field shapes of the dominant trained-on tools
   (Claude Code's WebFetch/WebSearch are the reference); `web_fetch`
   prompt/extraction parameter or raw-markdown-only in v1?
3. Domain policy surface: where workspace allow/block lists are declared
   and how they compose with per-turn grants (same-declaration discipline
   as exec allowlists).
4. Specialized fetchers: expose which of the 13 in v1 (github + docs +
   wikipedia + stackoverflow are the obvious agent set); each adds an
   implicit third-party endpoint — enumerate, don't blanket-enable.
5. Fetch cache policy (§5): scope, retention, revalidation.
6. Budgets: per-turn fetch/search counters (existing counter machinery);
   response-size caps vs context economy (truncate-with-marker already
   fetchkit's behavior on body-cap hit).
7. DO path build boxes: NeedsHttp executor for the tool, TS-shell SSRF
   guard, vendored converter, upstream PR.
8. bot-auth adoption timing (agent identity signing) — later, but decide
   whether the door matters for the provider choice.

## 8. Settled vs. open

**Settled in principle:** two tools, one engine; fetchkit as that engine
(native validated; DO via NeedsHttp + pure conversion half, both
verified); IFC shape imported from the surface-hardening pass; SSRF
private-network deny unconditional; effects recorded/replayed like all
effects.

**Open:** the search-provider fork (§6, gating); everything in §7; the
formal close-out of the "network-tool policy discussion" gate — this
note is the draft of that discussion, and Jack settles it.

## 9. Relationships

- **`owned-harness-tool-surface.md`** — canonical home of the web search
  open item; this note is its design; on acceptance that item points
  here.
- **DR-0033 / `compute-plane-design-note.md`** — NeedsHttp on the step
  machine; egress-allowlist discipline shared with sidecar containers.
- **`information-flow-surface.md` / `information-flow-governance.md`** —
  egress flow-check on queries/URLs; low-integrity ingress labeling on
  results.
- **`in-isolate-bash-design-note.md`** — a future in-isolate `curl`
  builtin would ride this exact NeedsHttp + policy machinery (its §9.6).
- **P6 secrets (DO tracker)** — search-provider API keys arrive scoped.
- **`claude-agent-sdk-strategy.md`** — command-backed agents keep native
  web tools; this surface is owned-harness only.
