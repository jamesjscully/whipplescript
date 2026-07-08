// Local end-to-end validation of the DO runtime WITHOUT Cloudflare (DR-0033 5d):
// drives the ACTUAL wasm module through the REAL wasm-bindgen boundary, backed by
// real SQLite (node:sqlite) as the DoSqlBridge. This is the live worker path minus
// only Cloudflare's `state.storage.sql` (swapped for node:sqlite) + `wrangler deploy`.
const assert = require("node:assert");
const fs = require("node:fs");
const path = require("node:path");
const { DatabaseSync } = require("node:sqlite");
const { WasmDurableInstance } = require("./pkg/whipplescript_host_do.js");

const schema = fs.readFileSync(path.join(__dirname, "do_schema.sql"), "utf8");

// A fresh DO SQLite + the DoSqlBridge the wasm core imports.
function freshInstanceEnv(seedSql) {
  const db = new DatabaseSync(":memory:");
  db.exec(schema);
  for (const stmt of seedSql) db.exec(stmt);
  const bridge = {
    exec(sql, paramsJson) {
      const params = JSON.parse(paramsJson);
      return Number(db.prepare(sql).run(...params).changes);
    },
    query(sql, paramsJson) {
      const params = JSON.parse(paramsJson);
      const rows = db.prepare(sql).all(...params);
      // Positional in SELECT order; BigInt -> Number so JSON.stringify is safe.
      const positional = rows.map((row) =>
        Object.values(row).map((v) => (typeof v === "bigint" ? Number(v) : v)),
      );
      return JSON.stringify(positional);
    },
  };
  return bridge;
}

// 1) An effect-free workflow drives to a terminal in one step.
function testEffectFree() {
  const source = [
    "workflow MinimalNoop", "", "output result StartupSeen", "",
    'class StartupSeen {', "  source string", '  state "observed"', "}", "",
    "rule observe_start", "  when started", "=> {",
    '  record StartupSeen {', '    source "external.started"', '    state "observed"', "  }", "",
    "  complete result {", '    source "external.started"', '    state "observed"', "  }", "}",
  ].join("\n");
  const bridge = freshInstanceEnv([]);
  const inst = WasmDurableInstance.create(bridge, source, "{}", "local/MinimalNoop", undefined, undefined);
  const outcome = JSON.parse(inst.step(undefined, Date.now()));
  assert.strictEqual(outcome.kind, "terminal", `effect-free: ${JSON.stringify(outcome)}`);
  assert.strictEqual(inst.status(), "completed");
  console.log("PASS  effect-free workflow -> completed (one step)");
}

// 2) A coerce workflow SUSPENDS on fetch and RESUMES to a terminal across two
//    separate step() calls -- the real durable-object sans-IO pattern, through wasm.
function testCoerceSuspendResume() {
  const source = [
    "workflow CoerceScore", "", "output result Decision", "",
    "class Decision {", "  score float", "}", "",
    "coerce scoreIt() -> Decision {", '  prompt """', "  Score it.", "  {{ ctx.output_format }}", '  """', "}", "",
    "rule go", "  when started", "=> {", "  coerce scoreIt() as review",
    "  after review succeeds as decision {", "    complete result { score decision.score }", "  }",
    "  after review fails {", "    complete result { score 0.0 }", "  }", "}",
  ].join("\n");
  const seed = [
    "INSERT INTO capability_schemas (capability, description, schema_json) VALUES ('schema.coerce', 'Coerce.', '{}')",
    "INSERT INTO effect_providers (provider_id, effect_kind, provider, capability, config_json) VALUES ('provider_coerce_builtin', 'schema.coerce', 'builtin-coerce', 'schema.coerce', '{}')",
    "INSERT INTO capability_bindings (binding_id, program_id, capability, provider, config_json) VALUES ('binding_coerce_builtin', NULL, 'schema.coerce', 'builtin-coerce', '{}')",
  ];
  const bridge = freshInstanceEnv(seed);
  const coerceConfig = JSON.stringify({
    provider: "anthropic", base_url: "https://api.anthropic.com",
    api_key: "test-key", model: "claude-test", max_tokens: 1024,
  });
  const inst = WasmDurableInstance.create(bridge, source, "{}", "local/CoerceScore", coerceConfig, undefined);

  // First step: the coerce effect suspends on `fetch`.
  const first = JSON.parse(inst.step(undefined, Date.now()));
  assert.strictEqual(first.kind, "needs_http", `coerce step1: ${JSON.stringify(first)}`);
  assert.ok(first.request.url.includes("anthropic"), "request targets the provider");

  // The shell performs the fetch; feed a canned Anthropic structured output back.
  const response = JSON.stringify({
    status: 200,
    body: {
      content: [{ type: "tool_use", name: "Decision", input: { score: 0.9 } }],
      usage: { input_tokens: 1, output_tokens: 1 },
    },
  });
  const second = JSON.parse(inst.step(response, Date.now()));
  assert.strictEqual(second.kind, "terminal", `coerce step2: ${JSON.stringify(second)}`);
  assert.strictEqual(inst.status(), "completed");
  console.log("PASS  coerce workflow -> needs_http -> (fetch) -> terminal (two steps)");
}

// 3) An AGENT workflow: the multi-round turn's first model call SUSPENDS on fetch
//    (the real Anthropic messages request the MessagesApiClient built), and a canned
//    final reply RESUMES it to a terminal -- the agent counterpart to (2), through wasm.
function testAgentSuspendResume() {
  const source = [
    "workflow AgentDemo", "", "output result Done", "",
    "class Done {", "  ok int", "}", "",
    "agent helper {", "  provider owned", '  profile "repo-reader"', "  capacity 1", "}", "",
    "rule go", "  when started", "=> {", '  tell helper as reply """', "  Do the thing.", '  """', "",
    "  after reply succeeds {", "    complete result { ok 1 }", "  }", "",
    "  after reply fails {", "    complete result { ok 0 }", "  }", "}",
  ].join("\n");
  const seed = [
    "INSERT INTO capability_schemas (capability, description, schema_json) VALUES ('agent.tell', 'Run an agent turn.', '{}')",
    "INSERT INTO effect_providers (provider_id, effect_kind, provider, capability, config_json) VALUES ('provider_agent_tell_builtin', 'agent.tell', 'builtin-agent-harness', 'agent.tell', '{}')",
    "INSERT INTO capability_bindings (binding_id, program_id, capability, provider, config_json) VALUES ('binding_agent_tell_builtin', NULL, 'agent.tell', 'builtin-agent-harness', '{}')",
    "INSERT INTO profiles (profile_id, name, description, enforcement_mode, allowed_capabilities, config_json) VALUES ('profile_repo_reader', 'repo-reader', 'reads', 'enforce', '[\"agent.tell\"]', '{}')",
  ];
  const bridge = freshInstanceEnv(seed);
  const agentConfig = JSON.stringify({
    provider: "anthropic", base_url: "https://api.anthropic.com",
    api_key: "test-key", model: "claude-test", max_tokens: 4096,
  });
  const inst = WasmDurableInstance.create(bridge, source, "{}", "local/AgentDemo", undefined, agentConfig);

  // First step: the agent turn's first model call suspends on `fetch`.
  const first = JSON.parse(inst.step(undefined, Date.now()));
  assert.strictEqual(first.kind, "needs_http", `agent step1: ${JSON.stringify(first)}`);
  assert.ok(first.request.url.endsWith("/v1/messages"), "request targets the messages endpoint");

  // The shell performs the fetch; feed a canned Anthropic final reply (no tool calls).
  const response = JSON.stringify({
    status: 200,
    body: {
      content: [{ type: "text", text: "did the thing" }],
      usage: { input_tokens: 1, output_tokens: 1 },
    },
  });
  const second = JSON.parse(inst.step(response, Date.now()));
  assert.strictEqual(second.kind, "terminal", `agent step2: ${JSON.stringify(second)}`);
  assert.strictEqual(inst.status(), "completed");
  console.log("PASS  agent workflow -> needs_http -> (fetch) -> terminal (two steps)");
}

testEffectFree();
testCoerceSuspendResume();
testAgentSuspendResume();
console.log("\nALL PASS: the wasm DO runtime drives real workflows over real SQLite through the wasm-bindgen boundary.");
