import assert from "node:assert/strict";
import test from "node:test";
import {
  MODEL_AUTH_SENTINEL,
  MODEL_EGRESS_PROTOCOL,
  performModelBrokerFetch,
  stripSentinelAuthentication,
} from "./model-broker.ts";

const binding = {
  credential_id: "credential:project:alpha:v3",
  provider: "openai" as const,
  model: "gpt-test",
  base_url: "https://api.openai.com",
};

test("broker envelope strips provider auth and preserves idempotency", async () => {
  let capturedUrl = "";
  let capturedInit: RequestInit | undefined;
  const result = await performModelBrokerFetch(
    {
      url: "https://api.openai.com/v1/responses",
      headers: [
        ["authorization", `Bearer ${MODEL_AUTH_SENTINEL}`],
        ["content-type", "application/json"],
        ["idempotency-key", "turn-123"],
      ],
      body: { model: "gpt-test", input: "hello" },
    },
    binding,
    { url: "https://home.example/model-egress", token: "broker-token" },
    async (url, init) => {
      capturedUrl = url;
      capturedInit = init;
      return Response.json({
        protocol: MODEL_EGRESS_PROTOCOL,
        status: 200,
        body: { output: [{ type: "message" }] },
        reconciliation_ref: "gateway-request-7",
      });
    },
  );

  assert.equal(capturedUrl, "https://home.example/model-egress");
  const transportHeaders = capturedInit?.headers as Record<string, string>;
  assert.equal(transportHeaders.authorization, "Bearer broker-token");
  assert.equal(transportHeaders["idempotency-key"], "turn-123");
  const envelope = JSON.parse(String(capturedInit?.body));
  assert.equal(envelope.protocol, MODEL_EGRESS_PROTOCOL);
  assert.equal(envelope.credential_ref, binding.credential_id);
  assert.deepEqual(envelope.request.headers, [
    ["content-type", "application/json"],
    ["idempotency-key", "turn-123"],
  ]);
  assert.ok(!JSON.stringify(envelope).includes("broker-token"));
  assert.ok(!JSON.stringify(envelope).includes(MODEL_AUTH_SENTINEL));
  assert.deepEqual(JSON.parse(result), {
    status: 200,
    body: { output: [{ type: "message" }] },
  });
});

test("provider credentials cannot cross the broker boundary", () => {
  assert.throws(
    () => stripSentinelAuthentication([["authorization", "Bearer actual-secret"]]),
    /not the broker sentinel/,
  );
  assert.throws(
    () => stripSentinelAuthentication([["cookie", "session=secret"]]),
    /forbidden cookie header/,
  );
  assert.throws(
    () => stripSentinelAuthentication([["content-type", "application\/json"]]),
    /no broker-sentinel authentication/,
  );
});

test("broker configuration and protocol failures are fail-closed", async () => {
  const request = {
    url: "https://api.openai.com/v1/responses",
    headers: [["authorization", `Bearer ${MODEL_AUTH_SENTINEL}`]] as [string, string][],
    body: {},
  };
  await assert.rejects(
    performModelBrokerFetch(request, binding, { url: "http://broker.example", token: "token" }),
    /must use HTTPS/,
  );
  await assert.rejects(
    performModelBrokerFetch(request, binding, { url: "https://broker.example" }),
    /token is unavailable/,
  );
  await assert.rejects(
    performModelBrokerFetch(
      request,
      binding,
      { url: "http://127.0.0.1:8789/model-egress", token: "token" },
      async () => Response.json({ protocol: "wrong", status: 200, body: {} }),
    ),
    /wrong protocol/,
  );
});
