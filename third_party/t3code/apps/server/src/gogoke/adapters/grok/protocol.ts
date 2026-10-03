import { ContractCodecError, parseStrictJsonBytes } from "../../contracts/strictJson.ts";

/** Grok Build version fixed by the Design37 adapter research pin. */
export const GROK_BUILD_PINNED_VERSION = "1.0.41" as const;
export const GROK_BUILD_PROTOCOL_VERSION = 1 as const;
const MAX_FRAME_BYTES = 1024 * 1024;

export class GrokProtocolError extends Error {
  override readonly name = "GrokProtocolError";
  readonly code: string;
  constructor(code: string, detail: string) {
    super(`${code}: ${detail}`);
    this.code = code;
  }
}

type JsonRecord = Record<string, unknown>;
const isRecord = (value: unknown): value is JsonRecord =>
  value !== null && typeof value === "object" && !Array.isArray(value);
const validId = (value: unknown): value is string | number =>
  typeof value === "string" || (typeof value === "number" && Number.isSafeInteger(value));

export type GrokFrame =
  | { readonly kind: "notification"; readonly method: string; readonly params: unknown }
  | { readonly kind: "server-request"; readonly id: string | number; readonly method: string; readonly params: unknown }
  | { readonly kind: "response"; readonly id: string | number; readonly result?: unknown; readonly error?: unknown };

export function decodeGrokFrame(bytes: Uint8Array): GrokFrame {
  if (bytes.length > MAX_FRAME_BYTES) throw new GrokProtocolError("OVERSIZE_FRAME", "one JSONL frame");
  let value: unknown;
  try {
    value = parseStrictJsonBytes(bytes);
  } catch (error) {
    if (error instanceof ContractCodecError) throw new GrokProtocolError(error.code, error.message);
    throw new GrokProtocolError("INVALID_JSON", error instanceof Error ? error.message : "unknown parse error");
  }
  if (!isRecord(value)) throw new GrokProtocolError("INVALID_MESSAGE", "top-level object required");
  if (value.jsonrpc !== "2.0") throw new GrokProtocolError("INVALID_MESSAGE", "jsonrpc must be 2.0");
  if (typeof value.method === "string" && value.method.length > 0) {
    if (!isRecord(value.params)) throw new GrokProtocolError("INVALID_MESSAGE", "method params must be an object");
    if (Object.hasOwn(value, "id")) {
      if (!validId(value.id)) throw new GrokProtocolError("INVALID_ID", "server request id");
      return Object.freeze({ kind: "server-request", id: value.id, method: value.method, params: value.params });
    }
    return Object.freeze({ kind: "notification", method: value.method, params: value.params });
  }
  if (!validId(value.id) || Object.hasOwn(value, "result") === Object.hasOwn(value, "error")) {
    throw new GrokProtocolError("INVALID_MESSAGE", "response id and exactly one result or error required");
  }
  return Object.freeze({ kind: "response", id: value.id,
    ...(Object.hasOwn(value, "result") ? { result: value.result } : { error: value.error }) });
}

/** ACP stdio uses one JSON-RPC message per line. Partial messages fail closed at EOF. */
export class GrokJsonlDecoder {
  #pending = new Uint8Array();
  #discarding = false;
  private readonly onFrame: (frame: GrokFrame) => void;
  private readonly onError: (error: GrokProtocolError) => void;

  constructor(
    onFrame: (frame: GrokFrame) => void,
    onError: (error: GrokProtocolError) => void,
  ) {
    this.onFrame = onFrame;
    this.onError = onError;
  }

  push(chunk: Uint8Array): void {
    if (!(chunk instanceof Uint8Array)) throw new GrokProtocolError("INVALID_CHUNK", "Uint8Array required");
    let offset = 0;
    while (offset < chunk.length) {
      const newline = chunk.indexOf(10, offset);
      const end = newline < 0 ? chunk.length : newline;
      if (!this.#discarding) {
        const part = chunk.subarray(offset, end);
        if (this.#pending.length + part.length > MAX_FRAME_BYTES + 1) {
          this.onError(new GrokProtocolError("OVERSIZE_FRAME", "one JSONL frame"));
          this.#pending = new Uint8Array();
          this.#discarding = true;
        } else {
          const next = new Uint8Array(this.#pending.length + part.length);
          next.set(this.#pending);
          next.set(part, this.#pending.length);
          this.#pending = next;
        }
      }
      if (newline < 0) break;
      if (!this.#discarding) {
        let frame = this.#pending;
        if (frame.at(-1) === 13) frame = frame.subarray(0, frame.length - 1);
        try { this.onFrame(decodeGrokFrame(frame)); }
        catch (error) {
          this.onError(error instanceof GrokProtocolError ? error :
            new GrokProtocolError("INVALID_MESSAGE", error instanceof Error ? error.message : "unknown decode error"));
        }
      }
      this.#pending = new Uint8Array();
      this.#discarding = false;
      offset = newline + 1;
    }
  }

  finish(): void {
    if (this.#pending.length > 0 || this.#discarding) {
      this.onError(new GrokProtocolError("PARTIAL_FRAME_EOF", "stdout ended mid-frame"));
    }
    this.#pending = new Uint8Array();
    this.#discarding = false;
  }
}

export type GrokCapabilityState = "OBSERVED" | "ADVERTISED" | "UNKNOWN" | "NOT_RUN" | "UNSUPPORTED";

export interface GrokCapabilityReport {
  readonly pinnedVersion: typeof GROK_BUILD_PINNED_VERSION;
  readonly observedVersion: string | null;
  readonly versionEvidence: "LIVE_INITIALIZE" | "NOT_RUN";
  readonly acpInitialize: GrokCapabilityState;
  readonly sessionNew: GrokCapabilityState;
  readonly sessionLoad: GrokCapabilityState;
  readonly sessionResume: GrokCapabilityState;
  readonly sessionCancel: GrokCapabilityState;
  readonly steer: GrokCapabilityState;
  readonly nativeQuestionCard: GrokCapabilityState;
  readonly memoryOff: GrokCapabilityState;
  readonly memoryCrossProjectIsolation: GrokCapabilityState;
  readonly observedFeatures: readonly string[];
}
