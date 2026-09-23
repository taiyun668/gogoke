// Test-only Pi JSONL process. It produces a deterministic report from supplied public fixture bytes.
import { createHash } from "node:crypto";

const sourcePath = "apps/desktop/test-fixtures/s1-r4/sealing/model-asset.json";
const digest = (bytes) => createHash("sha256").update(bytes).digest("hex");
const send = (frame) => process.stdout.write(`${JSON.stringify(frame)}\n`);
let input = Buffer.alloc(0);
let handled = false;

function handle(line) {
  let command;
  try {
    command = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(line));
    if (command?.type !== "prompt" || typeof command.id !== "string") throw new Error();
    const task = JSON.parse(command.message);
    const source = task.source;
    if (task.schema !== "gogoke.s1-r4.r2-02.fixture-task.v1" || task.testOnly !== true ||
        source?.repository !== "taiyun668/gogoke" || source.path !== sourcePath ||
        !/^[0-9a-f]{40}$/.test(source.commit) || !/^[0-9a-f]{64}$/.test(source.sha256) ||
        typeof source.content !== "string" || digest(Buffer.from(source.content)) !== source.sha256) {
      throw new Error();
    }
    const material = JSON.parse(source.content);
    if (typeof material.modelId !== "string" || typeof material.relativePath !== "string" ||
        typeof material.bytesBase64 !== "string") throw new Error();
    const report = {
      schema: "gogoke.s1-r4.r2-02.fixture-report.v1",
      testOnly: true,
      source: { repository: source.repository, commit: source.commit, path: source.path,
        sha256: source.sha256 },
      observed: { modelId: material.modelId, relativePath: material.relativePath,
        embeddedBytesSha256: digest(Buffer.from(material.bytesBase64, "base64")) },
    };
    const message = { role: "assistant", content: [{ type: "text", text: JSON.stringify(report) }],
      api: "gogoke-test-protocol", provider: "gogoke-test-only", model: "deterministic-fixture",
      stopReason: "stop", timestamp: Date.now() };
    send({ type: "response", id: command.id, command: "prompt", success: true });
    send({ type: "agent_start" });
    send({ type: "message_end", message });
    send({ type: "agent_end", messages: [message], willRetry: false });
    send({ type: "agent_settled" });
  } catch {
    if (command?.id && typeof command.id === "string") {
      send({ type: "response", id: command.id, command: "prompt", success: false,
        error: "INVALID_TEST_ONLY_TASK" });
    }
    process.exitCode = 1;
  }
}

process.stdin.on("data", (chunk) => {
  if (handled) return;
  input = Buffer.concat([input, chunk]);
  if (input.length > 64 * 1024) { handled = true; process.exitCode = 1; return; }
  const end = input.indexOf(0x0a);
  if (end < 0) return;
  handled = true;
  handle(input.subarray(0, end));
});
process.stdin.on("end", () => { if (!handled) process.exitCode = 1; });
