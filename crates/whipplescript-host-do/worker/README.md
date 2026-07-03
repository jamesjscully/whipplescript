# WhippleScript runtime — Cloudflare Worker + Durable Object shell (DR-0033 chunk 5d)

This is the **live-deployment shell** for the sans-IO runtime. The Rust core
(`crate whipplescript-host-do`) is compiled to wasm and exposes
`WasmDurableInstance` (`create` / `step` / `status`); this shell adds only the JS
glue — a `DoSqlBridge` over `state.storage.sql`, the `fetch` for each suspended
round, and step-protocol JSON marshalling. **No scheduling logic lives here.**

## What is done vs. what needs a deployment

Everything in Rust is built, proven against a real SQLite engine (native + wasm),
and wasm-clean: the instance scheduler, the store, every effect family, both HTTP
effects (sans-IO suspend/resume), eviction-safe agent turns, the
`create`/`step`/`status` handle, and the `#[wasm_bindgen]` surface. `src/index.ts`
+ `wrangler.toml` are the shell that plugs those into a live Durable Object.

**Chunk 5d — live validation — is the only step that cannot happen in-repo.** It
needs a Cloudflare account and:

## Deploy steps

1. **Build the wasm module** (from the `#[wasm_bindgen]` surface):
   ```
   npm install
   npm run build:wasm      # wasm-pack build .. --target bundler --out-dir worker/pkg
   ```
2. **Apply the runtime schema to the DO SQLite.** The DO's SQLite must carry the
   runtime store schema (`crates/whipplescript-store/migrations/0001_runtime_store.sql`,
   the same schema `do_store.rs` is ported against). Run it once on object init
   (embed the SQL and `sql.exec` it before `create`, or ship it as a DO migration).
3. **Set provider secrets** (for coerce/agent effects):
   ```
   wrangler secret put ANTHROPIC_API_KEY
   wrangler secret put OPENAI_API_KEY
   ```
4. **Deploy**: `npm run deploy` (`wrangler deploy`).
5. **Validate**: `POST /start { program, input, principal }` and confirm the
   instance drives to `completed` — an effect-free workflow in one `step`, a coerce/
   agent workflow across `needs_http` rounds.

## The integration contract (three seams)

- **`DoSqlBridge` → `state.storage.sql`** — `makeBridge` in `index.ts`. Rows come
  back positionally (`Object.values`, column order preserved per Cloudflare docs).
- **`DurableEffectPorts` → secrets** — `create` accepts `coerce_config_json` (DO
  secret), so **coerce runs on the deployed surface** alongside store-only + effect-
  free workflows. The `index.ts` shell builds it from `ANTHROPIC_API_KEY`. *Remaining
  follow-on: the messages-API agent model client (a pure Rust `HttpModelClient` like
  coerce's `build_request`/`parse_response`, but for the tool-use messages API) — its
  eviction-safe multi-round machinery is already built + proven (`snapshot`/`restore`).*
- **`needs_http` → `fetch`** — `performFetch` in `index.ts`.

Once these are wired and the object deployed, 5d is exercising already-proven Rust.
