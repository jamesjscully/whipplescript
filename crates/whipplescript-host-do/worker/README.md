# WhippleScript Durable Object host (Worker shell)

The Cloudflare Worker + Durable Object shell for the sans-IO WhippleScript core
(DR-0033 Phase 5). The Rust crate above (`whipplescript-host-do`) is the host
binding; this directory is the JavaScript side that owns the async primitives and
drives the synchronous Rust step machine.

## Architecture

```
Worker fetch / DO alarm  (src/index.ts)
        │  host callbacks: fetch · state.storage.sql · setAlarm · env secrets · R2
        ▼
wasm module  (whipplescript-host-do, built --no-default-features → wasm32)
        │  FetchHost · DoFileStore/DoStorage · Alarms · Secrets · TieredFileStore/ObjectStore
        ▼
sans-IO core  (whipplescript-kernel + whipplescript-store, no rusqlite)
```

The DO class `WhippleInstance` runs the **sans-IO drive loop**: it steps the
synchronous Rust machine; on `NeedsIo(Http)` it awaits the DO's `fetch` and
re-enters the synchronous `step` with the response — the isolate can be evicted
in between, and the durable step state + a stable idempotency key make the retry
safe (at-least-once, DR-0033 Decision 3).

## Build + deploy

The one piece not yet wired is the `wasm-bindgen` surface on the Rust crate
(`createInstance` / `step` / `snapshot` and the `RuntimeStore`-over-`DoStorage`
SQL implementation) — those need a live Cloudflare DO to build and test against.
The seams they plug into are implemented and tested in the Rust crate.

```sh
# 1. Build the Rust core to wasm (produces ./pkg with the wasm-bindgen glue):
npm run build:wasm      # wasm-pack build .. --target bundler --no-default-features

# 2. Provider credentials as Worker secrets (the Rust Secrets plane):
npx wrangler secret put ANTHROPIC_API_KEY
npx wrangler secret put OPENAI_API_KEY

# 3. Type-check and deploy:
npm run typecheck
npm run deploy          # wrangler deploy
```

## What maps to what

| Rust host trait | DO primitive (this shell) |
|---|---|
| `FetchClient` / `FetchHost` | the Worker's global `fetch` |
| `DoStorage` / `RuntimeStore` | `state.storage.sql` (synchronous DO SQLite) |
| `Alarms` | `state.storage.setAlarm` / `getAlarm` |
| `Secrets` | Worker secret bindings (`env`) |
| `ObjectStore` (large tier) | the R2 bucket `WHIPPLE_OBJECTS` |
