import { ContractCodecError, parseStrictJsonBytes } from "../../contracts/strictJson.ts";

/** Direct source: openai/codex rust-v0.149.0 (commit 758ef40f), app-server-protocol. */
export const CODEX_PINNED_VERSION = "0.149.0" as const;
const MAX_FRAME_BYTES = 1024 * 1024;

export class CodexProtocolError extends Error {
  override readonly name = "CodexProtocolError";
  readonly code: string;
  constructor(code: string, detail: string) { super(`${code}: ${detail}`); this.code = code; }
}

type RecordValue = Record<string, unknown>;
const record = (value: unknown): value is RecordValue =>
  value !== null && typeof value === "object" && !Array.isArray(value);
const rpcId = (value: unknown): value is string | number =>
  typeof value === "string" || (typeof value === "number" && Number.isSafeInteger(value));

export interface CodexQuestion {
  readonly id: string;
  readonly header: string;
  readonly question: string;
  readonly isOther: boolean;
  readonly isSecret: boolean;
  readonly options: ReadonlyArray<{ readonly label: string; readonly description: string }> | null;
}

export interface CodexQuestionCard {
  readonly requestId: string | number;
  readonly threadId: string;
  readonly turnId: string;
  readonly itemId: string;
  readonly isBlocking: boolean;
  readonly autoResolutionMs: number | null;
  readonly questions: ReadonlyArray<CodexQuestion>;
}

/** Payload only; C/UI must obtain the user's answer and H correlates requestId. */
export function buildQuestionCardResponse(card: CodexQuestionCard,
  answers: Readonly<Record<string, readonly string[]>>): { readonly answers: Readonly<Record<string, { readonly answers: readonly string[] }>> } {
  const expected = new Set(card.questions.map((question) => question.id));
  if (Object.keys(answers).length !== expected.size ||
      Object.keys(answers).some((id) => !expected.has(id))) {
    throw new CodexProtocolError("INVALID_QUESTION_ANSWER", "answer ids must match the pending card");
  }
  const result: Record<string, { readonly answers: readonly string[] }> = Object.create(null);
  for (const id of expected) {
    const values = answers[id];
    if (!Array.isArray(values) || values.length === 0 ||
        values.some((value) => typeof value !== "string" || value.length === 0)) {
      throw new CodexProtocolError("INVALID_QUESTION_ANSWER", `missing or empty answer for ${id}`);
    }
    result[id] = Object.freeze({ answers: Object.freeze([...values]) });
  }
  return Object.freeze({ answers: Object.freeze(result) });
}

export type CodexFrame =
  | { readonly kind: "notification"; readonly method: string; readonly params: unknown }
  | { readonly kind: "server-request"; readonly id: string | number; readonly method: string; readonly params: unknown;
      readonly questionCard?: CodexQuestionCard }
  | { readonly kind: "response"; readonly id: string | number; readonly result?: unknown; readonly error?: unknown };

const requiredString = (value: unknown, name: string): string => {
  if (typeof value !== "string" || value.length === 0) throw new CodexProtocolError("INVALID_FIELD", name);
  return value;
};

/** Parse without answering. The native card stays bound to the server request ID and turn. */
export function parseQuestionCard(id: string | number, params: unknown): CodexQuestionCard {
  if (!record(params) || !Array.isArray(params.questions) || params.questions.length === 0 ||
      typeof params.isBlocking !== "boolean" ||
      !(params.autoResolutionMs === null || (Number.isSafeInteger(params.autoResolutionMs) &&
        (params.autoResolutionMs as number) >= 0))) {
    throw new CodexProtocolError("INVALID_QUESTION_CARD", "required 0.149.0 question fields");
  }
  const ids = new Set<string>();
  const questions = params.questions.map((value: unknown): CodexQuestion => {
    if (!record(value) || typeof value.isOther !== "boolean" || typeof value.isSecret !== "boolean" ||
        !(value.options === null || Array.isArray(value.options))) {
      throw new CodexProtocolError("INVALID_QUESTION_CARD", "question shape");
    }
    const questionId = requiredString(value.id, "question.id");
    if (ids.has(questionId)) throw new CodexProtocolError("DUPLICATE_QUESTION_ID", questionId);
    ids.add(questionId);
    const options = value.options === null ? null : value.options.map((option: unknown) => {
      if (!record(option)) throw new CodexProtocolError("INVALID_QUESTION_CARD", "option shape");
      return Object.freeze({ label: requiredString(option.label, "option.label"),
        description: requiredString(option.description, "option.description") });
    });
    return Object.freeze({ id: questionId, header: requiredString(value.header, "question.header"),
      question: requiredString(value.question, "question.question"), isOther: value.isOther,
      isSecret: value.isSecret, options: options === null ? null : Object.freeze(options) });
  });
  return Object.freeze({ requestId: id, threadId: requiredString(params.threadId, "threadId"),
    turnId: requiredString(params.turnId, "turnId"), itemId: requiredString(params.itemId, "itemId"),
    isBlocking: params.isBlocking, autoResolutionMs: params.autoResolutionMs as number | null,
    questions: Object.freeze(questions) });
}

export function decodeCodexFrame(bytes: Uint8Array): CodexFrame {
  const value = parseStrictJsonBytes(bytes);
  if (!record(value)) throw new CodexProtocolError("INVALID_MESSAGE", "top-level object required");
  if (typeof value.method === "string") {
    const method = requiredString(value.method, "method");
    if (Object.hasOwn(value, "id")) {
      if (!rpcId(value.id)) throw new CodexProtocolError("INVALID_ID", "server request id");
      const questionCard = method === "item/tool/requestUserInput"
        ? parseQuestionCard(value.id, value.params) : undefined;
      return Object.freeze({ kind: "server-request", id: value.id, method, params: value.params,
        ...(questionCard === undefined ? {} : { questionCard }) });
    }
    return Object.freeze({ kind: "notification", method, params: value.params });
  }
  if (!rpcId(value.id) || Object.hasOwn(value, "result") === Object.hasOwn(value, "error")) {
    throw new CodexProtocolError("INVALID_MESSAGE", "response id and exactly one result or error required");
  }
  return Object.freeze({ kind: "response", id: value.id,
    ...(Object.hasOwn(value, "result") ? { result: value.result } : { error: value.error }) });
}

/** Stdout JSONL framing; malformed frames are reported and never treated as receipts. */
export class CodexJsonlDecoder {
  #pending: Uint8Array = new Uint8Array();
  #discarding = false;
  readonly onFrame: (frame: CodexFrame) => void;
  readonly onError: (error: CodexProtocolError) => void;
  constructor(onFrame: (frame: CodexFrame) => void,
    onError: (error: CodexProtocolError) => void) {
    this.onFrame = onFrame;
    this.onError = onError;
  }

  push(chunk: Uint8Array): void {
    if (!(chunk instanceof Uint8Array)) throw new CodexProtocolError("INVALID_CHUNK", "Uint8Array required");
    const bytes = new Uint8Array(this.#pending.length + chunk.length);
    bytes.set(this.#pending);
    bytes.set(chunk, this.#pending.length);
    this.#pending = new Uint8Array();
    let start = 0;
    for (let index = 0; index < bytes.length; index += 1) {
      if (bytes[index] !== 10) continue;
      if (this.#discarding) { this.#discarding = false; start = index + 1; continue; }
      let frame = bytes.subarray(start, index);
      if (frame.at(-1) === 13) frame = frame.subarray(0, frame.length - 1);
      start = index + 1;
      if (frame.length > MAX_FRAME_BYTES) {
        this.onError(new CodexProtocolError("OVERSIZE_FRAME", "one JSONL frame"));
        continue;
      }
      try { this.onFrame(decodeCodexFrame(frame)); }
      catch (error) {
        this.onError(error instanceof CodexProtocolError ? error :
          error instanceof ContractCodecError ? new CodexProtocolError(error.code, error.message) :
          new CodexProtocolError("INVALID_JSON", error instanceof Error ? error.message : "unknown parse error"));
      }
    }
    this.#pending = bytes.slice(start);
    if (this.#pending.length > MAX_FRAME_BYTES + 1) {
      this.onError(new CodexProtocolError("OVERSIZE_FRAME", "unterminated JSONL frame"));
      this.#pending = new Uint8Array();
      this.#discarding = true;
    }
  }

  finish(): void {
    if (this.#pending.length > 0 || this.#discarding) {
      this.onError(new CodexProtocolError("PARTIAL_FRAME_EOF", "stdout ended mid-frame"));
    }
    this.#pending = new Uint8Array();
    this.#discarding = false;
  }
}
