import readline from "node:readline";

const rl = readline.createInterface({ input: process.stdin });

function send(value) {
  process.stdout.write(`${JSON.stringify(value)}\n`);
}

rl.on("line", (line) => {
  const message = JSON.parse(line);
  if (message.method === "initialize") {
    send({ id: message.id, result: { userAgent: "fake-codex-app-server" } });
    return;
  }
  if (message.method === "account/read") {
    send({ id: message.id, result: {
      account: { type: "chatgpt", email: "codex-seat@example.test", planType: "plus" },
      requiresOpenaiAuth: true,
    } });
    return;
  }
  if (message.method === "thread/start") {
    send({ id: message.id, result: { thread: { id: "fake-codex-thread" } } });
    return;
  }
  if (message.method === "thread/settings/update") {
    send({ id: message.id, result: {} });
    return;
  }
  if (message.method === "turn/start") {
    send({ id: message.id, result: { turn: { id: "fake-exit-turn" } } });
    send({ method: "turn/started", params: { turn: { id: "fake-exit-turn" } } });
    setTimeout(() => process.exit(17), 10);
  }
});
