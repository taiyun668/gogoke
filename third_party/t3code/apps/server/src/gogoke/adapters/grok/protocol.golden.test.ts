import { describe, expect, it } from "vite-plus/test";
import { decodeGrokFrame, GrokJsonlDecoder, GROK_BUILD_PINNED_VERSION } from "./protocol.ts";
import { realWriteInboundFrames, realWriteOutboundFrames } from "./protocol.write.golden.ts";

// Sanitized field subset from the live, no-login Grok Build 1.0.41 ACP probe.
const frames = [
  `{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1,"agentCapabilities":{"loadSession":true,"promptCapabilities":{"image":false,"audio":false,"embeddedContext":true},"sessionCapabilities":{"list":{},"resume":{},"close":{}}},"_meta":{"agentVersion":"1.0.41"}}}`,
  `{"jsonrpc":"2.0","method":"_x.ai/session/setup","params":{"method":"session/new","phase":"auth","sessionId":null}}`,
  `{"jsonrpc":"2.0","id":2,"error":{"code":-32000,"message":"Authentication required","data":"no auth method id provided"}}`,
] as const;

describe("Grok Build 1.0.41 observed ACP golden", () => {
it("preserves the no-auth boundary", () => {
  const init = decodeGrokFrame(new TextEncoder().encode(frames[0]));
  expect(init.kind).toBe("response");
  if (init.kind !== "response") throw new Error("initialize response expected");
  expect(init.id).toBe(1);
  expect(JSON.parse(JSON.stringify(init.result))).toEqual({
    protocolVersion: 1,
    agentCapabilities: {
      loadSession: true,
      promptCapabilities: { image: false, audio: false, embeddedContext: true },
      sessionCapabilities: { list: {}, resume: {}, close: {} },
    },
    _meta: { agentVersion: GROK_BUILD_PINNED_VERSION },
  });

  expect(decodeGrokFrame(new TextEncoder().encode(frames[1]))).toEqual({
    kind: "notification",
    method: "_x.ai/session/setup",
    params: { method: "session/new", phase: "auth", sessionId: null },
  });
  expect(decodeGrokFrame(new TextEncoder().encode(frames[2]))).toEqual({
    kind: "response",
    id: 2,
    error: { code: -32000, message: "Authentication required", data: "no auth method id provided" },
  });
});

it("decodes the observed messages across stdio chunk boundaries", () => {
  const decoded: unknown[] = [];
  const errors: unknown[] = [];
  const decoder = new GrokJsonlDecoder((frame) => decoded.push(frame), (error) => errors.push(error));
  const bytes = new TextEncoder().encode(frames.join("\n") + "\n");
  decoder.push(bytes.subarray(0, 37));
  decoder.push(bytes.subarray(37, 211));
  decoder.push(bytes.subarray(211));
  decoder.finish();
  expect(errors).toEqual([]);
  expect(decoded).toHaveLength(3);
});
});

describe("Grok Build 1.0.41 observed Write protocol subset", () => {
it("keeps the numeric zero permission ID, selected option, tool terminal and EndTurn", () => {
  const decode = (line: string) => decodeGrokFrame(new TextEncoder().encode(line));
  const inbound = realWriteInboundFrames.map(decode);
  const outbound = realWriteOutboundFrames.map(decode);

  // These are separately recorded direction sequences, not an invented merge.
  expect(inbound.map((frame) => frame.kind)).toEqual([
    "notification", "notification", "server-request", "notification", "notification", "response",
  ]);
  expect(inbound.map((frame) => "method" in frame ? frame.method : null)).toEqual([
    "session/update", "session/update", "session/request_permission", "session/update",
    "_x.ai/session/prompt_complete", null,
  ]);
  expect(outbound.map((frame) => frame.kind)).toEqual(["server-request", "response"]);

  const [started, updated, permission, completed, promptComplete, promptResult] = inbound;
  const [prompt, permissionReply] = outbound;
  if (!started || !updated || !permission || !completed || !promptComplete ||
      !promptResult || !prompt || !permissionReply) {
    throw new Error("recorded Grok frame count changed");
  }
  if (started.kind !== "notification" || updated.kind !== "notification" ||
      permission.kind !== "server-request" || completed.kind !== "notification" ||
      promptComplete.kind !== "notification" || promptResult.kind !== "response" ||
      prompt.kind !== "server-request" || permissionReply.kind !== "response") {
    throw new Error("recorded Grok frame direction or role changed");
  }

  const start = started.params as { update: { sessionUpdate: string; toolCallId: string } };
  const progress = updated.params as { update: { sessionUpdate: string; toolCallId: string; kind: string } };
  const request = permission.params as {
    toolCall: { toolCallId: string; kind: string; rawInput: { variant: string } };
    options: { optionId: string; kind: string }[];
  };
  const finish = completed.params as {
    update: { sessionUpdate: string; toolCallId: string; status: string; rawOutput: { type: string } };
  };
  expect(start.update.sessionUpdate).toBe("tool_call");
  expect(progress.update.sessionUpdate).toBe("tool_call_update");
  expect(progress.update.kind).toBe("edit");
  expect(request.toolCall).toMatchObject({ toolCallId: start.update.toolCallId,
    kind: "edit", rawInput: { variant: "Write" } });
  expect([progress.update.toolCallId, finish.update.toolCallId]).toEqual([
    start.update.toolCallId, start.update.toolCallId,
  ]);

  // The real request used numeric id 0; its offered option ID differs from its kind.
  expect(permission.id).toBe(0);
  expect(typeof permission.id).toBe("number");
  expect(request.options.map(({ optionId, kind }) => [optionId, kind])).toEqual([
    ["allow-edits-session", "allow_always"], ["allow-once", "allow_once"],
    ["reject-once", "reject_once"],
  ]);
  expect(permissionReply.id).toBe(permission.id);
  const reply = permissionReply.result as { outcome: { optionId: string; outcome: string } };
  expect(reply.outcome).toEqual({ optionId: "allow-once", outcome: "selected" });
  expect(request.options.find((option) => option.optionId === reply.outcome.optionId)?.kind)
    .toBe("allow_once");

  expect(finish.update).toMatchObject({ sessionUpdate: "tool_call_update", status: "completed",
    rawOutput: { type: "SearchReplace" } });
  expect(promptComplete.params).toMatchObject({ stopReason: "end_turn", agentResult: null });
  expect(typeof prompt.id).toBe("string");
  expect(prompt.method).toBe("session/prompt");
  expect(promptResult.id).toBe(prompt.id);
  expect(promptResult.result).toMatchObject({ stopReason: "end_turn" });
});
});
