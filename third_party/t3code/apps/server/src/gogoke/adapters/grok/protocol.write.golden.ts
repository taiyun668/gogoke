// Field-minimized, anonymized subset of one observed Grok Build 1.0.41 Write.
// Each array retains its recorded direction order; no cross-direction order is implied.
// The original readback and the field-level redaction index remain private.
export const realWriteInboundFrames = [
  `{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"<SESSION_ID>","update":{"sessionUpdate":"tool_call","toolCallId":"<TOOL_CALL_ID>","title":"<TOOL_TITLE>","rawInput":{"file_path":"<FILE_PATH>","content":"<FILE_CONTENT>"}}}}`,
  `{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"<SESSION_ID>","update":{"sessionUpdate":"tool_call_update","toolCallId":"<TOOL_CALL_ID>","kind":"edit","title":"<TOOL_TITLE>","content":[{"type":"diff","path":"<FILE_PATH>","oldText":"<OLD_TEXT>","newText":"<NEW_TEXT>"}],"locations":[{"path":"<FILE_PATH>"}],"rawInput":{"file_path":"<FILE_PATH>","content":"<FILE_CONTENT>"}}}}`,
  `{"jsonrpc":"2.0","id":0,"method":"session/request_permission","params":{"sessionId":"<SESSION_ID>","toolCall":{"toolCallId":"<TOOL_CALL_ID>","kind":"edit","title":"<TOOL_TITLE>","rawInput":{"variant":"Write","file_path":"<FILE_PATH>","content":"<FILE_CONTENT>"}},"options":[{"optionId":"allow-edits-session","name":"<OPTION_NAME>","kind":"allow_always"},{"optionId":"allow-once","name":"<OPTION_NAME>","kind":"allow_once"},{"optionId":"reject-once","name":"<OPTION_NAME>","kind":"reject_once"}],"_meta":{"kind":"sensitive"}}}`,
  `{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"<SESSION_ID>","update":{"sessionUpdate":"tool_call_update","toolCallId":"<TOOL_CALL_ID>","status":"completed","content":[{"type":"diff","path":"<FILE_PATH>","oldText":"<OLD_TEXT>","newText":"<NEW_TEXT>"}],"rawOutput":{"type":"SearchReplace"}}}}`,
  `{"jsonrpc":"2.0","method":"_x.ai/session/prompt_complete","params":{"sessionId":"<SESSION_ID>","promptId":"<PROMPT_ID>","stopReason":"end_turn","agentResult":null}}`,
  `{"jsonrpc":"2.0","id":"<PROMPT_RPC_ID>","result":{"stopReason":"end_turn"}}`,
] as const;

export const realWriteOutboundFrames = [
  `{"jsonrpc":"2.0","id":"<PROMPT_RPC_ID>","method":"session/prompt","params":{"prompt":[{"text":"<PROMPT_TEXT>","type":"text"}],"sessionId":"<SESSION_ID>"}}`,
  `{"jsonrpc":"2.0","id":0,"result":{"outcome":{"optionId":"allow-once","outcome":"selected"}}}`,
] as const;
