import type { PiRpcDiagnostic } from "./types.ts";

const LF = 0x0a;
const CR = 0x0d;

const concat = (left: Uint8Array, right: Uint8Array): Uint8Array => {
  const result = new Uint8Array(left.length + right.length);
  result.set(left);
  result.set(right, left.length);
  return result;
};

/** Strict byte-level JSONL decoder: LF only, with an optional trailing CR. */
export class PiJsonlFramer {
  readonly #maxFrameBytes: number;
  readonly #onFrame: (value: unknown) => void;
  readonly #onDiagnostic: (diagnostic: PiRpcDiagnostic) => void;
  #buffer: Uint8Array<ArrayBufferLike> = new Uint8Array();
  #discardingOversize = false;

  constructor(options: {
    readonly maxFrameBytes: number;
    readonly onFrame: (value: unknown) => void;
    readonly onDiagnostic: (diagnostic: PiRpcDiagnostic) => void;
  }) {
    if (!Number.isSafeInteger(options.maxFrameBytes) || options.maxFrameBytes <= 0) {
      throw new TypeError("maxFrameBytes must be a positive safe integer");
    }
    this.#maxFrameBytes = options.maxFrameBytes;
    this.#onFrame = options.onFrame;
    this.#onDiagnostic = options.onDiagnostic;
  }

  push(chunk: Uint8Array): void {
    if (!(chunk instanceof Uint8Array) || chunk.length === 0) return;
    let bytes = concat(this.#buffer, chunk);
    this.#buffer = new Uint8Array();

    while (bytes.length > 0) {
      const delimiter = bytes.indexOf(LF);
      if (this.#discardingOversize) {
        if (delimiter < 0) return;
        this.#discardingOversize = false;
        bytes = bytes.slice(delimiter + 1);
        continue;
      }
      if (delimiter < 0) {
        const possibleTrailingCr = bytes.at(-1) === CR ? 1 : 0;
        if (bytes.length - possibleTrailingCr > this.#maxFrameBytes) {
          this.#oversize();
          this.#discardingOversize = true;
          return;
        }
        this.#buffer = bytes;
        return;
      }

      let frame = bytes.slice(0, delimiter);
      bytes = bytes.slice(delimiter + 1);
      if (frame.at(-1) === CR) frame = frame.slice(0, -1);
      if (frame.length > this.#maxFrameBytes) {
        this.#oversize();
        continue;
      }
      this.#decode(frame);
    }
  }

  finish(): void {
    if (this.#discardingOversize) {
      this.#discardingOversize = false;
      return;
    }
    if (this.#buffer.length > 0) {
      this.#onDiagnostic(
        Object.freeze({
          code: "PARTIAL_FRAME_EOF",
          detail: `stdout ended with ${this.#buffer.length} unframed byte(s)`,
        }),
      );
      this.#buffer = new Uint8Array();
    }
  }

  #oversize(): void {
    this.#onDiagnostic(
      Object.freeze({
        code: "OVERSIZE_FRAME",
        detail: `frame exceeds ${this.#maxFrameBytes} bytes`,
      }),
    );
  }

  #decode(frame: Uint8Array): void {
    let text: string;
    try {
      text = new TextDecoder("utf-8", { fatal: true }).decode(frame);
    } catch {
      this.#onDiagnostic(
        Object.freeze({ code: "INVALID_UTF8", detail: "frame is not valid UTF-8" }),
      );
      return;
    }
    try {
      this.#onFrame(JSON.parse(text));
    } catch {
      this.#onDiagnostic(
        Object.freeze({ code: "INVALID_JSON", detail: "frame is not valid JSON" }),
      );
    }
  }
}
