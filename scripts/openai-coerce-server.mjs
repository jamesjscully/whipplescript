#!/usr/bin/env node
import http from "node:http";
import fs from "node:fs";
import path from "node:path";

const root = path.resolve(import.meta.dirname, "..");
loadDotenv(path.join(root, ".env"));

const host = process.env.WHIPPLESCRIPT_OPENAI_COERCE_HOST || "127.0.0.1";
const port = Number(process.env.WHIPPLESCRIPT_OPENAI_COERCE_PORT || "18765");
const model = process.env.WHIPPLESCRIPT_OPENAI_MODEL || "gpt-5.4-mini";
const apiKey = process.env.OPENAI_API_KEY;
const authToken = process.env.WHIPPLESCRIPT_OPENAI_COERCE_TOKEN;

if (!apiKey) {
  console.error("OPENAI_API_KEY is required for the OpenAI coerce server");
  process.exit(2);
}

if (!authToken || authToken.length < 16) {
  console.error("WHIPPLESCRIPT_OPENAI_COERCE_TOKEN with at least 16 characters is required");
  process.exit(2);
}

const server = http.createServer(async (request, response) => {
  try {
    if (request.method === "GET" && request.url === "/health") {
      writeJson(response, 200, { status: "ok", provider: "openai", model });
      return;
    }

    if (request.method !== "POST" || request.url !== "/coerce") {
      writeJson(response, 404, { error: "not_found" });
      return;
    }

    if (!isAuthorized(request)) {
      writeJson(response, 401, { error: "unauthorized" });
      return;
    }

    const body = await readJson(request);
    const result = await coerce(body);
    writeJson(response, 200, result);
  } catch (error) {
    writeJson(response, 500, {
      status: "failed",
      error: {
        code: "openai_coerce_failed",
        message: String(error?.message || error),
        recoverable: true,
      },
      summary: "OpenAI coerce bridge failed",
      usage: {},
    });
  }
});

server.listen(port, host, () => {
  console.error(`OpenAI coerce server listening on http://${host}:${port}`);
});

process.on("SIGTERM", () => server.close(() => process.exit(0)));
process.on("SIGINT", () => server.close(() => process.exit(130)));

function isAuthorized(request) {
  return request.headers.authorization === `Bearer ${authToken}`;
}

async function coerce(request) {
  const outputType = String(request.output_type || "CoerceResult");
  const schema = responseSchema(outputType);
  const prompt = [
    "Run this WhippleScript coerce function and return only the structured output.",
    "Use the function name, output type, and arguments as the source of truth.",
    "Choose conservative confidence values when the input is sparse.",
    JSON.stringify(
      {
        function_name: request.function_name,
        output_type: outputType,
        arguments: request.arguments,
      },
      null,
      2,
    ),
  ].join("\n\n");

  const apiResponse = await fetch("https://api.openai.com/v1/responses", {
    method: "POST",
    headers: {
      "Authorization": `Bearer ${apiKey}`,
      "Content-Type": "application/json",
    },
    body: JSON.stringify({
      model,
      input: [
        {
          role: "system",
          content:
            "You are a deterministic data coercion provider for WhippleScript. Return JSON that exactly matches the requested schema.",
        },
        { role: "user", content: prompt },
      ],
      text: {
        format: {
          type: "json_schema",
          name: safeSchemaName(outputType),
          schema,
          strict: true,
        },
      },
      max_output_tokens: 512,
    }),
  });

  const apiBody = await apiResponse.json().catch(async () => ({
    error: { message: await apiResponse.text() },
  }));

  if (!apiResponse.ok) {
    return {
      status: "failed",
      error: {
        code: "openai_api_error",
        message: apiBody?.error?.message || `OpenAI API returned ${apiResponse.status}`,
        recoverable: apiResponse.status >= 500 || apiResponse.status === 429,
        details: { status_code: apiResponse.status },
      },
      summary: `OpenAI API returned ${apiResponse.status}`,
      transcript: `openai responses model=${model}`,
      usage: apiBody?.usage || {},
    };
  }

  const outputText = extractOutputText(apiBody);
  const value = JSON.parse(outputText);
  return {
    status: "succeeded",
    value,
    summary: "OpenAI coerce succeeded",
    transcript: `openai responses model=${model} response=${apiBody.id || "unknown"}`,
    usage: apiBody.usage || {},
  };
}

function responseSchema(outputType) {
  if (process.env.WHIPPLESCRIPT_OPENAI_COERCE_SCHEMA_JSON) {
    return JSON.parse(process.env.WHIPPLESCRIPT_OPENAI_COERCE_SCHEMA_JSON);
  }

  if (outputType === "MessageClassification") {
    return {
      type: "object",
      additionalProperties: false,
      required: ["priority", "summary", "confidence"],
      properties: {
        priority: { type: "string", enum: ["Low", "Normal", "Urgent"] },
        summary: { type: "string" },
        confidence: { type: "number" },
      },
    };
  }

  if (outputType === "WorkReview") {
    return {
      type: "object",
      additionalProperties: false,
      required: ["status", "reason", "followups", "confidence"],
      properties: {
        status: { type: "string", enum: ["Accept", "Revise", "Blocked"] },
        reason: { type: "string" },
        followups: { type: "array", items: { type: "string" } },
        confidence: { type: "number" },
      },
    };
  }

  return {
    type: "object",
    additionalProperties: false,
    required: ["result"],
    properties: {
      result: { type: "string" },
    },
  };
}

function extractOutputText(body) {
  if (typeof body.output_text === "string") {
    return body.output_text;
  }

  for (const item of body.output || []) {
    for (const content of item.content || []) {
      if (content.type === "output_text" && typeof content.text === "string") {
        return content.text;
      }
    }
  }

  throw new Error("OpenAI response did not include output text");
}

function safeSchemaName(value) {
  return String(value || "CoerceResult")
    .replace(/[^A-Za-z0-9_-]/g, "_")
    .slice(0, 64);
}

function readJson(request) {
  return new Promise((resolve, reject) => {
    let body = "";
    request.setEncoding("utf8");
    request.on("data", (chunk) => {
      body += chunk;
      if (body.length > 1_000_000) {
        reject(new Error("request body too large"));
        request.destroy();
      }
    });
    request.on("end", () => {
      try {
        resolve(JSON.parse(body || "{}"));
      } catch (error) {
        reject(error);
      }
    });
    request.on("error", reject);
  });
}

function writeJson(response, status, body) {
  const text = JSON.stringify(body);
  response.writeHead(status, {
    "Content-Type": "application/json",
    "Content-Length": Buffer.byteLength(text),
  });
  response.end(text);
}

function loadDotenv(file) {
  if (!fs.existsSync(file)) {
    return;
  }

  const lines = fs.readFileSync(file, "utf8").split(/\r?\n/);
  for (const line of lines) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith("#")) {
      continue;
    }
    const equals = trimmed.indexOf("=");
    if (equals <= 0) {
      continue;
    }
    const key = trimmed.slice(0, equals).trim();
    let value = trimmed.slice(equals + 1).trim();
    if (
      (value.startsWith('"') && value.endsWith('"')) ||
      (value.startsWith("'") && value.endsWith("'"))
    ) {
      value = value.slice(1, -1);
    }
    if (!process.env[key]) {
      process.env[key] = value;
    }
  }
}
