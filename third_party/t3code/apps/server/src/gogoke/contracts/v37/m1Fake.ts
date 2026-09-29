import { canonicalJson, parseStrictJsonBytes } from "../strictJson.ts";
import type { JsonObject, JsonValue } from "../model.ts";
import { decodeV37Request, encodeV37Receipt, V37_SCHEMA, type V37Port, type V37Receipt, type V37Request, type V37TrustedCaller } from "./protocol.ts";
import { rawV37RequestKey } from "./rawRequest.ts";

type CardState = "OPEN" | "ANSWERED" | "EXPIRED";
interface Card {
  revision: bigint; state: CardState; requestRef: string; seatId: string;
  turnId: string; generation: string; options: readonly { id: string; recommended: boolean }[];
  answer?: string;
}
interface Seat {
  revision: bigint; layer: "USER" | "LEAD"; lifecycle: "SHORT" | "LONG" | "RECLAIMED";
  settings: JsonObject; instanceId?: string;
  takeover?: { epoch: string; takerSeatId: string; instanceId: string | null;
    questionIds: readonly string[]; answers: JsonObject };
}
interface Instance {
  revision: bigint; homeRef: string; programDigest: string; version: string;
  installed: boolean; loggedIn: boolean;
}
interface TemporaryHome {
  revision: bigint; instanceId: string; kind: "SESSION" | "CALL"; ownerId: string;
  generation: string; directoryRef: string; state: "ACTIVE" | "CLOSED" | "CLEANED" | "UNKNOWN";
}
interface Prior { readonly request: string; readonly receipt: V37Receipt; readonly takeoverContextKey?: string; }

export interface V37TakeoverContext {
  readonly epoch: string;
  readonly takerSeatId: string;
  readonly instanceId: string | null;
  readonly questionIds: readonly string[];
}

export class V37M1FakeStore {
  readonly templates = new Map<string, JsonObject>();
  readonly cards = new Map<string, Card>();
  readonly seats = new Map<string, Seat>();
  readonly instances = new Map<string, Instance>();
  readonly temporaryHomes = new Map<string, TemporaryHome>();
  readonly replies = new Map<string, Prior>();
  readonly homeRefs = new Set<string>();
  readonly temporaryDirectoryRefs = new Set<string>();
}

export interface V37M1FakeOptions {
  readonly caller: () => V37TrustedCaller | null;
  readonly granted: (caller: V37TrustedCaller, request: V37Request) => boolean;
  readonly verifyMemoryDisabled?: (instanceId: string) => boolean;
  readonly verifyProgramDigest?: (digest: string) => boolean;
  readonly nativeCardCapability?: (driverId: string) => boolean | null;
  readonly isSeatBusy?: (seatId: string) => boolean;
  readonly capacity?: (instanceId: string) => string;
  readonly takeoverContext?: (seatId: string) => V37TakeoverContext | null;
  readonly isTakeoverLead?: (seatId: string) => boolean;
  readonly createTemporaryHome?: (lifecycleId: string, instanceId: string,
    kind: "SESSION" | "CALL", ownerId: string, generation: string) =>
    { directoryRef: string; nativeReceiptId: string } | "UNKNOWN" | null;
  readonly closeTemporaryHome?: (lifecycleId: string, directoryRef: string) =>
    string | "UNKNOWN" | null;
  readonly cleanupTemporaryHome?: (lifecycleId: string, directoryRef: string) =>
    string | "UNKNOWN" | null;
  readonly activeInstanceAdmissions?: (instanceId: string) => number;
  readonly verifyTemporaryHomeIdentity?: (lifecycleId: string, directoryRef: string,
    instanceId: string) => boolean;
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

const contextKey = (context: V37TakeoverContext | null | undefined, isLead: boolean): string =>
  canonicalJson({ context: context ? { epoch: context.epoch,
    takerSeatId: context.takerSeatId, instanceId: context.instanceId,
    questionIds: [...context.questionIds] } : null, isLead });

const validContext = (context: V37TakeoverContext | null | undefined,
  instanceId: string | undefined): context is V37TakeoverContext => !!context &&
  typeof context.epoch === "string" && context.epoch.length > 0 &&
  typeof context.takerSeatId === "string" && context.takerSeatId.length > 0 &&
  context.instanceId === (instanceId ?? null) && Array.isArray(context.questionIds) &&
  context.questionIds.length > 0 &&
  new Set(context.questionIds).size === context.questionIds.length &&
  context.questionIds.every((id) => /^[A-Za-z][A-Za-z0-9_-]{0,127}$/u.test(id) &&
    !["__proto__", "constructor", "prototype"].includes(id));

const currentTakeover = (seat: Seat, context: V37TakeoverContext | null | undefined,
  isLead: boolean): boolean => seat.lifecycle !== "RECLAIMED" && isLead &&
  validContext(context, seat.instanceId) &&
  seat.takeover !== undefined && seat.takeover.epoch === context.epoch &&
  seat.takeover.takerSeatId === context.takerSeatId &&
  seat.takeover.instanceId === context.instanceId &&
  seat.takeover.questionIds.length === context.questionIds.length &&
  seat.takeover.questionIds.every((id, index) => id === context.questionIds[index]);

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
    const temporaryHome = request.family === "K-INSTANCE" &&
      request.operation === "home-lifecycle" ? this.store.temporaryHomes.get(key) : undefined;
    const current = card?.revision ?? seat?.revision ?? temporaryHome?.revision ?? instance?.revision ?? 0n;
    const reply = (status: V37Receipt["status"], next = current,
      result: JsonObject = {}): V37Receipt => ({ schema: V37_SCHEMA, family: request.family,
      operation: request.operation, requestId: request.requestId, targetId: request.targetId,
      status, previousRevision: current.toString(), revision: next.toString(), result });
    const caller = this.options.caller();
    if (caller === null || caller.domainId !== request.domainId ||
        !this.options.granted(caller, request)) {
      return encodeV37Receipt(reply("DENIED"));
    }
    const takeoverOperation = request.family === "K-SEAT" &&
      (request.operation === "state-card" || request.operation === "takeover-answers");
    const observedTakeover = takeoverOperation ? this.options.takeoverContext?.(request.targetId) : undefined;
    const observedLead = takeoverOperation && this.options.isTakeoverLead?.(request.targetId) === true;
    const observedKey = takeoverOperation ? contextKey(observedTakeover, observedLead) : undefined;
    const replayKey = `${request.family}:${request.domainId}:${request.requestId}`;
    const raw = rawV37RequestKey(bytes);
    const prior = this.store.replies.get(replayKey);
    if (prior) {
      if (prior.request !== raw) return encodeV37Receipt(reply("CONFLICT"));
      if (request.operation === "state-card" && prior.receipt.revision !== current.toString()) {
        return encodeV37Receipt(reply("STALE"));
      }
      if (takeoverOperation && prior.takeoverContextKey !== observedKey) {
        return encodeV37Receipt(reply("STALE"));
      }
      return encodeV37Receipt({ ...prior.receipt,
        status: prior.receipt.status === "UNKNOWN" ? "UNKNOWN" : "REPLAYED" });
    }
    const committed = (receipt: V37Receipt): Uint8Array => {
      this.store.replies.set(replayKey, { request: raw, receipt,
        ...(observedKey === undefined ? {} : { takeoverContextKey: observedKey }) });
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
        const ready = currentTakeover(seat, observedTakeover, observedLead);
        return committed(reply("APPLIED", current, { state: seat.lifecycle,
          layer: seat.layer, settings: copy(seat.settings), instanceId: seat.instanceId ?? null,
          takeoverReady: ready,
          takeoverEpoch: ready ? seat.takeover!.epoch : null,
          takeoverAnswers: ready ? copy(seat.takeover!.answers) : null }));
      }
      if (seat.lifecycle === "RECLAIMED") return encodeV37Receipt(reply("CONFLICT"));
      if (request.operation === "takeover-answers") {
        if (this.options.isTakeoverLead?.(request.targetId) !== true) {
          return encodeV37Receipt(reply("DENIED"));
        }
        if (!validContext(observedTakeover, seat.instanceId)) {
          return encodeV37Receipt(reply("UNSUPPORTED"));
        }
        if (caller.seatId !== observedTakeover.takerSeatId) {
          return encodeV37Receipt(reply("DENIED"));
        }
        if (nonempty(request.payload, "takeoverEpoch") !== observedTakeover.epoch) {
          return encodeV37Receipt(reply("STALE"));
        }
        const questions = observedTakeover.questionIds;
        const raw = request.payload.answers;
        if (!Array.isArray(raw) || raw.length !== questions.length) {
          return encodeV37Receipt(reply("CONFLICT"));
        }
        const answers: Record<string, JsonValue> = {};
        for (const item of raw) {
          if (item === null || Array.isArray(item) || typeof item !== "object") {
            return encodeV37Receipt(reply("CONFLICT"));
          }
          const answer = item as JsonObject;
          if (typeof answer.questionId !== "string" || !questions.includes(answer.questionId) ||
              Object.hasOwn(answers, answer.questionId) || typeof answer.answer !== "string" ||
              answer.answer.trim() !== answer.answer || answer.answer.length === 0) {
            return encodeV37Receipt(reply("CONFLICT"));
          }
          const unknown = answer.answer === "UNKNOWN";
          const evidence = unknown ? answer.howToFind : answer.sourceRef;
          if (typeof evidence !== "string" || evidence.trim() !== evidence || evidence.length === 0 ||
              (unknown && answer.sourceRef !== undefined) ||
              (!unknown && answer.howToFind !== undefined)) {
            return encodeV37Receipt(reply("CONFLICT"));
          }
          answers[answer.questionId] = unknown
            ? { answer: "UNKNOWN", howToFind: evidence }
            : { answer: answer.answer, sourceRef: evidence };
        }
        seat.takeover = { epoch: observedTakeover.epoch,
          takerSeatId: observedTakeover.takerSeatId,
          instanceId: observedTakeover.instanceId,
          questionIds: [...questions], answers };
      } else if (request.operation === "tune") {
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
        delete seat.takeover;
      } else if (request.operation === "reclaim") {
        seat.lifecycle = "RECLAIMED";
        delete seat.takeover;
      } else if (request.operation === "short-to-long") {
        if (seat.lifecycle !== "SHORT") return encodeV37Receipt(reply("CONFLICT"));
        seat.lifecycle = "LONG";
      } else return encodeV37Receipt(reply("UNSUPPORTED"));
      seat.revision += 1n;
      return committed(reply("APPLIED", seat.revision, { state: seat.lifecycle }));
    }

    if (request.operation === "home-lifecycle") {
      const action = nonempty(request.payload, "action");
      if (action === "CREATE") {
        if (Object.keys(request.payload).some((name) =>
          !["action", "instanceId", "kind", "ownerId", "generation"].includes(name))) {
          throw new Error("V37_M1_INVALID: temporary home payload");
        }
        if (temporaryHome) return encodeV37Receipt(reply("CONFLICT"));
        const instanceId = nonempty(request.payload, "instanceId");
        const kind = nonempty(request.payload, "kind");
        if (kind !== "SESSION" && kind !== "CALL") {
          throw new Error("V37_M1_INVALID: temporary home kind");
        }
        const ownerId = nonempty(request.payload, "ownerId");
        const generation = nonempty(request.payload, "generation");
        if (!this.store.instances.has(`${request.domainId}:${instanceId}`)) {
          return encodeV37Receipt(reply("DENIED"));
        }
        const created = this.options.createTemporaryHome?.(request.targetId, instanceId,
          kind, ownerId, generation);
        if (!created) return encodeV37Receipt(reply("UNSUPPORTED"));
        if (created === "UNKNOWN") {
          this.store.temporaryHomes.set(key, { revision: 0n, instanceId, kind, ownerId,
            generation, directoryRef: "", state: "UNKNOWN" });
          return committed(reply("UNKNOWN"));
        }
        if (!/^[A-Za-z][A-Za-z0-9_-]{0,127}$/u.test(created.directoryRef) ||
            !created.nativeReceiptId ||
            this.store.temporaryDirectoryRefs.has(created.directoryRef)) {
          return encodeV37Receipt(reply("FAILED"));
        }
        this.store.temporaryDirectoryRefs.add(created.directoryRef);
        this.store.temporaryHomes.set(key, { revision: 1n, instanceId, kind, ownerId,
          generation, directoryRef: created.directoryRef, state: "ACTIVE" });
        return committed(reply("APPLIED", 1n, { state: "ACTIVE",
          directoryRef: created.directoryRef, nativeReceiptId: created.nativeReceiptId }));
      }
      if (action !== "CLOSE" && action !== "CLEANUP") {
        throw new Error("V37_M1_INVALID: temporary home action");
      }
      if (Object.keys(request.payload).some((name) => name !== "action")) {
        throw new Error("V37_M1_INVALID: temporary home payload");
      }
      if (!temporaryHome || temporaryHome.state === "UNKNOWN" ||
          temporaryHome.state === "CLEANED") return encodeV37Receipt(reply("CONFLICT"));
      if (action === "CLOSE") {
        if (temporaryHome.state !== "ACTIVE") return encodeV37Receipt(reply("CONFLICT"));
        const proof = this.options.closeTemporaryHome?.(request.targetId, temporaryHome.directoryRef);
        if (!proof) return encodeV37Receipt(reply("DENIED"));
        if (proof === "UNKNOWN") {
          temporaryHome.state = "UNKNOWN";
          return committed(reply("UNKNOWN"));
        }
        temporaryHome.state = "CLOSED";
        temporaryHome.revision += 1n;
        return committed(reply("APPLIED", temporaryHome.revision, { state: "CLOSED",
          directoryRef: temporaryHome.directoryRef, nativeReceiptId: proof }));
      }
      if (temporaryHome.state !== "CLOSED" ||
          this.options.activeInstanceAdmissions?.(temporaryHome.instanceId) !== 0 ||
          this.options.verifyTemporaryHomeIdentity?.(request.targetId,
            temporaryHome.directoryRef, temporaryHome.instanceId) !== true) {
        return encodeV37Receipt(reply("DENIED"));
      }
      const proof = this.options.cleanupTemporaryHome?.(request.targetId, temporaryHome.directoryRef);
      if (!proof) return encodeV37Receipt(reply("DENIED"));
      if (proof === "UNKNOWN") {
        temporaryHome.state = "UNKNOWN";
        return committed(reply("UNKNOWN"));
      }
      temporaryHome.state = "CLEANED";
      temporaryHome.revision += 1n;
      return committed(reply("APPLIED", temporaryHome.revision, { state: "CLEANED",
        directoryRef: temporaryHome.directoryRef, nativeReceiptId: proof }));
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
