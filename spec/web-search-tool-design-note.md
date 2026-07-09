# The web search tool — Design Note (SearchProvider trait, Brave first)

**Status: ACCEPTED DESIGN NOTE (Jack, 2026-07-07; pre-build — a
build-time DR formalizes the trait + provider protocol; §8 residuals
land at build).** This note is the search half
of the **network-tool policy discussion** that the web search item in
`owned-harness-tool-surface.md` has been gated on since the 2026-07-01
surface-hardening pass. That item's settled shape (IFC egress/ingress,
capability via `with access to`, owned-harness only) is imported as
ground, not reopened. Scope is **search only** — the web *fetch* tool is
a separate open question (§8) after fetchkit was dropped 2026-07-07
(fetch-only, no search; its design note was withdrawn).

**Decision log (Jack):** 2026-07-07 — agreed the full composition (§2);
first first-party provider = **Exa**, revised the same day to **Brave
first, Exa deferred**.

## 1. Landscape ground truth (verified 2026-07-07)

Microsoft retired the Bing Search API in 2025. Brave is now the only
large independent western search index available to developers; it
removed its free plan in Feb 2026 (~$5/1k queries on the base paid
plan). The LLM-native providers: Tavily (~$8/1k pay-as-you-go, 1k free
credits/mo, extraction-ready results) and Exa (semantic/neural search,
~$5/1k entry, per-search cost varies by mode). Serper (~$0.30–1/1k) is a
Google-SERP wrapper — cheapest, weakest independence/durability story.
Model providers ship server-side search tools: Anthropic's web_search is
$10/1k searches plus tokens.

## 2. The settled composition

Three decisions, settled together (Jack, 2026-07-07):

1. **A `SearchProvider` trait is the durable artifact.** One async
   operation: query (+options) → ranked results in a familiar schema
   (title / url / snippet / published). Providers are ~a hundred lines
   each behind it; blessing a provider is configuration, not
   architecture.
2. **Resolution chain, progressive-rigor ordered:**
   - a **configured dedicated provider** if a key exists (v1: Brave,
     §4) — best quality-per-dollar, structured results;
   - else **model-provider-native search** through credentials the
     workspace already holds (§5) — the zero-config floor. Degraded
     (pricier per search, provider-mediated) and honestly tagged, but it
     works the day a workspace can run an `agent` effect at all, and it
     has the decisive IFC property: the query egresses only to a party
     that already receives the workspace's prompts — **no new reader**;
   - else the tool is **absent with a clear "configure a search
     provider" message**.
3. **Scraping/meta-search is rejected as a default** (DDG html,
   hosted SearXNG): ToS-gray, brittle, unattributable — against the
   evidence-grade legibility whip sells everywhere else. A self-hosted
   SearXNG *provider impl* may exist later for operators who run one;
   never the default.

## 3. Tool surface

One tool, familiar shape (reference: the dominant trained-on
WebSearch tools):

```jsonc
// web_search
input:  { "query": string,
          "allowed_domains"?: [string],   // maps to provider domain filters
          "blocked_domains"?: [string],
          "freshness"?: string }          // provider-mapped date filter
output: [ { "title": string, "url": string, "snippet": string,
            "published"?: string } ]
```

Result list is data, not fetched content — reading a result's page is a
fetch-tool concern (§8). `count`/pagination capped by policy (Brave
caps count at 20/offset at 9 anyway). The tool is exposed only when the
turn's grants include the search capability (`with access to`), same as
every brokered tool.

## 4. Brave provider (v1 first-party impl)

API surface pinned 2026-07-07: `GET
https://api.search.brave.com/res/v1/web/search`, auth header
`X-Subscription-Token`. Request: `q` (supports `site:`/`filetype:`/
quoted operators), `count` (≤20), `offset` (≤9), `country`,
`search_lang`, `freshness` (`pd`/`pw`/`pm`/`py`/range), `safesearch`,
`extra_snippets` (≤5 extra excerpts). Response: `web.results[]` with
`title`, `url`, `description`, `page_age`, optional `extra_snippets`;
`query.more_results_available` for pagination.

Mapping is direct: `description` (+`extra_snippets` concatenation under
a size cap) → `snippet`; `page_age` → `published`. Key arrives via the
P6 secrets path, scoped to the search capability. `safesearch` and
default `freshness` are workspace policy knobs with progressive-rigor
defaults (moderate / unset). Why Brave first: independent index (the
durability argument — every SERP wrapper inherits Google's terms), the
best price among independents, and the simplest possible API to sit
behind the trait.

## 5. The zero-config floor: provider-native search

Every workspace that can run `agent`/`coerce` already holds a
model-provider credential. Anthropic's web_search server tool can be
driven by a minimal Haiku request returning structured result blocks →
mapped to the same schema, tagged `provider-mediated`. Cost ~$10/1k +
minimal tokens — worse than Brave, which is the correct progressive-
rigor shape: works at zero setup, engages better as assets (a Brave
key) accrue. **Open probe:** the codex/ChatGPT-token path may not
permit arbitrary API search calls — verify; if it doesn't, the floor
exists only for workspaces with an Anthropic console key, and the
absent-with-message tier covers the rest.

## 6. IFC and effects (imported + mechanism map)

- **Query = egress**, flow-checked like `send`. Each provider is a
  **distinct egress principal visible in the grant** — Brave is a
  fourth party and the grant says so; the provider-native floor adds no
  reader beyond the model provider already in the loop.
- **Results = low-integrity ingress** (taint source; snippets are
  attacker-influenced text — the integrity label is the mitigation).
- Each search is a whip **effect**: idempotency-keyed, recorded,
  replay-served-from-record (a replayed turn never re-searches),
  metered by the existing spend machinery (per-search provider cost is
  knowable: Brave ~$0.005, floor ~$0.01+).

## 7. Deferred providers (with cause)

- **Exa** — deferred 2026-07-07 (Jack; was briefly first). API pinned
  while evaluating: `POST https://api.exa.ai/search`, `x-api-key`,
  modes instant/fast/auto/deep, domain/date filters, contents options
  (text/highlights/summary). It is a *semantic-search capability* more
  than a Brave competitor; revisit when an agent workload wants
  meaning-shaped queries.
- **Tavily** — deferred; strongest "LLM-reads-the-result" shaping;
  candidate second impl if Brave snippet quality proves thin for agent
  use.
- **Serper / SearXNG** — unscheduled; wrapper-durability and
  operator-hosted stories respectively.

## 8. Open items

1. **The web fetch tool** — designed 2026-07-07:
   [`web-fetch-tool-design-note.md`](web-fetch-tool-design-note.md)
   (in-house, GET-only, htmd converter). Shared with it: the
   domain-policy declarations and the quality-calibration corpus.
2. Codex-path probe for the provider-native floor (§5).
3. Quality calibration (non-gating now Brave is decided): Brave vs the
   floor on a dev-docs-heavy corpus of real agent queries — informs
   snippet sizing and whether Tavily gets pulled forward.
4. Result-schema residuals: max snippet bytes, dedup, `published`
   normalization.
5. Brave plan choice + rate-limit handling (429 → typed effect failure).
6. Search-result caching: none in v1 beyond effect recording (searches
   are non-hermetic ingress); revisit if spend data argues.
7. DO build boxes: tool executor over `NeedsHttp`; TS-shell egress
   allowlist entry for `api.search.brave.com`.

## 9. Settled vs. open

**Settled (Jack, 2026-07-07):** the composition (§2 — trait, resolution
chain, scraping rejected); Brave = first first-party provider; Exa +
Tavily + others deferred; zero-config floor = model-provider-native
search; search/fetch scope split (this note is search only).

**Open:** §8; ADR + build sequencing (owned-harness tool build home,
v0.3 cloud/owned-harness slice).

## 10. Relationships

- **`owned-harness-tool-surface.md`** — canonical home of the web
  search item; this note is its design; the network-tool-policy gate
  closed with acceptance of this note and the fetch note (2026-07-07).
- **DR-0033 / `compute-plane-design-note.md`** — `NeedsHttp` on the
  step machine (DO path); egress-allowlist discipline shared with
  sidecar containers.
- **`information-flow-surface.md` / `information-flow-governance.md`**
  — egress flow-check on queries; low-integrity ingress labeling;
  no-new-readers argument for the floor (§2, §5).
- **DO tracker P6 (secrets)** — provider keys arrive scoped.
- **`in-isolate-bash-design-note.md`** — a future in-isolate `curl`
  builtin rides the same NeedsHttp + egress policy machinery.
- **`claude-agent-sdk-strategy.md`** — command-backed agents keep their
  native web tools; this surface is owned-harness only.
