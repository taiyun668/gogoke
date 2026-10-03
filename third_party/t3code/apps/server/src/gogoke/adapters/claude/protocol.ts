/** Claude Code CLI 2.1.286 stream-json adapter boundary.
 * Protocol facts are pinned to the public Claude Code CLI/Agent SDK docs.
 * Runtime support remains unverified until a real pinned CLI sample is captured.
 */
export const CLAUDE_PINNED_VERSION = "2.1.286" as const;

export class ClaudeProtocolError extends Error {
  override readonly name = "ClaudeProtocolError";
  readonly code: string;
  constructor(code: string, detail: string) {
    super(`${code}: ${detail}`);
    this.code = code;
  }
}

type JsonRecord = Record<string, unknown>;
const record = (value: unknown): value is JsonRecord =>
  value !== null && typeof value === "object" && !Array.isArray(value);

function requiredString(value: unknown, field: string): string {
  if (typeof value !== "string" || value.length === 0 || value.includes("\0")) {
    throw new ClaudeProtocolError("INVALID_FIELD", field);
  }
  return value;
}

export type ClaudeFrame =
  | { readonly kind: "init"; readonly sessionId: string; readonly model: string | null; readonly raw: JsonRecord }
  | { readonly kind: "assistant"; readonly sessionId: string | null; readonly text: string; readonly raw: JsonRecord }
  | { readonly kind: "stream-event"; readonly sessionId: string | null; readonly raw: JsonRecord }
  | { readonly kind: "result"; readonly sessionId: string | null; readonly subtype: string | null;
      readonly isError: boolean; readonly text: string | null; readonly raw: JsonRecord }
  | { readonly kind: "control-request"; readonly requestId: string | null; readonly subtype: string | null;
      readonly raw: JsonRecord }
  | { readonly kind: "other"; readonly type: string | null; readonly raw: JsonRecord };

/** Parse one complete stdout JSONL record. A process exit is never a turn receipt. */
export function decodeClaudeFrame(bytes: Uint8Array): ClaudeFrame {
  let value: unknown;
  try {
    value = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(bytes));
  } catch (error) {
    throw new ClaudeProtocolError("INVALID_JSONL", error instanceof Error ? error.message : "invalid JSON");
  }
  if (!record(value)) throw new ClaudeProtocolError("INVALID_MESSAGE", "object required");
  const type = typeof value.type === "string" ? value.type : null;
  const sessionId = typeof value.session_id === "string" && value.session_id.length > 0
    ? value.session_id : null;

  if (type === "system" && value.subtype === "init") {
    const model = record(value.model) && typeof value.model.id === "string" ? value.model.id
      : typeof value.model === "string" ? value.model : null;
    return Object.freeze({ kind: "init", sessionId: requiredString(value.session_id, "session_id"), model, raw: value });
  }
  if (type === "assistant") {
    const message = record(value.message) ? value.message : null;
    const content = message && Array.isArray(message.content) ? message.content : [];
    const text = content.filter(record).filter((item) => item.type === "text")
      .map((item) => typeof item.text === "string" ? item.text : "").join("");
    return Object.freeze({ kind: "assistant", sessionId, text, raw: value });
  }
  if (type === "stream_event") return Object.freeze({ kind: "stream-event", sessionId, raw: value });
  if (type === "result") {
    return Object.freeze({ kind: "result", sessionId,
      subtype: typeof value.subtype === "string" ? value.subtype : null,
      isError: value.is_error === true,
      text: typeof value.result === "string" ? value.result : null,
      raw: value });
  }
  if (type === "control_request") {
    const request = record(value.request) ? value.request : null;
    return Object.freeze({ kind: "control-request",
      requestId: typeof value.request_id === "string" ? value.request_id : null,
      subtype: request && typeof request.subtype === "string" ? request.subtype : null,
      raw: value });
  }
  return Object.freeze({ kind: "other", type, raw: value });
}

/** Incremental JSONL framing; malformed lines remain errors and do not become events. */
export class ClaudeJsonlDecoder {
  #pending = new Uint8Array();
  readonly onFrame: (frame: ClaudeFrame) => void;
  readonly onError: (error: ClaudeProtocolError) => void;

  constructor(onFrame: (frame: ClaudeFrame) => void, onError: (error: ClaudeProtocolError) => void) {
    this.onFrame = onFrame;
    this.onError = onError;
  }

  push(chunk: Uint8Array): void {
    if (!(chunk instanceof Uint8Array)) throw new ClaudeProtocolError("INVALID_CHUNK", "Uint8Array required");
    const combined = new Uint8Array(this.#pending.length + chunk.length);
    combined.set(this.#pending);
    combined.set(chunk, this.#pending.length);
    let start = 0;
    for (let index = 0; index < combined.length; index += 1) {
      if (combined[index] !== 10) continue;
      let line = combined.subarray(start, index);
      if (line.at(-1) === 13) line = line.subarray(0, line.length - 1);
      try { this.onFrame(decodeClaudeFrame(line)); }
      catch (error) {
        this.onError(error instanceof ClaudeProtocolError ? error :
          new ClaudeProtocolError("INVALID_JSONL", error instanceof Error ? error.message : "decode failed"));
      }
      start = index + 1;
    }
    this.#pending = combined.slice(start);
  }

  finish(): void {
    if (this.#pending.length !== 0) this.onError(new ClaudeProtocolError("PARTIAL_FRAME_EOF", "stdout ended mid-frame"));
    this.#pending = new Uint8Array();
  }
}

export function encodeClaudeUserInput(text: string): Uint8Array {
  if (typeof text !== "string" || text.length === 0 || text.includes("\0"))
    throw new ClaudeProtocolError("INVALID_INPUT", "text");
  return new TextEncoder().encode(`${JSON.stringify({
    type: "user",
    message: { role: "user", content: [{ type: "text", text }] },
  })}\n`);
}
