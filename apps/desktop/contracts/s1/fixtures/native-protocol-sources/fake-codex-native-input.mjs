import readline from "node:readline";
import { closeSync } from "node:fs";

const rl = readline.createInterface({ input: process.stdin });
let prompt = "";
let answered = new Set();
let turnCounter = 0;
let activeTurnId = "fake-input-turn";
let requiredAnswers = 2;

function send(value) { process.stdout.write(`${JSON.stringify(value)}\n`); }

function complete(text = "continued after native answers") {
  send({ method: "item/agentMessage/delta", params: {
    threadId: "fake-codex-thread", turnId: activeTurnId, itemId: "assistant-1",
    delta: text,
  } });
  send({ method: "item/completed", params: {
    threadId: "fake-codex-thread", turnId: activeTurnId,
    item: { id: "assistant-1", type: "agentMessage", text },
  } });
  send({ method: "turn/completed", params: {
    threadId: "fake-codex-thread", turn: { id: activeTurnId, status: "completed" },
  } });
}

rl.on("line", (line) => {
  const message = JSON.parse(line);
  if (message.method === "initialize") return send({ id: message.id, result: { userAgent: "fake-native-input" } });
  if (message.method === "account/read") return send({ id: message.id, result: {
    account: { type: "chatgpt", email: "native-input@example.test", planType: "plus" }, requiresOpenaiAuth: true,
  } });
  if (message.method === "thread/start") return send({ id: message.id, result: { thread: { id: "fake-codex-thread" } } });
  if (message.method === "thread/settings/update") return send({ id: message.id, result: {} });
  if (message.method === "turn/start") {
    prompt = String(message.params?.input?.[0]?.text ?? "");
    answered = new Set();
    turnCounter += 1;
    activeTurnId = turnCounter === 1 ? "fake-input-turn" : `fake-input-turn-${turnCounter}`;
    requiredAnswers = prompt.includes("reuse-id") ? 1 : 2;
    // The reverse request deliberately reuses the numeric turn/start id. A
    // method-bearing message must never be consumed as the client's response.
    send({ id: message.id, result: { turn: { id: activeTurnId } } });
    send({ method: "turn/started", params: { threadId: "fake-codex-thread", turn: { id: activeTurnId } } });
    if (prompt.includes("unsupported")) {
      send({ id: 77, method: "item/commandExecution/requestApproval", params: {
        threadId: "fake-codex-thread", turnId: activeTurnId, itemId: "approval-1",
      } });
      return;
    }
    const firstId = prompt.includes("reuse-id") ? "reused-native-question" : message.id;
    send({ id: firstId, method: "item/tool/requestUserInput", params: {
      threadId: "fake-codex-thread", turnId: activeTurnId, itemId: "input-secret",
      isBlocking: true, autoResolutionMs: null,
      questions: [
        { id: "mode", header: "Mode", question: "Choose the native mode?", isOther: false, isSecret: false,
          options: [{ label: "Alpha", description: "Use alpha" }, { label: "Beta", description: "Use beta" }] },
        { id: "token", header: "Token", question: "Enter the native secret?", isOther: false, isSecret: true, options: null },
      ],
    } });
    if (prompt.includes("response-loss")) {
      setInterval(() => {}, 1000);
      closeSync(0);
      return;
    }
    if (prompt.includes("cancel")) {
      setTimeout(() => {
        send({ method: "serverRequest/resolved", params: { threadId: "fake-codex-thread", requestId: firstId } });
        complete();
      }, 15);
      return;
    }
    if (prompt.includes("reuse-id")) return;
    send({ id: "nonblocking-2", method: "item/tool/requestUserInput", params: {
      threadId: "fake-codex-thread", turnId: activeTurnId, itemId: "input-note",
      isBlocking: false, autoResolutionMs: 5000,
      questions: [{ id: "note", header: "Note", question: "Add the native note?", isOther: true, isSecret: false, options: null }],
    } });
    return;
  }
  if (message.id === 77 && message.error?.code === -32601) return complete("unsupported rejected");
  if (Object.prototype.hasOwnProperty.call(message, "result") && !message.method) {
    if (message.id === "nonblocking-2") {
      if (message.result?.answers?.note?.answers?.[0] !== "answer-freeform-827") process.exit(31);
      answered.add("second");
    } else {
      if (message.result?.answers?.mode?.answers?.[0] !== "Beta" ||
          message.result?.answers?.token?.answers?.[0] !== "native-secret-value") process.exit(32);
      answered.add("first");
    }
    if (answered.size === requiredAnswers) complete();
  }
});
