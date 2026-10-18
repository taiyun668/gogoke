import assert from "node:assert/strict";
import { test } from "node:test";
import { Codex0160Session, type CodexSessionTransport } from "./session.ts";
import { CodexProtocolError, decodeCodexFrame } from "./protocol.ts";

const frame = (value: unknown) => decodeCodexFrame(new TextEncoder().encode(JSON.stringify(value)));
const memoryOffConfig = { config: { features: { memories: false },
  memories: { generate_memories: false, use_memories: false } } };

test("0.160.0 session initializes, starts a thread and binds a turn before steering", async () => {
  const calls: Array<{ method: string; params: object }> = [];
  const transport: CodexSessionTransport = {
    async request(method, params) {
      calls.push({ method, params });
      if (method === "initialize") return { userAgent: "codex" };
      if (method === "config/read") return memoryOffConfig;
      if (method === "thread/start") return { thread: { id: "thread-a", cwd: "D:/isolated-instance" } };
      if (method === "turn/start") return { turn: { id: "turn-a", status: "inProgress" } };
      if (method === "turn/steer") return { turnId: "turn-a" };
      return {};
    },
    async notify(method) { calls.push({ method, params: {} }); },
  };
  const session = new Codex0160Session(transport, "0.160.0");
  assert.deepEqual(session.capabilityReport(), { pinnedVersion: "0.160.0",
    runtimeEvidence: "UNKNOWN", observedMethods: [] });
  await session.initialize("test");
  await session.verifyMemoryOff("D:/isolated-instance");
  assert.equal(await session.startThread("D:/isolated-instance"), "thread-a");
  assert.equal(await session.startTurn("hello"), "turn-a");
  await session.steer("turn-a", "focus");
  assert.deepEqual(calls.map((call) => call.method), ["initialize", "initialized", "config/read", "thread/start", "turn/start", "turn/steer"]);
  assert.deepEqual(calls[5]?.params, { threadId: "thread-a", expectedTurnId: "turn-a",
    input: [{ type: "text", text: "focus" }] });
  assert.equal(session.turnStatus, "running");
  assert.equal(session.capabilityReport().runtimeEvidence, "METHOD_RESPONSES_ONLY");
});

test("capability discovery counts actual returned pages and keeps pagination visible", async () => {
  const calls: string[] = [];
  const session = new Codex0160Session({
    async request(method) {
      calls.push(method);
      if (method === "model/list") return { data: [{ id: "m" }], nextCursor: "next-models" };
      if (method === "experimentalFeature/list") return { data: [], nextCursor: null };
      return {};
    }, async notify() {},
  }, "0.160.0");
  await session.initialize("test");
  assert.deepEqual(await session.discover(), { modelPageCount: 1, modelNextCursor: "next-models",
    featurePageCount: 0, featureNextCursor: null });
  assert.deepEqual(calls, ["initialize", "model/list", "experimentalFeature/list"]);
  assert.deepEqual(session.capabilityReport().observedMethods,
    ["experimentalFeature/list", "initialize", "model/list"]);
});

test("one initialized session verifies effective memory-off and redacts local account details", async () => {
  const calls: Array<{ method: string; params: object }> = [];
  let account: unknown = { account: null, requiresOpenaiAuth: true };
  const session = new Codex0160Session({
    async request(method, params) {
      calls.push({ method, params });
      if (method === "config/read") return { config: {
        features: { memories: false },
        memories: { generate_memories: false, use_memories: false },
      } };
      if (method === "account/read") return account;
      return {};
    }, async notify() {},
  }, "0.160.0");
  await session.initialize("test");
  await session.verifyMemoryOff("D:/isolated-instance");
  await assert.rejects(session.startThread("D:/other-project"), /MEMORY_NOT_VERIFIED/);
  assert.equal(await session.observeLocalAccount(), "LOGGED_OUT");
  account = { account: { type: "chatgpt", email: "private@example.invalid", planType: "plus" },
    requiresOpenaiAuth: true };
  const observation = await session.observeLocalAccount();
  assert.equal(observation, "CREDENTIAL_PRESENT");
  assert.equal(JSON.stringify(observation).includes("private@example.invalid"), false);
  assert.deepEqual(calls.filter((call) => call.method === "account/read").map((call) => call.params), [{}, {}]);
  assert.deepEqual(calls.find((call) => call.method === "config/read")?.params,
    { cwd: "D:/isolated-instance", includeLayers: true });
});

test("memory-on effective config cannot precede a Codex thread", async () => {
  const session = new Codex0160Session({
    async request(method) {
      if (method === "config/read") return { config: {
        features: { memories: true },
        memories: { generate_memories: false, use_memories: false },
      } };
      return {};
    }, async notify() {},
  }, "0.160.0");
  await session.initialize("test");
  await assert.rejects(session.verifyMemoryOff("D:/isolated-instance"), /MEMORY_NOT_DISABLED/);
  assert.equal(session.phase, "recovery-required");
  await assert.rejects(session.startThread("D:/project"), CodexProtocolError);
});

test("resumed Codex thread must report the verified working directory", async () => {
  const session = new Codex0160Session({
    async request(method) {
      if (method === "config/read") return memoryOffConfig;
      if (method === "thread/resume") return { thread: { id: "thread-a", cwd: "D:/other-project" } };
      return {};
    }, async notify() {},
  }, "0.160.0");
  await session.initialize("test");
  await session.verifyMemoryOff("D:/isolated-instance");
  await assert.rejects(session.resume("thread-a", "D:/isolated-instance"), /different cwd/);
  assert.equal(session.phase, "recovery-required");
});

test("new Codex thread must report the verified working directory", async () => {
  const session = new Codex0160Session({
    async request(method) {
      if (method === "config/read") return memoryOffConfig;
      if (method === "thread/start") return { thread: { id: "thread-a", cwd: "D:/other-project" } };
      return {};
    }, async notify() {},
  }, "0.160.0");
  await session.initialize("test");
  await session.verifyMemoryOff("D:/isolated-instance");
  await assert.rejects(session.startThread("D:/isolated-instance"), /different cwd/);
  assert.equal(session.phase, "recovery-required");
});

test("native question uses exact server request ID and terminal event closes the turn", async () => {
  const session = new Codex0160Session({
    async request(method) {
      if (method === "config/read") return memoryOffConfig;
      if (method === "thread/resume") return { thread: { id: "thread-a", cwd: "D:/isolated-instance" } };
      if (method === "turn/start") return { turn: { id: "turn-a", status: "inProgress" } };
      return {};
    },
    async notify() {},
  }, "0.160.0");
  await session.initialize("test");
  await session.verifyMemoryOff("D:/isolated-instance");
  await session.resume("thread-a", "D:/isolated-instance");
  await session.startTurn("question");
  const question = session.observe(frame({ id: 44, method: "item/tool/requestUserInput", params: {
    threadId: "thread-a", turnId: "turn-a", itemId: "item-a",
    questions: [{ id: "q", header: "Choose", question: "Which?", options: [{ label: "A", description: "First" }] }],
  } }));
  assert.equal(question?.kind, "question");
  assert.equal(session.pendingQuestion?.questions[0]?.isSecret, false);
  assert.equal(JSON.stringify(session.answerQuestion(44, { q: ["A"] })),
    JSON.stringify({ id: 44, result: { answers: { q: { answers: ["A"] } } } }));
  assert.throws(() => session.answerQuestion(45, { q: ["A"] }), CodexProtocolError);
  session.questionAnswered(44);
  const terminal = session.observe(frame({ method: "turn/completed", params: {
    threadId: "thread-a", turn: { id: "turn-a", status: "completed", items: [] },
  } }));
  assert.deepEqual(terminal, { kind: "turn-terminal", threadId: "thread-a", turnId: "turn-a", status: "completed" });
  assert.equal(session.turnStatus, "completed");
});

test("append and compact have separate ACK and observed completion semantics", async () => {
  const calls: string[] = [];
  const session = new Codex0160Session({
    async request(method) { calls.push(method); return method === "config/read" ? memoryOffConfig :
      method === "thread/start" ? { thread: { id: "thread-a", cwd: "D:/isolated-instance" } } : {}; },
    async notify() {},
  }, "0.160.0");
  await session.initialize("test");
  await session.verifyMemoryOff("D:/isolated-instance");
  await session.startThread("D:/isolated-instance");
  await session.appendWithoutTurn("context only");
  await session.requestCompaction();
  assert.equal(session.compactionPending, true);
  assert.deepEqual(calls.slice(-2), ["thread/inject_items", "thread/compact/start"]);
  assert.equal(session.observe(frame({ method: "item/completed", params: {
    threadId: "other-thread", item: { id: "i", type: "contextCompaction" },
  } })), null);
  assert.deepEqual(session.observe(frame({ method: "item/completed", params: {
    threadId: "thread-a", item: { id: "i", type: "contextCompaction" },
  } })), { kind: "compaction-item", threadId: "thread-a", itemId: "i" });
  assert.equal(session.compactionPending, false);
});

test("failed transport is unknown acceptance and never falls back to a new thread", async () => {
  const session = new Codex0160Session({
    async request(method) {
      if (method === "config/read") return memoryOffConfig;
      if (method === "thread/resume") return { thread: { id: "thread-a", cwd: "D:/isolated-instance" } };
      if (method === "turn/steer") throw new Error("stdio lost after write");
      if (method === "turn/start") return { turn: { id: "turn-a", status: "inProgress" } };
      return {};
    },
    async notify() {},
  }, "0.160.0");
  await session.initialize("test");
  await session.verifyMemoryOff("D:/isolated-instance");
  await session.resume("thread-a", "D:/isolated-instance");
  await session.startTurn("hello");
  await assert.rejects(session.steer("turn-a", "urgent"), /stdio lost/);
  assert.equal(session.phase, "recovery-required");
  assert.equal(session.uncertainMethod, "turn/steer");
  assert.deepEqual(session.transportLost(), { threadId: "thread-a", uncertainMethod: "turn/steer" });
  await assert.rejects(session.startThread("D:/project"), CodexProtocolError);
});

test("malformed mutation ACK remains unknown and cannot be retried on this connection", async () => {
  const session = new Codex0160Session({
    async request(method) {
      if (method === "config/read") return memoryOffConfig;
      if (method === "thread/start") return { thread: { id: "thread-a", cwd: "D:/isolated-instance" } };
      if (method === "thread/inject_items") return { accepted: true };
      return {};
    }, async notify() {},
  }, "0.160.0");
  await session.initialize("test"); await session.verifyMemoryOff("D:/isolated-instance");
  await session.startThread("D:/isolated-instance");
  await assert.rejects(session.appendWithoutTurn("context"),
    (error: unknown) => error instanceof CodexProtocolError && error.code === "INVALID_RESPONSE");
  assert.equal(session.phase, "recovery-required");
  assert.equal(session.uncertainMethod, "thread/inject_items");
  assert.equal(session.capabilityReport().observedMethods.includes("thread/inject_items"), false);
});

test("wrong thread or turn never creates a native question", async () => {
  const session = new Codex0160Session({
    async request(method) {
      if (method === "config/read") return memoryOffConfig;
      if (method === "thread/resume") return { thread: { id: "thread-a", cwd: "D:/isolated-instance" } };
      if (method === "turn/start") return { turn: { id: "turn-a", status: "inProgress" } };
      return {};
    }, async notify() {},
  }, "0.160.0");
  await session.initialize("test"); await session.verifyMemoryOff("D:/isolated-instance");
  await session.resume("thread-a", "D:/isolated-instance"); await session.startTurn("hello");
  assert.throws(() => session.observe(frame({ id: "server-1", method: "item/tool/requestUserInput", params: {
    threadId: "thread-b", turnId: "turn-a", itemId: "item-a",
    questions: [{ id: "q", header: "Choose", question: "Which?" }],
  } })), (error: unknown) => error instanceof CodexProtocolError && error.code === "QUESTION_BINDING");
  assert.equal(session.pendingQuestion, null);
});

test("terminal notification before turn/start response is retained without a phantom active turn", async () => {
  let release!: (value: unknown) => void;
  const session = new Codex0160Session({
    async request(method) {
      if (method === "config/read") return memoryOffConfig;
      if (method === "thread/start") return { thread: { id: "thread-a", cwd: "D:/isolated-instance" } };
      if (method === "turn/start") return await new Promise<unknown>((resolve) => { release = resolve; });
      return {};
    }, async notify() {},
  }, "0.160.0");
  await session.initialize("test"); await session.verifyMemoryOff("D:/isolated-instance");
  await session.startThread("D:/isolated-instance");
  const pending = session.startTurn("fast");
  await assert.rejects(session.startTurn("duplicate"), CodexProtocolError);
  session.observe(frame({ method: "turn/completed", params: {
    threadId: "thread-a", turn: { id: "turn-fast", status: "completed", items: [] },
  } }));
  release({ turn: { id: "turn-fast", status: "inProgress" } });
  assert.equal(await pending, "turn-fast");
  assert.equal(session.turnStatus, "completed");
  assert.equal(session.observe(frame({ method: "turn/completed", params: {
    threadId: "thread-a", turn: { id: "turn-fast", status: "completed", items: [] },
  } })), null);
});
