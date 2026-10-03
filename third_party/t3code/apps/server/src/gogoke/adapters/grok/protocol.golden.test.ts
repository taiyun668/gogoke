import { describe, expect, it } from "vite-plus/test";
import { decodeGrokFrame, GrokJsonlDecoder, GROK_BUILD_PINNED_VERSION } from "./protocol.ts";

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
