# WhippleScript runtime — Cloudflare Worker + Durable Object shell (DR-0033 chunk 5d)

This is the **live-deployment shell** for the sans-IO runtime. The Rust core
(`crate whipplescript-host-do`) is compiled to wasm and exposes
`WasmDurableInstance` (`create` / `attach_host` / `step` / `status`). The shell
also exposes the authenticated `whipplescript.host.v1` placement API: signed
policy bootstrap, package/instance open, workspace sync and projection,
turn/cancel/result, evidence/events, and checkpoint/restore.

## What is done vs. what needs a deployment

Everything in Rust is built, proven against a real SQLite engine (native + wasm),
and wasm-clean: the instance scheduler, the store, every effect family, both HTTP
effects (sans-IO suspend/resume), eviction-safe agent turns, the
`create`/`step`/`status` handle, and the `#[wasm_bindgen]` surface. `src/index.ts`
+ `wrangler.toml` are the shell that plugs those into a live Durable Object.

**Chunk 5d — live validation — is the only step that cannot happen in-repo.** It
needs a Cloudflare account and:

## Deploy steps

**One command:** `whip deploy` (compute plane P8) runs the whole sequence
below — dependency install, wasm build, optional transitional provider secrets
(`--set-secrets` forwards `ANTHROPIC_API_KEY`/`OPENAI_API_KEY` from the local
environment), and the wrangler deploy. `--dry-run` validates the bundle
without publishing; `--worker-dir`/`WHIPPLESCRIPT_WORKER_DIR` point it at
this directory when running outside the repo. The manual steps, for
reference:

1. **Build the wasm module** (from the `#[wasm_bindgen]` surface):
   ```
   npm install
   npm run build:wasm      # wasm-pack build .. --target bundler --out-dir worker/pkg
   ```
2. **Apply the runtime schema to the DO SQLite.** The DO's SQLite must carry the
   runtime store schema (`crates/whipplescript-store/migrations/0001_runtime_store.sql`,
   the same schema `do_store.rs` is ported against). Run it once on object init
   (embed the SQL and `sql.exec` it before `create`, or ship it as a DO migration).
3. **Choose provider egress** (for governed host turns). The transitional
   `worker-secret` realization resolves a named Worker secret only after
   admission. Its binding must explicitly set `"execution":"worker-secret"`
   and name `OPENAI_API_KEY` or `ANTHROPIC_API_KEY` in `secret`:
   ```
   wrangler secret put ANTHROPIC_API_KEY
   wrangler secret put OPENAI_API_KEY
   ```
   The DR-0042 `model-broker` realization instead sets
   `WHIP_MODEL_BROKER_URL`, stores only the broker-hop token as
   `WHIP_MODEL_BROKER_TOKEN`, and declares no provider secret:
   ```json
   {
     "binding_project_openai": {
       "credential_id": "credential:project:alpha:v3",
       "provider": "openai",
       "model": "gpt-5",
       "base_url": "https://api.openai.com",
       "execution": "model-broker"
     }
   }
   ```
   This JSON is the non-secret `WHIP_HOST_PROVIDER_BINDINGS_JSON` deployment
   variable. Set the hop token with
   `wrangler secret put WHIP_MODEL_BROKER_TOKEN`. Broker failure is fail-closed;
   the Worker never falls back to direct provider egress.
4. **Deploy**: `npm run deploy` (`wrangler deploy`).
5. **Validate**: use the canonical managed route
   `/v1/tenants/:tenant/placements/:placement/host/...` (Bearer
   `WHIP_CONTROL_TOKEN`), or `POST /start` for the legacy workflow surface, and confirm the
   instance drives to `completed` — an effect-free workflow in one `step`, a coerce/
   agent workflow across `needs_http` rounds.

## The integration contract (three seams)

- **`DoSqlBridge` → `state.storage.sql`** — `makeBridge` in `index.ts`. Rows come
  back positionally (`Object.values`, column order preserved per Cloudflare docs).
- **`DurableEffectPorts` → admitted provider realization** — governed host turns
  resolve either a transitional Worker secret or the fixed non-secret broker
  sentinel. The binding id + opaque credential ref must match exactly before
  either is available.
- **`needs_http` → egress** — `performFetch` handles direct/container rounds;
  `performModelBrokerFetch` strips sentinel auth and sends the admitted request
  through the authenticated `whipplescript.model-egress.v1` broker envelope.

Once these are wired and the object deployed, 5d is exercising already-proven Rust.
