import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { ArmatureSdkError, getEvent, getPayload, getRunContext, readJson, writeJson } from "./index.js";

test("getRunContext reads known environment variables", () => {
  process.env.ARMATURE_RUN_ID = "run_01ARZ3NDEKTSV4RRFFQ69G5FAV";
  process.env.ARMATURE_TASK_NAME = "status";

  const context = getRunContext();

  assert.equal(context.runId, "run_01ARZ3NDEKTSV4RRFFQ69G5FAV");
  assert.equal(context.taskName, "status");
});

test("getEvent and getPayload parse the embedded event envelope", () => {
  process.env.ARMATURE_EVENT = JSON.stringify({
    type: "tool.run.completed",
    payload: { ok: true }
  });

  assert.equal(getEvent<{ ok: boolean }>().type, "tool.run.completed");
  assert.deepEqual(getPayload<{ ok: boolean }>(), { ok: true });
});

test("getEvent raises a typed error when the event is absent", () => {
  delete process.env.ARMATURE_EVENT;

  assert.throws(() => getEvent(), (error: unknown) => {
    assert.ok(error instanceof ArmatureSdkError);
    assert.equal(error.kind, "missing_env");
    return true;
  });
});

test("writeJson emits stable formatted output", async () => {
  const directory = await mkdtemp(join(tmpdir(), "armature-sdk-"));
  const path = join(directory, "payload.json");

  await writeJson(path, { ok: true, count: 1 });

  assert.equal(await readFile(path, "utf8"), '{\n  "ok": true,\n  "count": 1\n}\n');
  assert.deepEqual(await readJson(path), { ok: true, count: 1 });
});

