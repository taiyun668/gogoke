import readline from "node:readline";

const rl = readline.createInterface({ input: process.stdin });
let prompt = "";

function send(value) { process.stdout.write(`${JSON.stringify(value)}\n`); }
function complete(text = "continued after Claude native answer") {
  send({ type: "assistant", uuid: "claude-answer", message: { content: [{ type: "text", text }] } });
  send({ type: "result", subtype: "success" });
}

if (!process.argv.some((value, index) => value === "--permission-prompt-tool" && process.argv[index + 1] === "stdio")) {
  process.exit(42);
}
send({ type: "system", subtype: "init", session_id: "fake-claude-session", apiKeySource: "none" });

rl.on("line", (line) => {
  const message = JSON.parse(line);
  if (message.type === "user") {
    prompt = String(message.message?.content?.[0]?.text ?? "");
    if (prompt.includes("malformed")) {
      send({ type: "control_request", request_id: "claude-malformed-1" });
      return;
    }
    if (prompt.includes("unsupported")) {
      send({ type: "control_request", request_id: "claude-approval-1", request: {
        subtype: "can_use_tool", tool_name: "Bash", input: { command: "echo no" }, tool_use_id: "toolu-bash-1",
      } });
      return;
    }
    const input = { questions: [
      { question: "Choose the Claude mode?", header: "Mode", multiSelect: false,
        options: [{ label: "Alpha", description: "Use alpha" }, { label: "Beta", description: "Use beta" }] },
      { question: "Which Claude features?", header: "Features", multiSelect: true,
        options: [{ label: "Fast", description: "Enable fast" }, { label: "Safe", description: "Enable safe" }] },
    ], marker: "original-input" };
    send({ type: "control_request", request_id: "claude-question-1", request: {
      subtype: "can_use_tool", tool_name: "AskUserQuestion", input, tool_use_id: "toolu-question-1",
    } });
    if (prompt.includes("cancel")) {
      setTimeout(() => { send({ type: "control_cancel_request", request_id: "claude-question-1" }); complete("cancelled natively"); }, 15);
    }
    return;
  }
  if (message.type === "control_response" && message.response?.request_id === "claude-approval-1") {
    if (message.response.subtype !== "error") process.exit(43);
    return complete("unsupported rejected");
  }
  if (message.type === "control_response" && message.response?.request_id === "claude-malformed-1") {
    if (message.response.subtype !== "error") process.exit(44);
    return complete("malformed control request rejected");
  }
  if (message.type === "control_response" && message.response?.request_id === "claude-question-1") {
    const response = message.response;
    if (prompt.includes("replay") && response.subtype === "error") return complete("replay rejected");
    const updated = response.response?.updatedInput;
    if (response.subtype !== "success" || response.response?.behavior !== "allow" ||
        updated?.marker !== "original-input" || updated?.questions?.length !== 2 ||
        updated?.answers?.["Choose the Claude mode?"] !== "Beta" ||
        updated?.answers?.["Which Claude features?"] !== "Fast, Safe") process.exit(41);
    complete();
  }
});
