import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const processUrl = new URL("./controlled-pi.mjs", import.meta.url);
const sourceUrl = new URL("../sealing/model-asset.json", import.meta.url);
const content = readFileSync(sourceUrl, "utf8");
const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");
const task = {
  schema: "gogoke.s1-r4.r2-02.fixture-task.v1",
  testOnly: true,
  source: {
    repository: "taiyun668/gogoke",
    commit: "f6a820dda05a3eac5c29be48c4149bff7e1c9598",
    path: "apps/desktop/test-fixtures/s1-r4/sealing/model-asset.json",
    sha256: sha256(Buffer.from(content)),
    content,
  },
};

function run(value) {
  const result = spawnSync(process.execPath, [fileURLToPath(processUrl)], {
    input: `${JSON.stringify({ id: "r2-02-test-1", type: "prompt", message: JSON.stringify(value) })}\n`,
    encoding: "utf8",
    timeout: 5_000,
  });
  assert.equal(result.error, undefined);
  return { status: result.status, frames: result.stdout.trim().split("\n").map(JSON.parse) };
}

test("test-only process reports derived fixture bytes after Pi ACK and settlement", () => {
  const { status, frames } = run(task);
  assert.equal(status, 0);
  assert.deepEqual(frames.map((frame) => frame.type),
    ["response", "agent_start", "message_end", "agent_end", "agent_settled"]);
  assert.equal(frames[0].success, true);
  const report = JSON.parse(frames[2].message.content[0].text);
  const source = JSON.parse(content);
  assert.equal(report.testOnly, true);
  assert.equal(report.source.sha256, task.source.sha256);
  assert.equal(report.observed.modelId, source.modelId);
  assert.equal(report.observed.relativePath, source.relativePath);
  assert.equal(report.observed.embeddedBytesSha256, sha256(Buffer.from(source.bytesBase64, "base64")));
});

test("source hash mismatch rejects before agent work", () => {
  const { status, frames } = run({ ...task, source: { ...task.source, sha256: "0".repeat(64) } });
  assert.equal(status, 1);
  assert.deepEqual(frames.map((frame) => frame.type), ["response"]);
  assert.equal(frames[0].success, false);
});

test("rejected task exits with stdin still open", async () => {
  const child = spawn(process.execPath, [fileURLToPath(processUrl)], { stdio: ["pipe", "pipe", "pipe"] });
  let output = "";
  child.stdout.setEncoding("utf8").on("data", (chunk) => { output += chunk; });
  const exited = new Promise((resolve, reject) => {
    child.once("error", reject);
    child.once("exit", (code) => resolve(code));
  });
  child.stdin.write(`${JSON.stringify({ id: "bad", type: "prompt", message: JSON.stringify({
    ...task, source: { ...task.source, sha256: "0".repeat(64) },
  }) })}\n`);
  const timer = setTimeout(() => child.kill(), 2_000);
  try {
    assert.equal(await exited, 1);
    assert.equal(JSON.parse(output.trim()).success, false);
  } finally {
    clearTimeout(timer);
    child.stdin.destroy();
  }
});
