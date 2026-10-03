import assert from "node:assert/strict";
import test from "node:test";
import { join } from "node:path";
import { OpenCode11832Adapter } from "./adapter.ts";
import {
  OPENCODE_PINNED_VERSION,
  OpenCodeProtocolError,
  opencodeAcpArguments,
  opencodeInstanceEnvironment,
  opencodeOfficialLoginArguments,
  type OpenCodeAcpTransport,
} from "./protocol.ts";

type RequestMethod = Exclude<Parameters<OpenCodeAcpTransport["request"]>[0], "session/cancel">;
type Deferred<T> = { promise: Promise<T>; resolve(value: T): void; reject(reason: unknown): void };
function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

class ScriptedTransport implements OpenCodeAcpTransport {
  readonly calls: Array<{ method: string; params: Readonly<Record<string, unknown>> }> = [];
  readonly replies = new Map<string, unknown>();
  readonly waits = new Map<string, Promise<unknown>>();
  readonly notifications: Array<{ method: string; params: Readonly<Record<string, unknown>> }> = [];
  eventHandler: ((event: unknown) => void) | undefined;

  async request(method: RequestMethod, params: Readonly<Record<string, unknown>>): Promise<unknown> {
    this.calls.push({ method, params });
    const wait = this.waits.get(method);
    if (wait) return wait;
    if (!this.replies.has(method)) throw new Error(`no scripted response for ${method}`);
    return this.replies.get(method);
  }

  async notify(method: "session/cancel", params: Readonly<Record<string, unknown>>): Promise<void> {
    this.notifications.push({ method, params });
  }

  setEventHandler(handler: (event: unknown) => void): void { this.eventHandler = handler; }
}

const initializeResponse = {
  protocolVersion: 1,
  agentCapabilities: { loadSession: true },
  agentInfo: { name: "OpenCode", version: OPENCODE_PINNED_VERSION },
  authMethods: [{
    id: "opencode-login",
    _meta: { "terminal-auth": { command: "opencode", args: ["auth", "login"], label: "OpenCode Login" } },
  }],
};

async function readyAdapter(transport = new ScriptedTransport(),
  onEvent?: (event: { provider: "opencode"; nativeSessionId: string; method: string; payload: unknown }) => void) {
  transport.replies.set("initialize", initializeResponse);
  transport.replies.set("session/new", { sessionId: "native-session-a", configOptions: [] });
  const adapter = new OpenCode11832Adapter({ transport, observedVersion: OPENCODE_PINNED_VERSION, onEvent });
  await adapter.initialize("test-client");
  return { adapter, transport };
}

test("ACP initialization pins source version and binds a newly opened native session", async () => {
  const { adapter, transport } = await readyAdapter();
  assert.equal(await adapter.open("workspace-root"), "native-session-a");
  assert.equal(adapter.nativeSessionId, "native-session-a");
  assert.equal(transport.calls[0]?.method, "initialize");
  assert.equal(transport.calls[1]?.params.cwd, "workspace-root");
  assert.deepEqual(opencodeAcpArguments(), ["acp"]);
  assert.deepEqual(adapter.hostedLoginCommand, {
    methodId: "opencode-login", args: ["auth", "login"], label: "OpenCode Login",
  });
  assert.deepEqual(opencodeOfficialLoginArguments(), ["auth", "login"]);
  assert.deepEqual(transport.calls[0]?.params.clientCapabilities, {
    fs: { readTextFile: false, writeTextFile: false }, _meta: { "terminal-auth": true },
  });
  assert.equal(adapter.capabilityReport().runtimeEvidence, "NOT_RUN");
});

test("resume accepts upstream v1.18.32 response that omits sessionId but never a changed ID", async () => {
  const transport = new ScriptedTransport();
  transport.replies.set("initialize", initializeResponse);
  transport.replies.set("session/resume", { configOptions: [] });
  const adapter = new OpenCode11832Adapter({ transport, observedVersion: OPENCODE_PINNED_VERSION });
  await adapter.initialize("test-client");
  await adapter.resume("native-session-a", "workspace-root");
  assert.equal(adapter.nativeSessionId, "native-session-a");
  assert.equal(transport.calls[1]?.params.sessionId, "native-session-a");

  const changed = new ScriptedTransport();
  changed.replies.set("initialize", initializeResponse);
  changed.replies.set("session/resume", { sessionId: "other-session", configOptions: [] });
  const second = new OpenCode11832Adapter({ transport: changed, observedVersion: OPENCODE_PINNED_VERSION });
  await second.initialize("test-client");
  await assert.rejects(second.resume("native-session-a", "workspace-root"),
    (error: unknown) => error instanceof OpenCodeProtocolError && error.code === "SESSION_MISMATCH");
});

test("events emitted before session/new response are buffered then tagged with the bound session", async () => {
  const transport = new ScriptedTransport();
  const received: unknown[] = [];
  const { adapter } = await readyAdapter(transport, (event) => received.push(event));
  transport.eventHandler?.({
    method: "session/update",
    params: { sessionId: "native-session-a", update: { sessionUpdate: "available_commands_update" } },
  });
  await adapter.open("workspace-root");
  assert.deepEqual(received, [{
    provider: "opencode",
    nativeSessionId: "native-session-a",
    method: "session/update",
    payload: { sessionId: "native-session-a", update: { sessionUpdate: "available_commands_update" } },
  }]);
});

test("ACP interruption waits for cancelled prompt and reports no process-stop proof", async () => {
  const transport = new ScriptedTransport();
  const pending = deferred<unknown>();
  transport.waits.set("session/prompt", pending.promise);
  const { adapter } = await readyAdapter(transport);
  await adapter.open("workspace-root");
  const turn = adapter.send("work");
  const interruption = adapter.interrupt("native-session-a");
  assert.deepEqual(transport.notifications, [{
    method: "session/cancel", params: { sessionId: "native-session-a" },
  }]);
  pending.resolve({ stopReason: "cancelled" });
  const result = await interruption;
  assert.deepEqual(result, { status: "interrupted", nativeSessionId: "native-session-a", processStopped: false });
  assert.deepEqual(await turn, { status: "settled", nativeSessionId: "native-session-a", stopReason: "cancelled" });
});

test("ACP cannot claim in-turn steer and requires exact session for interrupt", async () => {
  const transport = new ScriptedTransport();
  const pending = deferred<unknown>();
  transport.waits.set("session/prompt", pending.promise);
  const { adapter } = await readyAdapter(transport);
  await adapter.open("workspace-root");
  await assert.rejects(adapter.steer("native-session-a", "clarify"),
    (error: unknown) => error instanceof OpenCodeProtocolError && error.code === "UNSUPPORTED");
  const turn = adapter.send("work");
  await assert.rejects(adapter.interrupt("another-session"),
    (error: unknown) => error instanceof OpenCodeProtocolError && error.code === "SESSION_MISMATCH");
  pending.resolve({ stopReason: "end_turn" });
  await turn;
});

test("interruptAndResume sends only after the prior prompt confirms cancellation", async () => {
  const transport = new ScriptedTransport();
  const pending = deferred<unknown>();
  transport.waits.set("session/prompt", pending.promise);
  const { adapter } = await readyAdapter(transport);
  await adapter.open("workspace-root");
  const oldTurn = adapter.send("old work");
  const resumed = adapter.interruptAndResume("native-session-a", "new direction");
  pending.resolve({ stopReason: "cancelled" });
  await oldTurn;
  const newResult = await resumed;
  assert.equal(newResult.stopReason, "cancelled");
  assert.deepEqual(transport.calls.filter((call) => call.method === "session/prompt")
    .map((call) => call.params.prompt), [
      [{ type: "text", text: "old work" }],
      [{ type: "text", text: "new direction" }],
    ]);
});

test("instance home environment isolates global roots and disables Claude compatibility", () => {
  const environment = opencodeInstanceEnvironment("instance-home");
  assert.equal(environment.USERPROFILE, "instance-home");
  assert.equal(environment.XDG_CONFIG_HOME, join("instance-home", ".config"));
  assert.equal(environment.OPENCODE_CONFIG_DIR, join("instance-home", ".opencode"));
  assert.equal(environment.OPENCODE_CONFIG, join("instance-home", ".opencode", "opencode.json"));
  assert.equal(environment.OPENCODE_CONFIG_CONTENT, "{}");
  assert.equal(environment.OPENCODE_DISABLE_CLAUDE_CODE_PROMPT, "1");
  assert.equal(environment.OPENCODE_DISABLE_CLAUDE_CODE_SKILLS, "1");
  assert.equal(environment.OPENCODE_DISABLE_CLAUDE_CODE, "1");
});
