// Cloudflare Worker + Durable Object shell for the WhippleScript sans-IO core
// (DR-0033 Phase 5). This is the JavaScript side of the host boundary: it owns
// the async primitives (`fetch`, alarms, storage) and drives the *synchronous*
// Rust step machine, awaiting `fetch` on a `NeedsIo(Http)` and re-entering the
// synchronous `step` on resolve — the whole point of the sans-IO design.
//
// The Rust core (crate `whipplescript-host-do`, built with `--no-default-features`
// for wasm) is compiled to wasm by `wasm-pack` and imported below. The Rust side
// exposes a driver that takes host callbacks (this file) and returns either a
// pending HTTP request or a settled outcome.

import initWasm, {
  createInstance,
  type StepResult,
  type HttpRequest,
} from "./pkg/whipplescript_host_do";

export interface Env {
  WHIPPLE_INSTANCE: DurableObjectNamespace;
  WHIPPLE_OBJECTS: R2Bucket;
  WHIPPLE_FILE_SPILL_THRESHOLD_BYTES: string;
  // Provider credentials arrive as secrets (the Rust `Secrets` plane).
  ANTHROPIC_API_KEY?: string;
  OPENAI_API_KEY?: string;
}

// The host callbacks the Rust core calls. Each maps one Rust host trait
// (FetchClient / DoStorage / Alarms / Secrets / ObjectStore) onto a DO primitive.
interface HostBindings {
  // DoStorage: the DO's synchronous SQLite.
  sqlExec(sql: string, params: unknown[]): unknown[];
  // Alarms: single-wake-up scheduler.
  setAlarm(atUnixMs: number): void;
  getAlarm(): number | null;
  // Secrets: config/credentials plane.
  getSecret(name: string): string | null;
}

/** One durable workflow instance = one Durable Object (single-writer). */
export class WhippleInstance {
  private state: DurableObjectState;
  private env: Env;
  private ready: Promise<void>;

  constructor(state: DurableObjectState, env: Env) {
    this.state = state;
    this.env = env;
    // Initialize the wasm module once per isolate.
    this.ready = initWasm().then(() => undefined);
  }

  private bindings(): HostBindings {
    const sql = this.state.storage.sql;
    return {
      sqlExec: (query, params) => Array.from(sql.exec(query, ...params)),
      setAlarm: (atUnixMs) => this.state.storage.setAlarm(atUnixMs),
      getAlarm: async () => (await this.state.storage.getAlarm()) ?? null,
      getSecret: (name) => (this.env as Record<string, unknown>)[name] as string ?? null,
    } as unknown as HostBindings;
  }

  /** External entry: enqueue input, then drive the instance to a quiescent point. */
  async fetch(request: Request): Promise<Response> {
    await this.ready;
    const input = await request.text();
    const instance = createInstance(this.bindings(), input);
    await this.drive(instance);
    return new Response(JSON.stringify(instance.snapshot()), {
      headers: { "content-type": "application/json" },
    });
  }

  /** Alarm entry: a scheduled wake-up (clock-source/timer) — resume stepping. */
  async alarm(): Promise<void> {
    await this.ready;
    const instance = createInstance(this.bindings(), null);
    await this.drive(instance);
  }

  // The sans-IO drive loop: step the synchronous Rust machine; on NeedsIo(Http)
  // await the DO's `fetch` and re-enter with the response; stop when it settles
  // or parks (needs another wake-up). Eviction between the request and the
  // re-entry is safe: durable step state survives and the request is retried
  // (at-least-once + idempotency key — DR-0033 Decision 3).
  private async drive(instance: {
    step(incoming: unknown | null): StepResult;
    snapshot(): unknown;
  }): Promise<void> {
    let incoming: unknown | null = null;
    for (;;) {
      const result = instance.step(incoming);
      if (result.kind === "settled" || result.kind === "parked") {
        return;
      }
      // kind === "needs_http": perform the fetch, then re-enter.
      incoming = await this.performFetch(result.request);
    }
  }

  private async performFetch(req: HttpRequest): Promise<unknown> {
    try {
      const response = await fetch(req.url, {
        method: req.method ?? "POST",
        headers: req.headers,
        body: JSON.stringify(req.body),
      });
      const body = await response.json().catch(() => ({}));
      return { ok: { status: response.status, body } };
    } catch (error) {
      // Map a transport failure to the Rust `TransportError`.
      const message = error instanceof Error ? error.message : String(error);
      return { err: message.includes("timeout") ? { timeout: true } : { transport: message } };
    }
  }
}

// The top-level Worker: route a request to the addressed instance's Durable
// Object (one DO per workflow instance id).
export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url);
    const instanceId = url.pathname.slice(1) || "default";
    const id = env.WHIPPLE_INSTANCE.idFromName(instanceId);
    return env.WHIPPLE_INSTANCE.get(id).fetch(request);
  },
};
