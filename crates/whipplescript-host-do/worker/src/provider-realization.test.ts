import assert from "node:assert/strict";
import test from "node:test";
import { MODEL_AUTH_SENTINEL } from "./model-broker.ts";
import {
  type HostTurnAdmission,
  resolveAdmittedProvider,
} from "./provider-realization.ts";

const admission: HostTurnAdmission = {
  provider_binding_id: "gaugedesk:provider:primary",
  credential_id: "gaugedesk:credential:v2:account:616c696365:openai:4",
  placement_ceiling_ref: "gaugedesk:placement:do",
  provider: "openai",
  model: "gpt-5",
  base_url: "https://api.openai.com/v1/responses",
};

test("signed admission dynamically realizes the model broker without a provider map", () => {
  const resolved = resolveAdmittedProvider(admission, {
    WHIP_MODEL_BROKER_URL: "https://home.example/internal/model-egress",
    WHIP_MODEL_BROKER_TOKEN: "hop-token",
  });
  assert.deepEqual(resolved, {
    credential_id: admission.credential_id,
    provider: "openai",
    model: "gpt-5",
    base_url: "https://api.openai.com/v1/responses",
    execution: "model-broker",
    api_key: MODEL_AUTH_SENTINEL,
  });
});

test("codex admission uses only the authenticated local broker sentinel", () => {
  const codex: HostTurnAdmission = {
    ...admission,
    credential_id: "gaugedesk:credential:v2:account:616c696365:openai-codex:1",
    provider: "openai-codex",
    model: "gpt-5.5",
    base_url: "https://chatgpt.com",
  };
  const resolved = resolveAdmittedProvider(codex, {
    WHIP_MODEL_BROKER_URL: "https://outbound-session.example/internal/local-model-egress",
    WHIP_MODEL_BROKER_TOKEN: "session-token",
  });
  assert.equal(resolved.execution, "model-broker");
  assert.equal(resolved.api_key, MODEL_AUTH_SENTINEL);
  assert.equal(resolved.credential_id, codex.credential_id);
});

test("missing broker transport fails closed with no Worker-secret fallback", () => {
  assert.throws(
    () => resolveAdmittedProvider(admission, {}),
    /has no model broker/,
  );
});
