import readline from "node:readline";

const rl = readline.createInterface({ input: process.stdin });
rl.on("line", (line) => {
  const message = JSON.parse(line);
  if (message.type !== "user") return;
  process.stdout.write(JSON.stringify({
    type: "assistant", uuid: "fake-assistant",
    message: { content: [{ type: "text", text: "partial output" }] },
  }) + "\n");
  const text = String(message.message?.content?.[0]?.text ?? "");
  if (text.includes("exit-17")) {
    setTimeout(() => process.exit(17), 0);
    return;
  }
  process.stdout.write(JSON.stringify({ type: "result", subtype: "error_during_execution" }) + "\n");
});
