import { MODEL_AUTH_SENTINEL } from "./model-broker.ts";

export type HostedProvider =
  | "openai"
  | "openai-generic"
  | "anthropic"
  | "openai-codex";

export interface HostTurnAdmission {
  provider_binding_id: string;
  credential_id: string;
  placement_ceiling_ref: string;
  provider: HostedProvider;
  model: string;
  base_url: string;
}

export interface StaticHostProviderBinding {
  credential_id: string;
  provider: HostedProvider;
  model: string;
  base_url: string;
  execution: "worker-secret" | "model-broker";
  secret?: "OPENAI_API_KEY" | "ANTHROPIC_API_KEY";
}

export interface ResolvedHostProviderBinding {
  credential_id: string;
  provider: HostedProvider;
  model: string;
  base_url: string;
  execution: "worker-secret" | "model-broker";
  api_key: string;
  secret?: "OPENAI_API_KEY" | "ANTHROPIC_API_KEY";
}

export interface ProviderRealizationEnv {
  ANTHROPIC_API_KEY?: string;
  OPENAI_API_KEY?: string;
  WHIP_HOST_PROVIDER_BINDINGS_JSON?: string;
  WHIP_MODEL_BROKER_URL?: string;
  WHIP_MODEL_BROKER_TOKEN?: string;
}

const supportedProviders = new Set<HostedProvider>([
  "openai",
  "openai-generic",
  "anthropic",
  "openai-codex",
]);

function staticBindings(raw: string | undefined): Record<string, StaticHostProviderBinding> {
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw ?? "{}");
  } catch (error) {
    throw new Error(`invalid WHIP_HOST_PROVIDER_BINDINGS_JSON: ${String(error)}`);
  }
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
    throw new Error("invalid WHIP_HOST_PROVIDER_BINDINGS_JSON: binding map must be an object");
  }
  return parsed as Record<string, StaticHostProviderBinding>;
}

function validateAdmission(admission: HostTurnAdmission): void {
  if (
    !admission.provider_binding_id.trim()
    || !admission.credential_id.trim()
    || !admission.placement_ceiling_ref.trim()
    || !supportedProviders.has(admission.provider)
    || !admission.model.trim()
    || !admission.base_url.trim()
  ) {
    throw new Error("admitted provider capability has no exact hosted realization");
  }
}

function exactStaticBinding(
  admission: HostTurnAdmission,
  binding: StaticHostProviderBinding,
): boolean {
  return binding.credential_id === admission.credential_id
    && binding.provider === admission.provider
    && binding.model === admission.model
    && binding.base_url === admission.base_url;
}

/**
 * Realize a provider only after Rust has returned the signed-policy tuple.
 * Brokered egress is dynamic and needs no provider map; the deployment map is
 * retained solely as the explicit transitional Worker-secret escape hatch.
 */
export function resolveAdmittedProvider(
  admission: HostTurnAdmission,
  env: ProviderRealizationEnv,
): ResolvedHostProviderBinding {
  validateAdmission(admission);
  const configured = staticBindings(env.WHIP_HOST_PROVIDER_BINDINGS_JSON)[
    admission.provider_binding_id
  ];

  if (configured) {
    if (!exactStaticBinding(admission, configured)) {
      throw new Error("admitted provider capability has no exact hosted realization");
    }
    if (configured.execution === "model-broker") {
      throw new Error(
        "static model-broker realizations are retired; broker identity comes from signed policy",
      );
    }
    if (
      configured.execution !== "worker-secret"
      || (configured.secret !== "OPENAI_API_KEY" && configured.secret !== "ANTHROPIC_API_KEY")
    ) {
      throw new Error("admitted provider capability has an invalid hosted realization");
    }
    if (configured.provider === "openai-codex") {
      throw new Error("openai-codex requires an explicitly brokered non-bash turn capability");
    }
    const secret = env[configured.secret];
    if (typeof secret !== "string" || !secret.trim()) {
      throw new Error(`admitted provider credential ${configured.credential_id} is unavailable`);
    }
    return { ...configured, execution: "worker-secret", api_key: secret };
  }

  if (!env.WHIP_MODEL_BROKER_URL?.trim() || !env.WHIP_MODEL_BROKER_TOKEN?.trim()) {
    throw new Error(`admitted provider credential ${admission.credential_id} has no model broker`);
  }
  return {
    credential_id: admission.credential_id,
    provider: admission.provider,
    model: admission.model,
    base_url: admission.base_url,
    execution: "model-broker",
    api_key: MODEL_AUTH_SENTINEL,
  };
}
