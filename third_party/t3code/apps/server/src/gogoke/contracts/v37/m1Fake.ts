import { canonicalJson, parseStrictJsonBytes } from "../strictJson.ts";
import type { JsonObject, JsonValue } from "../model.ts";
import { decodeV37Request, encodeV37Receipt, V37_SCHEMA, type V37Port, type V37Receipt, type V37Request, type V37TrustedCaller } from "./protocol.ts";

type CardState = "OPEN" | "ANSWERED" | "EXPIRED";
interface Card {
  revision: bigint; state: CardState; requestRef: string; seatId: string;
  turnId: string; generation: string; options: readonly { id: string; recommended: boolean }[];
  answer?: string;
}
interface Seat {
  revision: bigint; layer: "USER" | "LEAD"; lifecycle: "SHORT" | "LONG" | "RECLAIMED";
  settings: JsonObject; instanceId?: string;
}
interface Instance {
  revision: bigint; homeRef: string; programDigest: string; version: string;
  installed: boolean; loggedIn: boolean;
}
interface Prior { readonly request: string; readonly receipt: V37Receipt; }

export class V37M1FakeStore {
  readonly templates = new Map<string, JsonObject>();
  readonly cards = new Map<string, Card>();
  readonly seats = new Map<string, Seat>();
  readonly instances = new Map<string, Instance>();
  readonly replies = new Map<string, Prior>();
  readonly homeRefs = new Set<string>();
}

export interface V37M1FakeOptions {
  readonly caller: () => V37TrustedCaller | null;
  readonly granted: (caller: V37TrustedCaller, request: V37Request) => boolean;
  readonly verifyMemoryDisabled?: (instanceId: string) => boolean;
  readonly verifyProgramDigest?: (digest: string) => boolean;
  readonly nativeCardCapability?: (driverId: string) => boolean | null;
  readonly isSeatBusy?: (seatId: string) => boolean;
  readonly capacity?: (instanceId: string) => string;
}

const nonempty = (payload: JsonObject, key: string): string => {
  const value = payload[key];
  if (typeof value !== "string" || value.length === 0 || value !== value.trim()) {
    throw new Error(`V37_M1_INVALID: payload.${key}`);
  }
  return value;
};

const copy = (value: JsonObject): JsonObject =>
  parseStrictJsonBytes(new TextEncoder().encode(canonicalJson(value))) as JsonObject;

function cardOptions(payload: JsonObject): readonly { id: string; recommended: boolean }[] {
  const raw = payload.options;
  if (!Array.isArray(raw) || raw.length === 0) throw new Error("V37_M1_INVALID: options");
  const parsed = raw.map((entry: JsonValue) => {
    if (entry === null || Array.isArray(entry) || typeof entry !== "object") {
      throw new Error("V37_M1_INVALID: option");
    }
    const option = entry as JsonObject;
    if (typeof option.id !== "string" || option.id.length === 0 ||
        typeof option.recommended !== "boolean") {
      throw new Error("V37_M1_INVALID: option fields");
    }
    return { id: option.id, recommended: option.recommended };
  });
  if (parsed.filter((option) => option.recommended).length !== 1 ||
      new Set(parsed.map((option) => option.id)).size !== parsed.length) {
    throw new Error("V37_M1_INVALID: exactly one recommended and unique option IDs required");
  }
  return parsed;
}

/** M1 test fake. Its callbacks stand for native observations; they grant no product authority. */
export class V37M1FakePort implements V37Port {
  readonly store: V37M1FakeStore;
  readonly options: V37M1FakeOptions;
  constructor(store: V37M1FakeStore, options: V37M1FakeOptions) {
    this.store = store;
    this.options = options;
  }

  async execute(bytes: Uint8Array): Promise<Uint8Array> {
    const request = decodeV37Request(bytes);
    if (!["K-QCARD", "K-SEAT", "K-INSTANCE"].includes(request.family)) {
      throw new Error("V37_M1_UNSUPPORTED_FAMILY");
    }
    const key = `${request.domainId}:${request.targetId}`;
    const card = request.family === "K-QCARD" ? this.store.cards.get(key) : undefined;
    const seat = request.family === "K-SEAT" ? this.store.seats.get(key) : undefined;
    const instance = request.family === "K-INSTANCE" ? this.store.instances.get(key) : undefined;
    const current = card?.revision ?? seat?.revision ?? instance?.revision ?? 0n;
    const reply = (status: V37Receipt["status"], next = current,
      result: JsonObject = {}): V37Receipt => ({ schema: V37_SCHEMA, family: request.family,
      operation: request.operation, requestId: request.requestId, targetId: request.targetId,
      status, previousRevision: current.toString(), revision: next.toString(), result });
    const caller = this.options.caller();
    if (caller === null || caller.domainId !== request.domainId ||
        !this.options.granted(caller, request)) {
      return encodeV37Receipt(reply("DENIED"));
    }
    const replayKey = `${request.family}:${request.domainId}:${request.requestId}`;
    const canonical = canonicalJson(request as unknown as JsonValue);
    const prior = this.store.replies.get(replayKey);
    if (prior) return encodeV37Receipt(prior.request === canonical
      ? { ...prior.receipt, status: "REPLAYED" } : reply("CONFLICT"));
    const committed = (receipt: V37Receipt): Uint8Array => {
      this.store.replies.set(replayKey, { request: canonical, receipt });
      return encodeV37Receipt(receipt);
    };
    if (BigInt(request.expectedRevision) !== current) return encodeV37Receipt(reply("STALE"));

    if (request.family === "K-QCARD") {
      if (request.operation === "raise") {
        if (card) return encodeV37Receipt(reply("CONFLICT"));
        // Capability comes from the adapter registry, not a caller-declared flag.
        if (this.options.nativeCardCapability?.(nonempty(request.payload, "driverId")) !== false) {
          return encodeV37Receipt(reply("UNSUPPORTED"));
        }
        const options = cardOptions(request.payload);
        const created: Card = { revision: 1n, state: "OPEN", options,
          requestRef: nonempty(request.payload, "requestRef"),
          seatId: nonempty(request.payload, "seatId"),
          turnId: nonempty(request.payload, "turnId"),
          generation: nonempty(request.payload, "generation") };
        this.store.cards.set(key, created);
        return committed(reply("APPLIED", 1n, { state: "OPEN" }));
      }
      if (!card) return encodeV37Receipt(reply("CONFLICT"));
      if (request.operation === "recover") {
        if (card.state !== "OPEN") return encodeV37Receipt(reply("CONFLICT"));
        card.revision += 1n;
        return committed(reply("APPLIED", card.revision,
          { state: card.state, requestRef: card.requestRef, seatId: card.seatId,
            turnId: card.turnId, generation: card.generation }));
      }
      if (card.state !== "OPEN") return encodeV37Receipt(reply("CONFLICT"));
      if (request.operation === "answer") {
        if (nonempty(request.payload, "requestRef") !== card.requestRef ||
            nonempty(request.payload, "seatId") !== card.seatId ||
            nonempty(request.payload, "turnId") !== card.turnId ||
            nonempty(request.payload, "generation") !== card.generation) {
          return encodeV37Receipt(reply("CONFLICT"));
        }
        const option = request.payload.optionId;
        const freeText = request.payload.freeText;
        if ((typeof option === "string") === (typeof freeText === "string")) {
          throw new Error("V37_M1_INVALID: exactly one answer form required");
        }
        if (typeof option === "string" && !card.options.some((x) => x.id === option)) {
          return encodeV37Receipt(reply("CONFLICT"));
        }
        card.answer = typeof option === "string" ? option : nonempty(request.payload, "freeText");
        card.state = "ANSWERED";
      } else if (request.operation === "expire") {
        card.state = "EXPIRED";
      } else return encodeV37Receipt(reply("UNSUPPORTED"));
      card.revision += 1n;
      return committed(reply("APPLIED", card.revision, { state: card.state }));
    }

    if (request.family === "K-SEAT") {
      if (request.operation === "create-from-template") {
        if (seat) return encodeV37Receipt(reply("CONFLICT"));
        const layer = nonempty(request.payload, "layer");
        if (layer !== "USER" && layer !== "LEAD") throw new Error("V37_M1_INVALID: layer");
        if (caller.role === "lead" && (layer !== "LEAD" || caller.seatId === request.targetId)) {
          return encodeV37Receipt(reply("DENIED"));
        }
        const template = this.store.templates.get(nonempty(request.payload, "templateId"));
        if (!template) return encodeV37Receipt(reply("CONFLICT"));
        this.store.seats.set(key, { revision: 1n, layer, lifecycle: "SHORT", settings: copy(template) });
        return committed(reply("APPLIED", 1n, { state: "SHORT", layer }));
      }
      if (!seat) return encodeV37Receipt(reply("CONFLICT"));
      if (caller.role === "lead" && (seat.layer !== "LEAD" || caller.seatId === request.targetId)) {
        return encodeV37Receipt(reply("DENIED"));
      }
      if (request.operation === "state-card") {
        return committed(reply("APPLIED", current, { state: seat.lifecycle,
          layer: seat.layer, settings: copy(seat.settings), instanceId: seat.instanceId ?? null }));
      }
      if (seat.lifecycle === "RECLAIMED") return encodeV37Receipt(reply("CONFLICT"));
      if (request.operation === "tune") {
        const setting = nonempty(request.payload, "setting");
        if (setting === "__proto__" || setting === "constructor" || setting === "prototype" ||
            request.payload.value === undefined) throw new Error("V37_M1_INVALID: setting");
        seat.settings = { ...seat.settings, [setting]: request.payload.value };
      } else if (request.operation === "bind-instance" || request.operation === "change-instance") {
        if (request.operation === "bind-instance" && seat.instanceId ||
            request.operation === "change-instance" &&
              (!seat.instanceId || this.options.isSeatBusy?.(request.targetId) !== false)) {
          return encodeV37Receipt(reply("CONFLICT"));
        }
        seat.instanceId = nonempty(request.payload, "instanceId");
      } else if (request.operation === "reclaim") {
        seat.lifecycle = "RECLAIMED";
      } else if (request.operation === "short-to-long") {
        if (seat.lifecycle !== "SHORT") return encodeV37Receipt(reply("CONFLICT"));
        seat.lifecycle = "LONG";
      } else return encodeV37Receipt(reply("UNSUPPORTED"));
      seat.revision += 1n;
      return committed(reply("APPLIED", seat.revision, { state: seat.lifecycle }));
    }

    if (request.operation === "register") {
      if (instance) return encodeV37Receipt(reply("CONFLICT"));
      const homeRef = nonempty(request.payload, "homeRef");
      const programDigest = nonempty(request.payload, "programDigest");
      if (this.store.homeRefs.has(homeRef) ||
          !this.options.verifyProgramDigest?.(programDigest) ||
          !this.options.verifyMemoryDisabled?.(request.targetId)) {
        return encodeV37Receipt(reply("DENIED"));
      }
      this.store.homeRefs.add(homeRef);
      this.store.instances.set(key, { revision: 1n, homeRef, programDigest,
        version: nonempty(request.payload, "version"), installed: true, loggedIn: false });
      return committed(reply("APPLIED", 1n, { state: "REGISTERED" }));
    }
    if (!instance) return encodeV37Receipt(reply("CONFLICT"));
    if (request.operation === "repin-after-manual-upgrade") {
      const digest = nonempty(request.payload, "programDigest");
      const version = nonempty(request.payload, "version");
      if (digest === instance.programDigest || !this.options.verifyProgramDigest?.(digest)) {
        return encodeV37Receipt(reply("DENIED"));
      }
      instance.programDigest = digest;
      instance.version = version;
      instance.revision += 1n;
      return committed(reply("APPLIED", instance.revision, { version, programDigest: digest }));
    }
    if (request.operation === "install-state" || request.operation === "login-state" ||
        request.operation === "version-and-new-version" || request.operation === "concurrency-input") {
      if (request.operation === "concurrency-input" && this.options.capacity?.(request.targetId) === undefined) {
        return encodeV37Receipt(reply("UNSUPPORTED"));
      }
      const result: JsonObject = request.operation === "install-state" ? { installed: instance.installed }
        : request.operation === "login-state" ? { loggedIn: instance.loggedIn }
        : request.operation === "version-and-new-version" ? { version: instance.version,
          programDigest: instance.programDigest } : { capacity: this.options.capacity!(request.targetId) };
      return committed(reply("APPLIED", current, result));
    }
    return encodeV37Receipt(reply("UNSUPPORTED"));
  }
}
