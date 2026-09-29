import assert from "node:assert/strict";
import { test } from "node:test";
import { Codex0149Adapter, CODEX_SOURCE_CAPABILITIES, codexMemoryOffAppServerArgs,
  type CodexAppServerTransport, type CodexMethod } from "./adapter.ts";
import { CodexJsonlDecoder, CodexProtocolError, buildQuestionCardResponse, decodeCodexFrame } from "./protocol.ts";

const bytes = (value: unknown): Uint8Array => new TextEncoder().encode(`${JSON.stringify(value)}\n`);

test("0.149.0 adapter binds resume and steering to the exact thread and active turn", async () => {
  const calls: Array<{ method: CodexMethod; params: Readonly<Record<string, unknown>> }> = [];
  const transport: CodexAppServerTransport = {
    async request(method, params) {
      calls.push({ method, params });
      if (method === "thread/resume") return { thread: { id: "thread-one" } };
      if (method === "turn/steer") return { turnId: "turn-one" };
      return {};
    },
  };
  const adapter = new Codex0149Adapter(transport, "0.149.0");
  assert.deepEqual(await adapter.resume("thread-one"), { status: "resumed", threadId: "thread-one" });
  assert.deepEqual(await adapter.steer("thread-one", "turn-one", "focus"),
    { status: "accepted", turnId: "turn-one" });
  assert.deepEqual(calls, [
    { method: "thread/resume", params: { threadId: "thread-one" } },
    { method: "turn/steer", params: { threadId: "thread-one", expectedTurnId: "turn-one",
      input: [{ type: "text", text: "focus" }] } },
  ]);
});

test("append does not start a turn and compaction ACK is not completion", async () => {
  const calls: Array<{ method: CodexMethod; params: Readonly<Record<string, unknown>> }> = [];
  const adapter = new Codex0149Adapter({ async request(method, params) {
    calls.push({ method, params }); return {};
  } }, "0.149.0");
  assert.deepEqual(await adapter.appendWithoutTurn("thread-one", "later context"),
    { status: "append-acknowledged" });
  assert.deepEqual(await adapter.requestCompaction("thread-one"),
    { status: "requested-not-completed" });
  assert.deepEqual(calls, [
    { method: "thread/inject_items", params: { threadId: "thread-one",
      items: [{ type: "message", role: "user", content: [{ type: "input_text", text: "later context" }] }] } },
    { method: "thread/compact/start", params: { threadId: "thread-one" } },
  ]);
  assert.equal(CODEX_SOURCE_CAPABILITIES.runtimeEvidence, "UNKNOWN");
});

test("version mismatch and changed active turn fail closed", async () => {
  assert.throws(() => new Codex0149Adapter({ async request() { return {}; } }, "0.158.0"),
    (error: unknown) => error instanceof CodexProtocolError && error.code === "VERSION_MISMATCH");
  const adapter = new Codex0149Adapter({ async request() { return { turnId: "other-turn" }; } }, "0.149.0");
  await assert.rejects(adapter.steer("thread-one", "turn-one", "focus"),
    (error: unknown) => error instanceof CodexProtocolError && error.code === "INVALID_RESPONSE");
});

test("native question request remains bound and unanswered", () => {
  // Matches rust-v0.149.0 app-server-protocol ToolRequestUserInputParams.
  const encoded = bytes({ id: 47, method: "item/tool/requestUserInput", params: {
    threadId: "thread-one", turnId: "turn-one", itemId: "item-one", isBlocking: true,
    autoResolutionMs: null, questions: [{ id: "q1", header: "Choice", question: "Which?",
      isOther: true, isSecret: false, options: [{ label: "A", description: "First" }] }],
  } });
  const frame = decodeCodexFrame(encoded.subarray(0, encoded.length - 1));
  assert.equal(frame.kind, "server-request");
  if (frame.kind !== "server-request") throw new Error("wrong frame");
  assert.equal(frame.questionCard?.requestId, 47);
  assert.equal(frame.questionCard?.turnId, "turn-one");
  assert.equal(frame.questionCard?.questions[0]?.id, "q1");
  assert.deepEqual(buildQuestionCardResponse(frame.questionCard!, { q1: ["A"] }).answers.q1?.answers, ["A"]);
  assert.throws(() => buildQuestionCardResponse(frame.questionCard!, { other: ["A"] }),
    (error: unknown) => error instanceof CodexProtocolError && error.code === "INVALID_QUESTION_ANSWER");
});

test("JSONL parser handles split frames and rejects duplicate keys", () => {
  const frames: string[] = [];
  const errors: string[] = [];
  const decoder = new CodexJsonlDecoder((frame) => frames.push(frame.kind), (error) => errors.push(error.code));
  const first = bytes({ method: "turn/started", params: { threadId: "thread-one" } });
  decoder.push(first.subarray(0, 7));
  decoder.push(first.subarray(7));
  decoder.push(new TextEncoder().encode('{"id":1,"id":2,"result":{}}\n'));
  decoder.finish();
  assert.deepEqual(frames, ["notification"]);
  assert.deepEqual(errors, ["DUPLICATE_KEY"]);
});

test("JSONL parser discards an oversized line and recovers at the next newline", () => {
  const frames: string[] = [];
  const errors: string[] = [];
  const decoder = new CodexJsonlDecoder((frame) => frames.push(frame.kind), (error) => errors.push(error.code));
  decoder.push(new Uint8Array(1024 * 1024 + 20).fill(65));
  decoder.push(new TextEncoder().encode('\n{"method":"turn/started","params":{}}\n'));
  decoder.finish();
  assert.deepEqual(errors, ["OVERSIZE_FRAME"]);
  assert.deepEqual(frames, ["notification"]);
});

test("native question accepts fields optional in pinned schema and defaults flags false", () => {
  const encoded = bytes({ id: "ask-1", method: "item/tool/requestUserInput", params: {
    threadId: "thread-one", turnId: "turn-one", itemId: "item-one",
    questions: [{ id: "q1", header: "Choice", question: "Which?" }],
  } });
  const parsed = decodeCodexFrame(encoded.subarray(0, encoded.length - 1));
  assert.equal(parsed.kind, "server-request");
  if (parsed.kind !== "server-request") throw new Error("wrong frame");
  assert.equal(parsed.questionCard?.questions[0]?.isOther, false);
  assert.equal(parsed.questionCard?.questions[0]?.isSecret, false);
  assert.equal(parsed.questionCard?.questions[0]?.options, null);
  assert.equal(parsed.questionCard?.autoResolutionMs, null);
});

test("launch args disable both memory generation and use at the pinned version", () => {
  assert.deepEqual(codexMemoryOffAppServerArgs(), [
    "-c", "features.memories=false", "-c", "memories.generate_memories=false",
    "-c", "memories.use_memories=false", "app-server",
  ]);
});
