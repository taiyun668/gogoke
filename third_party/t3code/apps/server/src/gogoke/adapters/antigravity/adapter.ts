import {
  ANTIGRAVITY_CAPABILITIES,
  ANTIGRAVITY_PIN,
  AntigravityProtocolError,
  antigravityUserEventLine,
  type AntigravityEventEnvelope,
} from "./protocol.ts";

export interface AntigravityStdinSink {
  write(line: string): void | Promise<void>;
}

export interface AntigravityAdapterOptions {
  readonly stdin: AntigravityStdinSink;
  /**
   * H supplies the version measured from its pinned binary identity. This is
   * not inferred from model output or a path supplied by the seat.
   */
  readonly observedPinnedVersion: string;
  /** Set only when H/F already bound this process to a resumed conversation. */
  readonly expectedConversationId?: string;
  /** Raw vendor events remain available for A's independent normalization. */
  readonly onVendorEvent: (event: AntigravityEventEnvelope) => void;
}

/**
 * Adapter for the pinned `agy` headless stream-json protocol. It owns no
 * process, filesystem, account, authority, or stop proof. H owns child custody
 * and must use its native stop receipt before starting a continuation process.
 */
export class AntigravityStreamAdapter {
  readonly #stdin: AntigravityStdinSink;
  readonly #onVendorEvent: AntigravityAdapterOptions["onVendorEvent"];
  readonly #expectedConversationId: string | undefined;
  readonly #decoder = new TextDecoder("utf-8", { fatal: true });
  #lineBuffer = "";
  #conversationId: string | undefined;
  #turnActive = false;
  #quarantined = false;
  #writeTail: Promise<void> = Promise.resolve();

  constructor(options: AntigravityAdapterOptions) {
    if (options.observedPinnedVersion !== ANTIGRAVITY_PIN.version) {
      throw new AntigravityProtocolError("VERSION_MISMATCH", `expected ${ANTIGRAVITY_PIN.version}`);
    }
    this.#stdin = options.stdin;
    this.#onVendorEvent = options.onVendorEvent;
    this.#expectedConversationId = options.expectedConversationId;
  }

  get conversationId(): string | undefined {
    return this.#conversationId;
  }
  get turnActive(): boolean {
    return this.#turnActive;
  }
  get processStopped(): false {
    return false;
  }
  get capabilities(): typeof ANTIGRAVITY_CAPABILITIES {
    return ANTIGRAVITY_CAPABILITIES;
  }

  /**
   * Feed stdout bytes. Unknown event types are passed through unchanged;
   * malformed framing or a changed conversation ID quarantines this adapter.
   */
  acceptStdout(chunk: Uint8Array): void {
    this.#requireUsable();
    let decoded: string;
    try {
      decoded = this.#decoder.decode(chunk, { stream: true });
    } catch (error) {
      this.#quarantine();
      throw new AntigravityProtocolError("INVALID_UTF8", "stdout is not valid UTF-8", error);
    }
    this.#lineBuffer += decoded;
    let newline = this.#lineBuffer.indexOf("\n");
    while (newline >= 0) {
      const line = this.#lineBuffer.slice(0, newline).replace(/\r$/u, "");
      this.#lineBuffer = this.#lineBuffer.slice(newline + 1);
      this.#acceptLine(line);
      newline = this.#lineBuffer.indexOf("\n");
    }
  }

  /** Signal a clean stdout EOF. The host still owns process stop settlement. */
  finishStdout(): void {
    if (this.#quarantined) return;
    try {
      this.#lineBuffer += this.#decoder.decode();
    } catch (error) {
      this.#quarantine();
      throw new AntigravityProtocolError("INVALID_UTF8", "stdout ended with invalid UTF-8", error);
    }
    if (this.#lineBuffer.length > 0) {
      this.#quarantine();
      throw new AntigravityProtocolError("PARTIAL_FRAME_EOF", "stdout ended mid-NDJSON event");
    }
  }

  /**
   * Submit one prompt to an idle headless stream. A successful write is only
   * transport acceptance; only a correlated `result` event settles the turn.
   */
  submitPrompt(content: string): Promise<{ readonly status: "sent-not-settled" }> {
    this.#requireUsable();
    if (this.#turnActive) {
      throw new AntigravityProtocolError(
        "TURN_ACTIVE",
        "agy headless protocol has no supported in-turn steer",
      );
    }
    const line = antigravityUserEventLine(content);
    this.#turnActive = true;
    const write = this.#writeTail.then(() => this.#stdin.write(line));
    this.#writeTail = write.then(
      () => undefined,
      () => undefined,
    );
    return write.then(
      () => Object.freeze({ status: "sent-not-settled" as const }),
      (error: unknown) => {
        this.#quarantine();
        throw new AntigravityProtocolError("WRITE_FAILED", "failed to write user event", error);
      },
    );
  }

  /**
   * Deliberately no adapter-level cancel/steer method exists. H must seal
   * admission, terminate under native custody, record confirmed stop, retain
   * the same K-SESSION admission, then launch with the same conversation ID.
   */

  #acceptLine(line: string): void {
    if (line.length === 0) {
      this.#quarantine();
      throw new AntigravityProtocolError("EMPTY_EVENT", "stdout contains an empty NDJSON frame");
    }
    let value: unknown;
    try {
      value = JSON.parse(line);
    } catch (error) {
      this.#quarantine();
      throw new AntigravityProtocolError("INVALID_JSON", "stdout event is not JSON", error);
    }
    if (!record(value) || typeof value.event !== "string" || value.event.length === 0) {
      this.#quarantine();
      throw new AntigravityProtocolError(
        "INVALID_EVENT",
        "event envelope requires a non-empty event name",
      );
    }
    const event = value as AntigravityEventEnvelope;
    let observedId: string | undefined;
    try {
      observedId = eventConversationId(event);
    } catch (error) {
      this.#quarantine();
      throw error;
    }
    if (event.event === "init") {
      let id: string;
      try {
        id = requiredId(event.conversation_id, "init.conversation_id");
      } catch (error) {
        this.#quarantine();
        throw error;
      }
      if (this.#conversationId !== undefined) {
        this.#quarantine();
        throw new AntigravityProtocolError(
          "DUPLICATE_INIT",
          "one process must emit one init event",
        );
      }
      if (this.#expectedConversationId !== undefined && this.#expectedConversationId !== id) {
        this.#quarantine();
        throw new AntigravityProtocolError(
          "RESUME_MISMATCH",
          "agy resumed a different conversation",
        );
      }
      this.#conversationId = id;
    } else {
      if (this.#conversationId === undefined) {
        this.#quarantine();
        throw new AntigravityProtocolError("UNBOUND_EVENT", `${event.event} arrived before init`);
      }
      if (observedId !== undefined && observedId !== this.#conversationId) {
        this.#quarantine();
        throw new AntigravityProtocolError(
          "CONVERSATION_CHANGED",
          `${event.event} belongs to another conversation`,
        );
      }
    }

    if (event.event === "step_update") {
      const step = event.step_update;
      if (!record(step)) {
        this.#quarantine();
        throw new AntigravityProtocolError(
          "INVALID_EVENT",
          "step_update payload must be an object",
        );
      }
      if (step.step_type === "user_input") this.#turnActive = true;
    } else if (event.event === "result") {
      if (!record(event.result) || typeof event.result.status !== "string") {
        this.#quarantine();
        throw new AntigravityProtocolError("INVALID_EVENT", "result requires a status");
      }
      const resultConversationId = event.result.conversation_id;
      if (
        typeof resultConversationId !== "string" ||
        resultConversationId !== this.#conversationId
      ) {
        this.#quarantine();
        throw new AntigravityProtocolError(
          "CONVERSATION_CHANGED",
          "result lacks or changes the bound conversation",
        );
      }
      if (event.result.status !== "RUNNING") this.#turnActive = false;
    }
    try {
      this.#onVendorEvent(Object.freeze(event));
    } catch (error) {
      this.#quarantine();
      throw new AntigravityProtocolError(
        "EVENT_SINK_FAILED",
        "host did not accept the raw vendor event",
        error,
      );
    }
  }

  #requireUsable(): void {
    if (this.#quarantined)
      throw new AntigravityProtocolError("SESSION_QUARANTINED", "protocol state is unknown");
  }

  #quarantine(): void {
    this.#quarantined = true;
    this.#turnActive = true;
  }
}

const record = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);

function requiredId(value: unknown, name: string): string {
  if (typeof value !== "string" || value.length === 0 || value.includes("\0")) {
    throw new AntigravityProtocolError("INVALID_EVENT", `${name} must be a non-empty string`);
  }
  return value;
}

function eventConversationId(event: AntigravityEventEnvelope): string | undefined {
  const candidates: unknown[] = [event.conversation_id];
  if (record(event.step_update)) candidates.push(event.step_update.conversation_id);
  if (record(event.result)) candidates.push(event.result.conversation_id);
  const ids = candidates.filter((id): id is string => typeof id === "string");
  if (ids.length > 1 && ids.some((id) => id !== ids[0])) {
    throw new AntigravityProtocolError(
      "CONVERSATION_CHANGED",
      `${event.event} contains conflicting conversation IDs`,
    );
  }
  return ids[0];
}
