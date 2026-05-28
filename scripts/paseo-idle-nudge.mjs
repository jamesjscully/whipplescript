#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import { homedir } from "node:os";
import { resolve } from "node:path";

const DEFAULT_AGENT_NAME = "commit and push";
const DEFAULT_CONTINUE_PROMPT = "continue with the implementation plan";
const DEFAULT_POLL_MS = 10_000;

function usage() {
  console.log(`Usage: scripts/paseo-idle-nudge.mjs [options]

Poll a Paseo agent and send a prompt whenever it is idle.

Options:
  --agent-id <id>       Paseo agent id or prefix. Skips name/cwd lookup.
  --agent-name <name>   Agent name to find. Default: "${DEFAULT_AGENT_NAME}"
  --cwd <path>          Agent cwd to match. Default: current directory.
  --message <text>      Message sent when idle. Default: "${DEFAULT_CONTINUE_PROMPT}"
  --poll-ms <ms>        Poll interval. Default: ${DEFAULT_POLL_MS}
  --max-sends <n>       Stop after sending n messages.
  --once                Send at most once, then exit.
  --paseo <path>        Paseo executable. Default: paseo
  --help                Show this help

Example:
  scripts/paseo-idle-nudge.mjs
`);
}

function parseArgs(argv) {
  const args = {
    agentId: null,
    agentName: DEFAULT_AGENT_NAME,
    cwd: process.cwd(),
    message: DEFAULT_CONTINUE_PROMPT,
    pollMs: DEFAULT_POLL_MS,
    maxSends: null,
    once: false,
    paseo: "paseo",
  };

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    const next = () => {
      if (i + 1 >= argv.length) {
        throw new Error(`Missing value for ${arg}`);
      }
      i += 1;
      return argv[i];
    };

    if (arg === "--help" || arg === "-h") {
      usage();
      process.exit(0);
    } else if (arg === "--agent-id") {
      args.agentId = next();
    } else if (arg === "--agent-name") {
      args.agentName = next();
    } else if (arg === "--cwd") {
      args.cwd = next();
    } else if (arg === "--message") {
      args.message = next();
    } else if (arg === "--poll-ms") {
      args.pollMs = Number.parseInt(next(), 10);
    } else if (arg === "--max-sends") {
      args.maxSends = Number.parseInt(next(), 10);
    } else if (arg === "--once") {
      args.once = true;
      args.maxSends = 1;
    } else if (arg === "--paseo") {
      args.paseo = next();
    } else {
      throw new Error(`Unknown option: ${arg}`);
    }
  }

  if (!Number.isFinite(args.pollMs) || args.pollMs < 1_000) {
    throw new Error("--poll-ms must be at least 1000");
  }
  if (args.maxSends !== null && (!Number.isFinite(args.maxSends) || args.maxSends < 0)) {
    throw new Error("--max-sends must be a non-negative integer");
  }

  return args;
}

function log(message) {
  console.log(`[${new Date().toISOString()}] ${message}`);
}

function runPaseo(args, paseo) {
  const result = spawnSync(paseo, args, {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  });

  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    const stderr = result.stderr.trim();
    const stdout = result.stdout.trim();
    throw new Error(`paseo ${args.join(" ")} failed: ${stderr || stdout}`);
  }

  return result.stdout;
}

function normalizePath(path) {
  if (!path) {
    return "";
  }
  const expanded = path === "~" || path.startsWith("~/")
    ? `${homedir()}${path.slice(1)}`
    : path;
  return resolve(expanded);
}

function displayAgent(agent) {
  return `${agent.name} (${agent.shortId ?? agent.id})`;
}

function listAgents(paseo) {
  const stdout = runPaseo(["--json", "ls"], paseo);
  return JSON.parse(stdout);
}

function findAgent(args) {
  if (args.agentId) {
    const agents = listAgents(args.paseo);
    const match = agents.find((agent) => {
      return agent.id === args.agentId || agent.shortId === args.agentId || agent.id.startsWith(args.agentId);
    });
    if (!match) {
      throw new Error(`No active Paseo agent matches id ${args.agentId}`);
    }
    return match;
  }

  const cwd = normalizePath(args.cwd);
  const candidates = listAgents(args.paseo).filter((agent) => {
    return agent.status !== "closed" && agent.name === args.agentName && normalizePath(agent.cwd) === cwd;
  });

  if (candidates.length === 0) {
    throw new Error(`No active Paseo agent named "${args.agentName}" in ${cwd}`);
  }
  if (candidates.length > 1) {
    const ids = candidates.map((agent) => agent.shortId ?? agent.id).join(", ");
    throw new Error(`Multiple matching agents found (${ids}); rerun with --agent-id`);
  }

  return candidates[0];
}

function sendPrompt(agent, args) {
  runPaseo(["send", "--no-wait", agent.id, args.message], args.paseo);
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  let sends = 0;
  let stopping = false;
  let tickInFlight = false;

  const shutdown = () => {
    stopping = true;
  };
  process.on("SIGINT", shutdown);
  process.on("SIGTERM", shutdown);

  log(`polling Paseo agent "${args.agentName}" every ${args.pollMs}ms`);

  async function tick() {
    if (stopping || tickInFlight) {
      return;
    }

    tickInFlight = true;
    try {
      const agent = findAgent(args);
      if (agent.status !== "idle") {
        log(`${displayAgent(agent)} is ${agent.status}; waiting`);
        return;
      }

      if (args.maxSends !== null && sends >= args.maxSends) {
        log(`max sends reached (${args.maxSends}); exiting`);
        stopping = true;
        return;
      }

      sends += 1;
      log(`${displayAgent(agent)} is idle; sending: ${args.message}`);
      sendPrompt(agent, args);

      if (args.once) {
        stopping = true;
      }
    } catch (error) {
      console.error(error.stack ?? error.message);
    } finally {
      tickInFlight = false;
    }
  }

  await tick();
  while (!stopping) {
    await new Promise((resolveTimer) => setTimeout(resolveTimer, args.pollMs));
    await tick();
  }
}

main().catch((error) => {
  console.error(error.stack ?? error.message);
  process.exit(1);
});
