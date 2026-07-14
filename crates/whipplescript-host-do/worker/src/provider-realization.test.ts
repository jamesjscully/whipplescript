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

test("static bindings remain only as exact explicit worker-secret transitions", () => {
  const configured = {
    [admission.provider_binding_id]: {
      credential_id: admission.credential_id,
      provider: admission.provider,
      model: admission.model,
      base_url: admission.base_url,
      execution: "worker-secret",
      secret: "OPENAI_API_KEY",
    },
  };
  const resolved = resolveAdmittedProvider(admission, {
    WHIP_HOST_PROVIDER_BINDINGS_JSON: JSON.stringify(configured),
    OPENAI_API_KEY: "transitional-secret",
  });
  assert.equal(resolved.execution, "worker-secret");
  assert.equal(resolved.api_key, "transitional-secret");

  configured[admission.provider_binding_id].model = "tampered-model";
  assert.throws(
    () => resolveAdmittedProvider(admission, {
      WHIP_HOST_PROVIDER_BINDINGS_JSON: JSON.stringify(configured),
      OPENAI_API_KEY: "transitional-secret",
    }),
    /no exact hosted realization/,
  );
});

test("legacy static broker declarations and missing broker transport fail closed", () => {
  const legacy = {
    [admission.provider_binding_id]: {
      credential_id: admission.credential_id,
      provider: admission.provider,
      model: admission.model,
      base_url: admission.base_url,
      execution: "model-broker",
    },
  };
  assert.throws(
    () => resolveAdmittedProvider(admission, {
      WHIP_HOST_PROVIDER_BINDINGS_JSON: JSON.stringify(legacy),
      WHIP_MODEL_BROKER_URL: "https://home.example/internal/model-egress",
      WHIP_MODEL_BROKER_TOKEN: "hop-token",
    }),
    /static model-broker realizations are retired/,
  );
  assert.throws(
    () => resolveAdmittedProvider(admission, {}),
    /has no model broker/,
  );
});
