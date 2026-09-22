import readline from "node:readline";

const rl = readline.createInterface({ input: process.stdin });
let cancelled = false;

function send(value) {
  process.stdout.write(`${JSON.stringify(value)}\n`);
}

rl.on("line", (line) => {
  const message = JSON.parse(line);
  if (message.method === "session/cancel") {
    cancelled = true;
    return;
  }
  if (message.method === "initialize") {
    send({ jsonrpc: "2.0", id: message.id, result: {
      protocolVersion: 1,
      authMethods: [{ id: "cached_token" }],
      defaultAuthMethodId: "cached_token",
    } });
    return;
  }
  if (message.method === "authenticate") {
    send({ jsonrpc: "2.0", id: message.id, result: { _meta: {
      email: "grok-seat@example.test",
      auth_mode: "Oidc",
      subscription_tier: "SuperGrok",
    } } });
    return;
  }
  if (message.method === "session/new") {
    send({ jsonrpc: "2.0", id: message.id, result: { sessionId: "fake-grok-session" } });
    return;
  }
  if (message.method === "session/prompt") {
    const text = String(message.params?.prompt?.[0]?.text ?? "");
    const delta = text.includes("补充") ? "已按中途补充继续。" : "第一段实时回答。";
    send({ jsonrpc: "2.0", method: "session/update", params: {
      sessionId: "fake-grok-session",
      update: { sessionUpdate: "agent_message_chunk", content: { type: "text", text: delta } },
    } });
    setTimeout(() => {
      if (text.includes("显式错误")) {
        send({ jsonrpc: "2.0", id: message.id, error: { code: -32001, message: "provider terminal failure" } });
        return;
      }
      send({ jsonrpc: "2.0", id: message.id, result: {
        stopReason: cancelled ? "cancelled" : "end_turn",
      } });
      cancelled = false;
    }, text.includes("慢速") || text.includes("迟到") ? 80 : 20);
  }
});
