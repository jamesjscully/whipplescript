export const MODEL_EGRESS_PROTOCOL = "whipplescript.model-egress.v1";
export const MODEL_AUTH_SENTINEL = "whipplescript-model-broker";

const MAX_BROKER_RESPONSE_BYTES = 16 * 1024 * 1024;
const STRIPPED_AUTH_HEADERS = new Set([
  "authorization",
  "chatgpt-account-id",
  "x-api-key",
]);
const FORBIDDEN_AMBIENT_AUTH_HEADERS = new Set([
  "cookie",
  "proxy-authorization",
]);

export interface ModelBrokerBinding {
  credential_id: string;
  provider: "openai" | "openai-generic" | "anthropic" | "openai-codex";
  model: string;
  base_url: string;
}

export interface ModelBrokerConfig {
  url?: string;
  token?: string;
}

export interface SuspendedModelRequest {
  url: string;
  headers: [string, string][];
  body: unknown;
}

type FetchLike = (input: string, init: RequestInit) => Promise<Response>;

interface BrokerResponse {
  protocol: typeof MODEL_EGRESS_PROTOCOL;
  status: number;
  body: unknown;
  reconciliation_ref?: string;
}

function validatedBrokerUrl(raw: string | undefined): string {
  if (!raw?.trim()) throw new Error("model broker URL is unavailable");
  let url: URL;
  try {
    url = new URL(raw);
  } catch {
    throw new Error("model broker URL is invalid");
  }
  const loopback = url.hostname === "localhost"
    || url.hostname === "127.0.0.1"
    || url.hostname === "[::1]"
    || url.hostname === "::1";
  if (url.protocol !== "https:" && !(url.protocol === "http:" && loopback)) {
    throw new Error("model broker URL must use HTTPS (HTTP is loopback-only)");
  }
  if (url.username || url.password || url.hash) {
    throw new Error("model broker URL may not contain credentials or a fragment");
  }
  return url.toString();
}

function sentinelValue(name: string): string {
  return name === "authorization"
    ? `Bearer ${MODEL_AUTH_SENTINEL}`
    : MODEL_AUTH_SENTINEL;
}

export function stripSentinelAuthentication(
  headers: [string, string][],
): [string, string][] {
  const sanitized: [string, string][] = [];
  let witnessedAuthentication = false;
  for (const [name, value] of headers) {
    const normalized = name.toLowerCase();
    if (FORBIDDEN_AMBIENT_AUTH_HEADERS.has(normalized)) {
      throw new Error(`model request contains forbidden ${normalized} header`);
    }
    if (STRIPPED_AUTH_HEADERS.has(normalized)) {
      if (value !== sentinelValue(normalized)) {
        throw new Error(`model request ${normalized} header is not the broker sentinel`);
      }
      witnessedAuthentication = true;
      continue;
    }
    sanitized.push([name, value]);
  }
  if (!witnessedAuthentication) {
    throw new Error("model request has no broker-sentinel authentication header");
  }
  return sanitized;
}

async function readJsonCapped(response: Response): Promise<unknown> {
  const declared = response.headers.get("content-length");
  if (declared && Number(declared) > MAX_BROKER_RESPONSE_BYTES) {
    throw new Error("model broker response exceeds the size cap");
  }
  if (!response.body) throw new Error("model broker response had no body");
  const reader = response.body.getReader();
  const chunks: Uint8Array[] = [];
  let total = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    if (!value) continue;
    total += value.byteLength;
    if (total > MAX_BROKER_RESPONSE_BYTES) {
      await reader.cancel();
      throw new Error("model broker response exceeds the size cap");
    }
    chunks.push(value);
  }
  const bytes = new Uint8Array(total);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.byteLength;
  }
  try {
    return JSON.parse(new TextDecoder().decode(bytes));
  } catch {
    throw new Error("model broker response was not valid JSON");
  }
}

function validatedBrokerResponse(value: unknown): BrokerResponse {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error("model broker response must be an object");
  }
  const response = value as Partial<BrokerResponse>;
  if (response.protocol !== MODEL_EGRESS_PROTOCOL) {
    throw new Error("model broker response has the wrong protocol");
  }
  if (!Number.isInteger(response.status) || Number(response.status) < 100 || Number(response.status) > 599) {
    throw new Error("model broker response has an invalid provider status");
  }
  if (!("body" in response)) {
    throw new Error("model broker response has no provider body");
  }
  if (response.reconciliation_ref !== undefined && typeof response.reconciliation_ref !== "string") {
    throw new Error("model broker reconciliation ref must be a string");
  }
  return response as BrokerResponse;
}

export async function performModelBrokerFetch(
  request: SuspendedModelRequest,
  binding: ModelBrokerBinding,
  config: ModelBrokerConfig,
  fetcher: FetchLike = fetch,
): Promise<string> {
  const brokerUrl = validatedBrokerUrl(config.url);
  const token = config.token?.trim();
  if (!token) throw new Error("model broker token is unavailable");
  if (!binding.credential_id.trim()) throw new Error("model broker credential ref is empty");

  const headers = stripSentinelAuthentication(request.headers);
  const idempotencyKey = headers.find(
    ([name]) => name.toLowerCase() === "idempotency-key",
  )?.[1];
  const envelope = {
    protocol: MODEL_EGRESS_PROTOCOL,
    credential_ref: binding.credential_id,
    provider: binding.provider,
    request: {
      url: request.url,
      headers,
      body: request.body,
    },
  };
  const brokerHeaders: Record<string, string> = {
    authorization: `Bearer ${token}`,
    "content-type": "application/json",
  };
  if (idempotencyKey) brokerHeaders["idempotency-key"] = idempotencyKey;
  const response = await fetcher(brokerUrl, {
    method: "POST",
    headers: brokerHeaders,
    body: JSON.stringify(envelope),
  });
  if (!response.ok) {
    throw new Error(`model broker returned HTTP ${response.status}`);
  }
  const decoded = validatedBrokerResponse(await readJsonCapped(response));
  return JSON.stringify({ status: decoded.status, body: decoded.body });
}
