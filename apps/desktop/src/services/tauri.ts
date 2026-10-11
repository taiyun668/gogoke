import { invoke } from "@tauri-apps/api/core";
import type { SecretarySource } from "@/features/secretary/Secretary";
import type { SecretaryPage, Routine } from "@/features/secretary/secretaryModel";
import type { ConversationItem } from "@/types";
import { readDesign37InstancesSnapshot } from "@/features/seats/design37Instances";

type SecretaryLimits = {
  globalParallelCap?: string | null;
  globalEffectiveLimit?: string | null;
};
type SecretaryConfiguration = SecretaryLimits & (
  | { schema: "gogoke.37.secretary-configuration.v1"; state: "UNSET" | "REVOKED" }
  | { schema: "gogoke.37.secretary-configuration.v1"; state: "DESIGNATED";
      seatId: string; incarnation: string; generation: string; revision: string;
      instanceId: string | null; model: string | null; effort: string | null;
      permissionTier: string | null; seatState: "IDLE" | "BUSY" | "RECLAIMED";
      conversation?: { state: "NONE" | "UNKNOWN" | "CONFLICT" | "FOUND";
        sessionId?: string; generation?: string; revision?: string;
        claimState?: string; stoppedFact?: boolean | null; runtimeAvailable?: boolean;
        historical?: boolean;
        turnState?: "IDLE" | "RUNNING" | "UNKNOWN";
        threadId?: string; ledgerEpoch?: string; ledgerCursor?: string } });

type SecretaryRoutineRow = { routineId: string; seatId: string; incarnation: string;
  originalText: string; scheduleRaw: string; timezone: string; nextDueMs: string;
  state: "ACTIVE" | "PAUSED" | "ABSENCE_PAUSED" | "WAITING_NEXT" | "DELETED";
  revision: string; lastResult: "NONE" | "UNKNOWN" | "FAILED" | "DELIVERED";
  lastReason: string };

const record = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);
const nonempty = (value: unknown): value is string =>
  typeof value === "string" && value.length > 0;
const decimal = (value: unknown): value is string =>
  typeof value === "string" && /^(0|[1-9][0-9]*)$/.test(value);
const optionalText = (value: unknown): value is string | null =>
  value === null || nonempty(value);

function parseSecretaryConfiguration(value: unknown): SecretaryConfiguration {
  if (!record(value) || value.schema !== "gogoke.37.secretary-configuration.v1") {
    throw new Error(`Native Secretary configuration schema is unavailable: ${JSON.stringify(value)}`);
  }
  for (const field of ["globalParallelCap", "globalEffectiveLimit"]) {
    const limit = value[field];
    if (limit !== undefined && limit !== null &&
        (!decimal(limit) || BigInt(limit) < 1n || BigInt(limit) > 4294967295n)) {
      throw new Error(`Native Secretary ${field} is invalid: ${JSON.stringify(limit)}`);
    }
  }
  if (value.state === "UNSET" || value.state === "REVOKED") {
    return value as SecretaryConfiguration;
  }
  if (value.state !== "DESIGNATED" || !nonempty(value.seatId) ||
      !nonempty(value.incarnation) || !decimal(value.generation) ||
      !decimal(value.revision) || !optionalText(value.instanceId) ||
      !optionalText(value.model) || !optionalText(value.effort) ||
      !optionalText(value.permissionTier) ||
      !["IDLE", "BUSY", "RECLAIMED"].includes(String(value.seatState))) {
    throw new Error(`Native Secretary designation is incomplete or invalid: ${JSON.stringify(value)}`);
  }
  if (value.conversation !== undefined) {
    const conversation = value.conversation;
    if (!record(conversation) ||
        !["NONE", "UNKNOWN", "CONFLICT", "FOUND"].includes(String(conversation.state)) ||
        ["sessionId", "generation", "revision", "claimState",
          "threadId", "ledgerEpoch", "ledgerCursor"].some((key) =>
            conversation[key] !== undefined && !nonempty(conversation[key])) ||
        (conversation.stoppedFact !== undefined && conversation.stoppedFact !== null &&
          typeof conversation.stoppedFact !== "boolean") ||
        (conversation.historical !== undefined &&
          typeof conversation.historical !== "boolean") ||
        (conversation.state === "FOUND" &&
          typeof conversation.historical !== "boolean") ||
        (conversation.runtimeAvailable !== undefined &&
          typeof conversation.runtimeAvailable !== "boolean") ||
        (conversation.turnState !== undefined &&
          !["IDLE", "RUNNING", "UNKNOWN"].includes(String(conversation.turnState)))) {
      throw new Error(`Native Secretary conversation fact is invalid: ${JSON.stringify(conversation)}`);
    }
  }
  return value as SecretaryConfiguration;
}

function parseSecretaryRoutines(value: unknown): SecretaryRoutineRow[] {
  if (!record(value) || value.schema !== "gogoke.37.secretary-routines.v1" ||
      value.command !== "secretary-routines-read" || value.status !== "READ" ||
      !Array.isArray(value.routines)) {
    throw new Error(`Native Secretary routines read is unavailable: ${JSON.stringify(value)}`);
  }
  const ids = new Set<string>();
  return value.routines.map((item: unknown) => {
    if (!record(item) || !nonempty(item.routineId) || ids.has(item.routineId) ||
        !nonempty(item.seatId) || !nonempty(item.incarnation) ||
        !nonempty(item.originalText) || !nonempty(item.scheduleRaw) ||
        !nonempty(item.timezone) || !decimal(item.nextDueMs) ||
        !["ACTIVE", "PAUSED", "ABSENCE_PAUSED", "WAITING_NEXT", "DELETED"].includes(String(item.state)) ||
        !decimal(item.revision) || !["NONE", "UNKNOWN", "FAILED", "DELIVERED"].includes(String(item.lastResult)) ||
        typeof item.lastReason !== "string") {
      throw new Error(`Native Secretary routine row is incomplete or invalid: ${JSON.stringify(item)}`);
    }
    ids.add(item.routineId);
    return item as SecretaryRoutineRow;
  });
}

async function readSecretaryConfiguration(): Promise<SecretaryConfiguration> {
  return parseSecretaryConfiguration(await design37UserFrame({
    schema: "gogoke.37.owner-configuration.v1", command: "secretary-configuration-read",
  }));
}

async function readSecretarySettingChoices(): Promise<SecretaryPage["settings"]> {
  const management = await design37UserFrame({
    schema: "gogoke.37.owner-configuration.v1", command: "instance-management-read",
  });
  if (!record(management) || management.schema !== "gogoke.37.instance-management.v1" ||
      !Array.isArray(management.profiles)) {
    throw new Error(`Native Secretary instance choices are malformed: ${JSON.stringify(management)}`);
  }
  const registry = readDesign37InstancesSnapshot(await invoke<unknown>("gogoke_design37_instances"));
  const settings: SecretaryPage["settings"] = { instances: [], efforts: [], permissions: [] };
  const ids = new Set<string>();
  let commonEfforts: string[] | null = null;
  let commonPermissions: string[] | null = null;
  for (const profile of management.profiles) {
    if (!record(profile) || !nonempty(profile.instanceId) || ids.has(profile.instanceId)) {
      throw new Error("Native Secretary instance choice identity is invalid or duplicated.");
    }
    ids.add(profile.instanceId);
    const instance = registry.instances.find((item) => item.instanceId === profile.instanceId);
    if (profile.enabled !== true || instance?.state !== "LOGGED_IN" ||
        profile.driverId !== instance.driverId || profile.programSourceError ||
        !nonempty(profile.name) || !nonempty(profile.modelsSource) ||
        !nonempty(profile.modelsObservedAt)) continue;
    if (!Array.isArray(profile.models) || !profile.models.every(nonempty) ||
        new Set(profile.models).size !== profile.models.length) {
      throw new Error("Native Secretary verified model choice list is invalid.");
    }
    if (profile.models.length === 0) continue;
    // G's present interface offers one effort list. Offer only the intersection
    // of original per-model options, never a vendor default or an E scope.
    if (profile.modelOptions === undefined) continue;
    if (!Array.isArray(profile.modelOptions) ||
        profile.modelOptionsSource !== profile.modelsSource ||
        profile.modelOptionsObservedAt !== profile.modelsObservedAt ||
        profile.modelOptions.length !== profile.models.length) {
      throw new Error("Native Secretary model option provenance differs from the verified model list.");
    }
    const availableModels: string[] = [];
    for (let index = 0; index < profile.models.length; index += 1) {
      const option: unknown = profile.modelOptions[index];
      if (!record(option) || option.model !== profile.models[index] ||
          (option.efforts !== undefined && (!Array.isArray(option.efforts) ||
            !option.efforts.every(nonempty) || new Set(option.efforts).size !== option.efforts.length))) {
        throw new Error("Native Secretary per-model effort options are invalid.");
      }
      const efforts = option.efforts === undefined ? [] : option.efforts as string[];
      if (efforts.length === 0) continue;
      availableModels.push(profile.models[index]);
      commonEfforts = commonEfforts === null ? efforts :
        commonEfforts.filter((effort) => efforts.includes(effort));
    }
    if (availableModels.length > 0) {
      settings.instances.push({ id: profile.instanceId, name: profile.name, models: availableModels });
      const permissions = profile.configurationPermissions ?? [];
      if (!Array.isArray(permissions) || !permissions.every((value: unknown) =>
          ["READ_ONLY", "NO_NETWORK", "ISOLATED_WRITE", "NETWORKED_WRITE"].includes(String(value))) ||
          new Set(permissions).size !== permissions.length) {
        throw new Error("Native Secretary configuration permission choices are invalid.");
      }
      commonPermissions = commonPermissions === null ? permissions as string[] :
        commonPermissions.filter((permission) => permissions.includes(permission));
    }
  }
  settings.efforts = commonEfforts ?? [];
  settings.permissions = commonPermissions ?? [];
  return settings;
}

async function readSecretaryRoutines(): Promise<SecretaryRoutineRow[]> {
  return parseSecretaryRoutines(await design37UserFrame({
    schema: "gogoke.37.owner-configuration.v1", command: "secretary-routines-read",
  }));
}

function routinePageRow(row: SecretaryRoutineRow): Routine {
  // E records delivery outcomes, not an execution time or completion proof.
  const ms = Number(row.nextDueMs);
  if (!Number.isSafeInteger(ms) || !Number.isFinite(new Date(ms).getTime())) {
    throw new Error("Native Secretary next due time is invalid.");
  }
  const nextRun = new Intl.DateTimeFormat("zh-CN", {
    timeZone: row.timezone, dateStyle: "medium", timeStyle: "short",
  }).format(ms);
  return { id: row.routineId, name: row.originalText,
    schedule: `${row.scheduleRaw} · ${row.timezone}`,
    ...(row.state === "ACTIVE" ? { nextRun } : {}),
    ...(row.lastResult === "NONE" ? {} : { lastOutcome: {
      result: row.lastResult,
      ...(row.lastReason === "" ? {} : { reason: row.lastReason }),
    } }),
    paused: row.state === "PAUSED" || row.state === "ABSENCE_PAUSED" };
}

type SecretaryConversation = {
  sessionId: string; threadId: string; seatId: string;
  generation: string; revision: string;
  turnState: "IDLE" | "RUNNING" | "UNKNOWN";
  historical: boolean; runtimeAvailable: boolean;
  messages: ConversationItem[];
  historyGap: string | null;
  statuses: string[];
  inputs: SecretaryOriginalInput[];
  inputRowsEnded: boolean;
  vendorUserFacts: SecretaryVendorUserFact[];
  verifiedSend: { requestId: string; body: string; hGeneration: string } | null;
  writer: { binding: SecretaryBinding; instanceId: string; model: string; effort: string; permissionTier: string;
    canSend: boolean; canStop: boolean } | null;
};

export type SecretaryBinding = {
  seatId: string; incarnation: string; eGeneration: string; eRevision: string;
  instanceId: string; model: string; effort: string; permissionTier: string;
  sessionId: string; threadId: string; hGeneration: string; hRevision: string;
};

export type SecretaryWriteFact = { operation: "send" | "stop"; requestId: string;
  binding: SecretaryBinding;
  sessionId: string; seatId: string; hGeneration: string;
  body: string | null; status: "UNKNOWN" | "ACCEPTED" | "REJECTED"; receipt: unknown | null;
  reason: string | null; inputVerified: boolean };

export type SecretaryOriginalInput = { requestId: string; generation: string;
  operation: "send" | "append-without-turn"; expectedRevision: string;
  body: string | null; bodyState: "VERIFIED" | "UNKNOWN" | "TOO_LARGE";
  occurredAtMs: string | null; phase: "PREPARED" | "UNKNOWN" | "RECEIPTED";
  receiptStatus: string | null; receipt: Record<string, unknown> | null;
  receiptState: string | null; turnId: string | null };

export type SecretaryVendorUserFact = { sourceEventId: string; turnId: string | null;
  text: string; matchedOriginal: boolean };

export const sameSecretaryBinding = (left: SecretaryBinding, right: SecretaryBinding) =>
  (["seatId", "incarnation", "eGeneration", "eRevision", "instanceId", "model",
    "effort", "permissionTier", "sessionId", "threadId", "hGeneration", "hRevision"] as const)
    .every((key) => nonempty(left[key]) && left[key] === right[key]);

export const sameSecretaryWriter = (left: SecretaryBinding, right: SecretaryBinding) =>
  (["seatId", "incarnation", "eGeneration", "eRevision", "instanceId", "model",
    "effort", "permissionTier", "sessionId", "threadId", "hGeneration"] as const)
    .every((key) => nonempty(left[key]) && left[key] === right[key]);

function bindingOf(configuration: Extract<SecretaryConfiguration, { state: "DESIGNATED" }>,
  fact: NonNullable<Extract<SecretaryConfiguration, { state: "DESIGNATED" }>["conversation"]>): SecretaryBinding | null {
  if (!nonempty(configuration.instanceId) || !nonempty(configuration.model) ||
      !nonempty(configuration.effort) || !nonempty(configuration.permissionTier) ||
      !nonempty(fact.sessionId) || !nonempty(fact.threadId) ||
      !decimal(fact.generation) || !decimal(fact.revision)) return null;
  return { seatId: configuration.seatId, incarnation: configuration.incarnation,
    eGeneration: configuration.generation, eRevision: configuration.revision,
    instanceId: configuration.instanceId, model: configuration.model,
    effort: configuration.effort, permissionTier: configuration.permissionTier,
    sessionId: fact.sessionId, threadId: fact.threadId,
    hGeneration: fact.generation, hRevision: fact.revision! };
}

function secretaryReceipt(value: unknown, family: "K-SESSION" | "K-LEDGER",
  operation: string, requestId: string, targetId: string): Record<string, unknown> {
  if (!record(value) || value.schema !== "gogoke.37.operations.v1" ||
      value.family !== family || value.operation !== operation ||
      value.requestId !== requestId || value.targetId !== targetId ||
      !["APPLIED", "REPLAYED"].includes(String(value.status)) ||
      !decimal(value.revision) || !record(value.result)) {
    throw new Error(`Native Secretary ${operation} was not confirmed: ${JSON.stringify(value)}`);
  }
  return value.result;
}

async function secretaryOperation(family: "K-SESSION" | "K-LEDGER", operation: string,
  targetId: string, expectedRevision: string, payload: Record<string, unknown>) {
  const requestId = `secretary_${crypto.randomUUID()}`;
  const reply = await design37UserFrame({ schema: "gogoke.37.operations.v1", family,
    operation, requestId, targetId, domainId: "global", expectedRevision, payload });
  return { result: secretaryReceipt(reply, family, operation, requestId, targetId), reply };
}

async function secretaryRawFrame(frame: string): Promise<unknown> {
  const raw = await invoke<string>("gogoke_design37_user_operation", { frame });
  if (typeof raw !== "string") throw new Error("Native Secretary USER reply is not a JSON frame.");
  return JSON.parse(raw) as unknown;
}

function originalWriteReceipt(value: unknown, operation: "send" | "stop",
  requestId: string, targetId: string, expectedRevision: string): "ACCEPTED" | "REJECTED" | "UNKNOWN" {
  if (!record(value) || value.schema !== "gogoke.37.operations.v1" ||
      value.family !== "K-SESSION" || value.operation !== operation ||
      value.requestId !== requestId || value.targetId !== targetId ||
      !decimal(value.previousRevision) || !decimal(value.revision) ||
      !record(value.result)) {
    throw new Error(`Native Secretary original ${operation} reply has the wrong identity: ${JSON.stringify(value)}`);
  }
  if (value.status === "STALE" || value.status === "DENIED" ||
      value.status === "CONFLICT" || value.status === "UNSUPPORTED") return "REJECTED";
  if (value.status !== "APPLIED" && value.status !== "REPLAYED") return "UNKNOWN";
  if (value.previousRevision !== expectedRevision) {
    throw new Error(`Native Secretary accepted ${operation} has a different original revision: ${JSON.stringify(value)}`);
  }
  if (operation === "stop" && (!record(value.result) || !nonempty(value.result.stopFact))) {
    throw new Error(`Native Secretary stop has no original H fact: ${JSON.stringify(value)}`);
  }
  return "ACCEPTED";
}

function confirmedConversation(configuration: SecretaryConfiguration):
  { configuration: Extract<SecretaryConfiguration, { state: "DESIGNATED" }>;
    fact: NonNullable<Extract<SecretaryConfiguration, { state: "DESIGNATED" }>["conversation"]> } | null {
  if (configuration.state !== "DESIGNATED" || configuration.conversation?.state !== "FOUND") return null;
  const fact = configuration.conversation;
  if (!nonempty(fact.sessionId) || !decimal(fact.generation) || !decimal(fact.revision) ||
      !nonempty(fact.threadId) || !nonempty(fact.ledgerEpoch) || !decimal(fact.ledgerCursor) ||
      !["COMMITTED", "STOPPED", "RELEASED"].includes(String(fact.claimState)) ||
      typeof fact.runtimeAvailable !== "boolean" || !["IDLE", "RUNNING", "UNKNOWN"].includes(String(fact.turnState))) {
    throw new Error(`Native Secretary FOUND conversation lacks original H/A facts: ${JSON.stringify(fact)}`);
  }
  return { configuration, fact };
}

function parseSecretaryInput(value: unknown, sessionId: string, threadId: string): SecretaryOriginalInput {
  if (!record(value) || !nonempty(value.requestId) || !decimal(value.generation) ||
      !["send", "append-without-turn"].includes(String(value.operation)) ||
      !decimal(value.expectedRevision) ||
      !["VERIFIED", "UNKNOWN", "TOO_LARGE"].includes(String(value.bodyState)) ||
      (value.bodyState === "VERIFIED" ? typeof value.body !== "string" : value.body !== null) ||
      (value.occurredAtMs !== null && !decimal(value.occurredAtMs)) ||
      (value.bodyState === "VERIFIED" && !decimal(value.occurredAtMs)) ||
      !["PREPARED", "UNKNOWN", "RECEIPTED"].includes(String(value.phase)) ||
      (value.receiptStatus !== null && !nonempty(value.receiptStatus)) ||
      (value.receipt !== null && !record(value.receipt)) ||
      (value.receiptState !== undefined &&
        value.receiptState !== "OMITTED_FOR_FRAME" && value.receiptState !== "TOO_LARGE")) {
    throw new Error(`Native Secretary original input is malformed: ${JSON.stringify(value)}`);
  }
  const receipt = value.receipt as Record<string, unknown> | null;
  if (receipt && (receipt.schema !== "gogoke.37.operations.v1" ||
      receipt.family !== "K-SESSION" || receipt.operation !== value.operation ||
      receipt.requestId !== value.requestId || receipt.targetId !== sessionId ||
      !decimal(receipt.previousRevision) || !decimal(receipt.revision) ||
      ((receipt.status === "APPLIED" || receipt.status === "REPLAYED") &&
        receipt.previousRevision !== value.expectedRevision) ||
      receipt.status !== value.receiptStatus || !record(receipt.result))) {
    throw new Error(`Native Secretary original input receipt identity is invalid: ${JSON.stringify(value)}`);
  }
  if (receipt && record(receipt.result) && nonempty(receipt.result.threadId) &&
      receipt.result.threadId !== threadId) {
    throw new Error(`Native Secretary original input receipt has a different thread: ${JSON.stringify(value)}`);
  }
  const turnId = receipt && record(receipt.result) && nonempty(receipt.result.turnId)
    ? receipt.result.turnId : null;
  return { requestId: value.requestId as string, generation: value.generation as string,
    operation: value.operation as "send" | "append-without-turn",
    expectedRevision: value.expectedRevision as string, body: value.body as string | null,
    bodyState: value.bodyState as "VERIFIED" | "UNKNOWN" | "TOO_LARGE",
    occurredAtMs: value.occurredAtMs as string | null,
    phase: value.phase as "PREPARED" | "UNKNOWN" | "RECEIPTED",
    receiptStatus: value.receiptStatus as string | null,
    receipt, receiptState: (value.receiptState as string | undefined) ?? null, turnId };
}

async function readSecretaryInputHistory(configuration: Extract<SecretaryConfiguration, { state: "DESIGNATED" }>,
  fact: NonNullable<Extract<SecretaryConfiguration, { state: "DESIGNATED" }>["conversation"]>):
  Promise<{ items: SecretaryOriginalInput[]; gap: string | null; rowsEnded: boolean }> {
  const items: SecretaryOriginalInput[] = [];
  const seen = new Set<string>();
  const tokens = new Set<string>();
  let continuation: string | null = null;
  for (;;) {
    const reply = await design37UserFrame({ schema: "gogoke.37.owner-configuration.v1",
      command: "secretary-configuration-read", inputHistory: {
        seatId: configuration.seatId, incarnation: configuration.incarnation,
        authorizationGeneration: configuration.generation, sessionId: fact.sessionId,
        hGeneration: fact.generation,
        ...(continuation ? { continuation } : {}),
      } });
    const parsed = parseSecretaryConfiguration(reply);
    if (record(reply) && record(reply.inputHistory) && reply.inputHistory.state === "UNKNOWN" &&
        Array.isArray(reply.inputHistory.items) && reply.inputHistory.items.length === 0 &&
        reply.inputHistory.nextContinuation === null && continuation === null &&
        parsed.state === "DESIGNATED" && parsed.seatId === configuration.seatId &&
        parsed.incarnation === configuration.incarnation &&
        parsed.generation === configuration.generation && parsed.revision === configuration.revision) {
      return { items: [], gap: "宿主无法证明原始用户输入的 E/H/A 来源", rowsEnded: false };
    }
    const returned = confirmedConversation(parsed);
    if (!returned || returned.configuration.seatId !== configuration.seatId ||
        returned.configuration.incarnation !== configuration.incarnation ||
        returned.configuration.generation !== configuration.generation ||
        returned.configuration.revision !== configuration.revision ||
        returned.configuration.instanceId !== configuration.instanceId ||
        returned.configuration.model !== configuration.model ||
        returned.configuration.effort !== configuration.effort ||
        returned.configuration.permissionTier !== configuration.permissionTier ||
        returned.fact.sessionId !== fact.sessionId || returned.fact.generation !== fact.generation ||
        returned.fact.threadId !== fact.threadId || returned.fact.ledgerEpoch !== fact.ledgerEpoch ||
        !record(reply) || !record(reply.inputHistory)) {
      throw new Error(`Native Secretary original input read changed E/H identity: ${JSON.stringify(reply)}`);
    }
    const page = reply.inputHistory;
    if (page.state !== "FOUND" || page.sessionId !== fact.sessionId ||
        !Array.isArray(page.items) ||
        (page.nextContinuation !== null &&
          (typeof page.nextContinuation !== "string" || !/^[0-9a-fA-F]{64}$/.test(page.nextContinuation)))) {
      throw new Error(`Native Secretary original input page is malformed: ${JSON.stringify(page)}`);
    }
    for (const raw of page.items) {
      const item = parseSecretaryInput(raw, fact.sessionId!, fact.threadId!);
      const key = JSON.stringify([item.generation, item.requestId]);
      if (seen.has(key)) throw new Error(`Native Secretary original input repeated: ${key}`);
      seen.add(key);
      items.push(item);
    }
    if (page.nextContinuation === null) break;
    if (page.items.length === 0 || tokens.has(page.nextContinuation)) {
      throw new Error(`Native Secretary original input continuation did not advance: ${page.nextContinuation}`);
    }
    tokens.add(page.nextContinuation);
    continuation = page.nextContinuation;
  }
  const gap = items.some((item) => item.bodyState !== "VERIFIED" ||
      item.phase !== "RECEIPTED" ||
      !["APPLIED", "REPLAYED", "STALE", "DENIED", "CONFLICT", "UNSUPPORTED"]
        .includes(item.receiptStatus ?? "") ||
      item.receipt === null)
    ? "原始用户输入含未验证正文或未结算的 H 记录" : null;
  return { items, gap, rowsEnded: true };
}

type SecretaryTranscript = { sessionId: string; seatId: string; epoch: string;
  cursor: string; target: string; headHint: string; complete: boolean; messages: ConversationItem[];
  ids: Set<string>; groups: Map<string, number>; statuses: string[];
  turnFirstMessage: Map<string, number>;
  vendorUserFacts: SecretaryVendorUserFact[]; vendorGroups: Map<string, number> };

function secretaryEvent(state: SecretaryTranscript, value: unknown, threadId: string): void {
  if (!record(value) || !decimal(value.cursor) || !nonempty(value.sourceEventId) ||
      !nonempty(value.sourceEpoch) || !nonempty(value.sourceCursor) ||
      !nonempty(value.domainId) || !nonempty(value.seatId) || !nonempty(value.sessionId) ||
      !record(value.update)) {
    throw new Error(`Native Secretary ledger event is malformed: ${JSON.stringify(value)}`);
  }
  if (value.sessionId !== state.sessionId || value.seatId !== state.seatId || value.domainId !== "global") return;
  if (state.ids.has(value.sourceEventId)) throw new Error(`Native Secretary source event repeated: ${value.sourceEventId}`);
  state.ids.add(value.sourceEventId);
  const update = value.update;
  if (!nonempty(update.sessionUpdate)) {
    throw new Error(`Native Secretary original update has no kind: ${JSON.stringify(value)}`);
  }
  const meta = record(update._meta) ? update._meta : null;
  if (meta && nonempty(meta.threadId) && meta.threadId !== threadId) {
    throw new Error(`Native Secretary original thread changed: ${JSON.stringify(value)}`);
  }
  if (meta && nonempty(meta.turnId) && !state.turnFirstMessage.has(meta.turnId)) {
    state.turnFirstMessage.set(meta.turnId, state.messages.length);
  }
  const group = meta && nonempty(meta.turnId) && nonempty(meta.itemId)
    ? JSON.stringify([update.sessionUpdate, meta.turnId, meta.itemId]) : null;
  if (update.sessionUpdate === "user_message_chunk" || update.sessionUpdate === "agent_message_chunk") {
    if (!record(update.content) || update.content.type !== "text" || typeof update.content.text !== "string") {
      throw new Error(`Native Secretary text update is malformed: ${JSON.stringify(value)}`);
    }
    if (update.sessionUpdate === "user_message_chunk") {
      const index = group ? state.vendorGroups.get(group) : undefined;
      if (index !== undefined) {
        const prior = state.vendorUserFacts[index];
        state.vendorUserFacts[index] = { ...prior, text: prior.text + update.content.text };
      } else {
        if (group) state.vendorGroups.set(group, state.vendorUserFacts.length);
        state.vendorUserFacts.push({ sourceEventId: value.sourceEventId,
          turnId: meta && nonempty(meta.turnId) ? meta.turnId : null,
          text: update.content.text, matchedOriginal: false });
      }
      return;
    }
    const role = "assistant";
    const index = group ? state.groups.get(group) : undefined;
    const old = index === undefined ? null : state.messages[index];
    if (old && old.kind === "message" && old.role === role) {
      state.messages[index!] = { ...old, text: old.text + update.content.text };
    } else {
      if (group) state.groups.set(group, state.messages.length);
      state.messages.push({ id: value.sourceEventId, kind: "message", role, text: update.content.text });
    }
    return;
  }
  if (update.sessionUpdate === "agent_thought_chunk") {
    if (!record(update.content) || update.content.type !== "text" || typeof update.content.text !== "string") {
      throw new Error(`Native Secretary thought update is malformed: ${JSON.stringify(value)}`);
    }
    const index = group ? state.groups.get(group) : undefined;
    const old = index === undefined ? null : state.messages[index];
    const summary = meta?.codexMethod === "item/reasoning/summaryTextDelta";
    if (old && old.kind === "reasoning") {
      state.messages[index!] = summary
        ? { ...old, summary: old.summary + update.content.text }
        : { ...old, content: old.content + update.content.text };
    } else {
      if (group) state.groups.set(group, state.messages.length);
      state.messages.push({ id: value.sourceEventId, kind: "reasoning",
        summary: summary ? update.content.text : "", content: summary ? "" : update.content.text });
    }
    return;
  }
  if (update.sessionUpdate === "tool_call" || update.sessionUpdate === "tool_call_update") {
    const toolKey = meta && nonempty(meta.turnId) && nonempty(meta.itemId)
      ? JSON.stringify(["tool", meta.turnId, meta.itemId]) : null;
    const index = toolKey ? state.groups.get(toolKey) : undefined;
    const old = index === undefined ? null : state.messages[index];
    const detail = update.rawOutput === undefined ? "" :
      typeof update.rawOutput === "string" ? update.rawOutput : JSON.stringify(update.rawOutput);
    const status = typeof update.status === "string" ? update.status : undefined;
    const method = typeof meta?.codexMethod === "string" ? meta.codexMethod : null;
    const delta = method === "item/commandExecution/outputDelta" || method === "item/fileChange/outputDelta";
    const progress = method === "item/mcpToolCall/progress";
    if (progress && detail) state.statuses.push(`工具进度：${detail}`);
    if (old && old.kind === "tool") {
      state.messages[index!] = { ...old,
        detail: progress || update.rawOutput === undefined ? old.detail : delta ? old.detail + detail : detail,
        ...(status ? { status } : {}) };
    } else {
      if (toolKey) state.groups.set(toolKey, state.messages.length);
      state.messages.push({ id: value.sourceEventId, kind: "tool",
        toolType: typeof update.kind === "string" ? update.kind : "native",
        title: typeof update.title === "string" ? update.title : update.sessionUpdate,
        detail: progress ? "" : detail, ...(status ? { status } : {}) });
    }
    return;
  }
  if (update.sessionUpdate === "session_info_update") {
    const source = meta ? { sourceEventId: value.sourceEventId,
      provider: meta.provider, method: meta.codexMethod ?? meta.providerMethod,
      threadId: meta.threadId, turnId: meta.turnId, turnStatus: meta.turnStatus,
      threadStatus: meta.threadStatus, codexError: meta.codexError,
      codexWillRetry: meta.codexWillRetry } : null;
    if (source && [meta?.turnStatus, meta?.threadStatus, meta?.codexError,
      meta?.codexWillRetry].some((part) => part !== undefined)) {
      state.statuses.push(JSON.stringify(source));
    }
    return;
  }
  // Configuration and usage updates are source metadata, not conversation tools.
}

/** Actual E configuration and E routines; the optional conversation is H's fact only. */
export function createDesign37SecretarySource(): {
  source: SecretarySource;
  readSnapshot: () => Promise<{ configuration: SecretaryConfiguration; page: SecretaryPage }>;
  readConversation: (configuration: SecretaryConfiguration) => Promise<SecretaryConversation | null>;
  invalidateTranscript: () => void;
  writeFacts: () => SecretaryWriteFact[];
  send: (binding: SecretaryBinding, body: string) => Promise<SecretaryWriteFact>;
  retrySend: (requestId: string) => Promise<SecretaryWriteFact>;
  stop: (binding: SecretaryBinding) => Promise<SecretaryWriteFact>;
  retryStop: (requestId: string) => Promise<SecretaryWriteFact>;
} {
  const readSnapshot = async () => {
    const configuration = await readSecretaryConfiguration();
    const settings: SecretaryPage["settings"] & {
      globalParallelCap?: number; globalEffectiveLimit?: number;
    } = await readSecretarySettingChoices();
    if (configuration.globalParallelCap) {
      settings.globalParallelCap = Number(configuration.globalParallelCap);
    }
    if (configuration.globalEffectiveLimit) {
      settings.globalEffectiveLimit = Number(configuration.globalEffectiveLimit);
    }
    if (configuration.state !== "DESIGNATED") {
      return { configuration, page: {
        entry: configuration.state === "UNSET" ? { kind: "unset" as const } :
          { kind: "down" as const, reason: "宿主已撤销秘书长席位" },
        routines: [], settings,
      } };
    }
    settings.instanceId = configuration.instanceId ?? undefined;
    settings.model = configuration.model ?? undefined;
    settings.effort = configuration.effort ?? undefined;
    settings.permission = configuration.permissionTier ?? undefined;
    // These are only the E-selected values in a disabled form, not a claim
    // that other efforts or permission tiers are available on this instance.
    if (configuration.effort) settings.efforts = [configuration.effort];
    if (configuration.permissionTier) settings.permissions = [configuration.permissionTier];
    if (!configuration.instanceId || !configuration.model || !configuration.effort ||
        !configuration.permissionTier || configuration.seatState === "RECLAIMED") {
      return { configuration, page: { entry: { kind: "unset" as const }, routines: [], settings } };
    }
    const management = await design37UserFrame({
      schema: "gogoke.37.owner-configuration.v1", command: "instance-management-read",
    });
    if (!record(management) || management.schema !== "gogoke.37.instance-management.v1" ||
        !Array.isArray(management.profiles)) {
      throw new Error(`Native Secretary instance management read is malformed: ${JSON.stringify(management)}`);
    }
    const matching = management.profiles.filter((profile: unknown) =>
      record(profile) && profile.instanceId === configuration.instanceId);
    if (matching.length > 1) {
      throw new Error(`Native Secretary instance profile is duplicated: ${configuration.instanceId}`);
    }
    const profile: unknown = matching[0];
    if (profile === undefined) {
      settings.cannot = [`宿主没有返回当前实例 ${configuration.instanceId} 的资料`];
    } else if (!record(profile) ||
        (profile.name !== null && profile.name !== undefined && typeof profile.name !== "string") ||
        (profile.models !== undefined && (!Array.isArray(profile.models) ||
          !profile.models.every(nonempty))) ||
        (profile.modelsSource !== null && profile.modelsSource !== undefined &&
          typeof profile.modelsSource !== "string") ||
        (profile.modelsObservedAt !== null && profile.modelsObservedAt !== undefined &&
          typeof profile.modelsObservedAt !== "string")) {
      throw new Error(`Native Secretary current instance profile is malformed: ${JSON.stringify(profile)}`);
    } else if (!nonempty(profile.name) || !Array.isArray(profile.models) ||
        !nonempty(profile.modelsSource) || !nonempty(profile.modelsObservedAt)) {
      settings.cannot = [`宿主没有给出当前实例 ${configuration.instanceId} 的名称或已验证模型来源`];
    } else if (!profile.models.includes(configuration.model)) {
      settings.cannot = [`当前配置模型 ${configuration.model} 不在实例 ${configuration.instanceId} 的已验证模型中`];
    } else {
      settings.instances = [{ id: configuration.instanceId,
        name: profile.name, models: profile.models }];
    }
    const rows = await readSecretaryRoutines();
    const routines = rows.filter((row) => {
      if (row.seatId !== configuration.seatId || row.incarnation !== configuration.incarnation) {
        throw new Error("Native Secretary routine belongs to a different designation.");
      }
      return row.state !== "DELETED";
    }).map(routinePageRow);
    const confirmed = await readSecretaryConfiguration();
    if (confirmed.state !== "DESIGNATED" ||
        confirmed.seatId !== configuration.seatId ||
        confirmed.incarnation !== configuration.incarnation ||
        confirmed.generation !== configuration.generation ||
        confirmed.revision !== configuration.revision) {
      throw new Error("Native Secretary snapshot changed while it was being read.");
    }
    // Turn and ledger progress may change during a read without changing E's
    // configuration. Use the last host observation instead of freezing it.
    const conversation = confirmed.conversation;
    const runnable = conversation?.state === "FOUND" &&
      conversation.historical === false && conversation.runtimeAvailable === true &&
      conversation.stoppedFact !== true;
    const entry: SecretaryPage["entry"] = runnable && conversation.turnState === "IDLE"
      ? { kind: "quiet" }
      : runnable && conversation.turnState === "RUNNING" ? { kind: "working" } :
        { kind: "down", reason: conversation?.state === "CONFLICT" ? "宿主报告秘书长会话冲突" :
          conversation?.state === "UNKNOWN" ? "宿主无法确认秘书长会话" :
          conversation?.state === "NONE" ? "宿主未找到秘书长会话" :
          conversation?.historical === true ? "原会话已结束，可查看历史" :
          conversation?.stoppedFact === true ? "宿主报告秘书长会话已停止" :
          "宿主未报告可用的秘书长会话轮次状态" };
    return { configuration: confirmed, page: { entry, routines, settings } };
  };
  const changeRoutine = async (id: string, command: "secretary-routine-pause" | "secretary-routine-delete") => {
    const configuration = await readSecretaryConfiguration();
    if (configuration.state !== "DESIGNATED") throw new Error("Secretary designation changed.");
    const rows = await readSecretaryRoutines();
    const confirmed = await readSecretaryConfiguration();
    if (confirmed.state !== "DESIGNATED" || confirmed.seatId !== configuration.seatId ||
        confirmed.incarnation !== configuration.incarnation ||
        confirmed.generation !== configuration.generation ||
        confirmed.revision !== configuration.revision) {
      throw new Error("Secretary designation changed before routine change.");
    }
    const row = rows.find((item) => item.routineId === id);
    if (!row || row.seatId !== configuration.seatId ||
        row.incarnation !== configuration.incarnation || row.state === "DELETED" ||
        (command === "secretary-routine-pause" &&
          row.state !== "ACTIVE" && row.state !== "WAITING_NEXT")) {
      throw new Error("Secretary routine is no longer eligible for this change.");
    }
    const reply = await design37UserFrame({ schema: "gogoke.37.owner-configuration.v1",
      command, routineId: id, requestId: `ui-${crypto.randomUUID()}`,
      expectedRevision: row.revision });
    if (!record(reply) || reply.schema !== "gogoke.37.secretary-routines.v1" ||
        reply.command !== command || reply.routineId !== id ||
        !["APPLIED", "REPLAYED"].includes(String(reply.status)) || !decimal(reply.revision)) {
      throw new Error(`Native Secretary routine change failed: ${JSON.stringify(reply)}`);
    }
  };
  let transcript: SecretaryTranscript | null = null;
  type OriginalWrite = { binding: SecretaryBinding; frame: string; requestId: string;
    expectedRevision: string; operation: "send" | "stop"; body: string | null;
    status: "UNKNOWN" | "ACCEPTED" | "REJECTED"; receipt: unknown | null; reason: string | null;
    inputVerified: boolean };
  const sends: OriginalWrite[] = [];
  const stops: OriginalWrite[] = [];
  const factOf = (write: OriginalWrite): SecretaryWriteFact => ({
    operation: write.operation, requestId: write.requestId, binding: write.binding,
    sessionId: write.binding.sessionId, seatId: write.binding.seatId,
    hGeneration: write.binding.hGeneration, body: write.body,
    status: write.status, receipt: write.receipt, reason: write.reason,
    inputVerified: write.inputVerified });
  const writeFacts = () => [...sends, ...stops]
    .map(factOf);
  const rejectWrite = (write: OriginalWrite, receipt: unknown) => {
    write.status = "REJECTED";
    write.receipt = receipt;
    write.reason = `Native Secretary ${write.operation} was refused without execution: ${JSON.stringify(receipt)}`;
    return write.reason;
  };
  const observeOriginalInputs = (values: unknown[]) => {
    for (const raw of values) {
      if (!record(raw) || !nonempty(raw.requestId)) continue;
      const send = sends.find((item) => item.requestId === raw.requestId && item.status === "UNKNOWN");
      if (!send) continue;
      if (raw.expectedRevision !== send.expectedRevision || !nonempty(raw.phase) ||
          (raw.receipt !== null && !record(raw.receipt))) {
        throw new Error(`Native Secretary original input fact is invalid: ${JSON.stringify(raw)}`);
      }
      if (raw.receipt === null) continue;
      const result = originalWriteReceipt(raw.receipt, "send", send.requestId,
        send.binding.sessionId, send.expectedRevision);
      if (result === "ACCEPTED") {
        send.status = "ACCEPTED";
        send.receipt = raw.receipt;
        send.reason = null;
      } else if (result === "REJECTED") {
        rejectWrite(send, raw.receipt);
      }
    }
  };
  const invalidateTranscript = () => { transcript = null; };
  const readConversation = async (configuration: SecretaryConfiguration): Promise<SecretaryConversation | null> => {
    const original = confirmedConversation(configuration);
    if (!original) { transcript = null; return null; }
    const { fact } = original;
    let sourceGap: string | null = "原始输出和待处理问题未由当前运行实例复核";
    if (fact.historical === false && fact.runtimeAvailable === true && fact.claimState === "COMMITTED") {
      const output = (await secretaryOperation("K-SESSION", "output-stream", fact.sessionId!, fact.revision!,
        { generation: fact.generation, afterCursor: fact.ledgerCursor })).result;
      if (output.generation !== fact.generation || !decimal(output.cursor) ||
          (output.sourceError !== null && output.sourceError !== undefined)) {
        throw new Error(`Native Secretary output source is unresolved: ${JSON.stringify(output)}`);
      }
      if (!decimal(output.unresolvedRawFrames) || !Array.isArray(output.nativeCardRefs) ||
          typeof output.nativeCardRefsIncomplete !== "boolean" ||
          !Array.isArray(output.nativeInputReceipts)) {
        throw new Error(`Native Secretary output completeness facts are malformed: ${JSON.stringify(output)}`);
      }
      observeOriginalInputs(output.nativeInputReceipts);
      sourceGap = output.unresolvedRawFrames !== "0" ?
        `宿主仍有 ${output.unresolvedRawFrames} 个未归档原始输出帧` :
        output.nativeCardRefsIncomplete || output.nativeCardRefs.length > 0 ?
          `宿主仍有 ${output.nativeCardRefs.length} 个待处理问题引用${output.nativeCardRefsIncomplete ? "，列表未完整" : ""}` : null;
    }
    const latest = confirmedConversation(await readSecretaryConfiguration());
    if (!latest || latest.configuration.seatId !== original.configuration.seatId ||
        latest.configuration.incarnation !== original.configuration.incarnation ||
        latest.configuration.generation !== original.configuration.generation ||
        latest.configuration.revision !== original.configuration.revision ||
        latest.fact.sessionId !== fact.sessionId || latest.fact.threadId !== fact.threadId ||
        latest.fact.ledgerEpoch !== fact.ledgerEpoch) {
      throw new Error("Native Secretary conversation identity changed during transcript read.");
    }
    const head = latest.fact.ledgerCursor!;
    let originalInputs: SecretaryOriginalInput[] = [];
    let inputGap: string | null = null;
    let inputRowsEnded = false;
    try {
      const read = await readSecretaryInputHistory(latest.configuration, latest.fact);
      originalInputs = read.items;
      inputGap = read.gap;
      inputRowsEnded = read.rowsEnded;
    } catch (cause) {
      inputGap = `宿主原始用户输入读取不可用或无法核实：${cause instanceof Error ? cause.message : String(cause)}`;
      invalidateTranscript();
    }
    const prior = transcript && transcript.sessionId === fact.sessionId &&
      transcript.seatId === original.configuration.seatId &&
      transcript.epoch === fact.ledgerEpoch && BigInt(transcript.cursor) <= BigInt(head)
      ? transcript : null;
    const state: SecretaryTranscript = prior ? {
      ...prior, messages: [...prior.messages], ids: new Set(prior.ids),
      groups: new Map(prior.groups), statuses: [...prior.statuses],
      turnFirstMessage: new Map(prior.turnFirstMessage),
      vendorUserFacts: [...prior.vendorUserFacts], vendorGroups: new Map(prior.vendorGroups),
      target: prior.complete ? head : prior.target, complete: false,
    } : { sessionId: fact.sessionId!, seatId: original.configuration.seatId,
      epoch: fact.ledgerEpoch!, cursor: "0", target: head, headHint: head, complete: false,
      messages: [], ids: new Set(), groups: new Map(), statuses: [],
      turnFirstMessage: new Map(), vendorUserFacts: [], vendorGroups: new Map() };
    let pageGap: string | null = null;
    while (BigInt(state.cursor) < BigInt(state.target)) {
      const current = confirmedConversation(await readSecretaryConfiguration());
      if (!current || current.configuration.seatId !== original.configuration.seatId ||
          current.configuration.incarnation !== original.configuration.incarnation ||
          current.configuration.generation !== original.configuration.generation ||
          current.configuration.revision !== original.configuration.revision ||
          current.fact.sessionId !== fact.sessionId || current.fact.threadId !== fact.threadId ||
          current.fact.ledgerEpoch !== fact.ledgerEpoch) {
        throw new Error("Native Secretary identity changed before ledger page read.");
      }
      const currentHead = current.fact.ledgerCursor!;
      if (BigInt(currentHead) < BigInt(state.headHint)) {
        throw new Error("Native Secretary ledger head regressed below the original reply.");
      }
      const requestId = `secretary_${crypto.randomUUID()}`;
      const reply = await design37UserFrame({ schema: "gogoke.37.operations.v1",
        family: "K-LEDGER", operation: "scoped-query", requestId, targetId: "ledger",
        domainId: "global", expectedRevision: currentHead,
        payload: { readerSessionId: fact.sessionId, scope: "GLOBAL", epoch: fact.ledgerEpoch,
          afterCursor: state.cursor } });
      if (!record(reply) || reply.schema !== "gogoke.37.operations.v1" ||
          reply.family !== "K-LEDGER" || reply.operation !== "scoped-query" ||
          reply.requestId !== requestId || reply.targetId !== "ledger") {
        throw new Error(`Native Secretary ledger reply identity is invalid: ${JSON.stringify(reply)}`);
      }
      if (reply.status === "STALE") {
        if (!decimal(reply.revision) || BigInt(reply.revision) < BigInt(currentHead)) {
          throw new Error(`Native Secretary STALE ledger head is invalid: ${JSON.stringify(reply)}`);
        }
        state.headHint = reply.revision;
        pageGap = `全局账本在读取时前进至 ${reply.revision}；已保留至 ${state.cursor}，下次继续`;
        break;
      }
      if (reply.status !== "APPLIED" && reply.status !== "REPLAYED") {
        throw new Error(`Native Secretary ledger read was refused: ${JSON.stringify(reply)}`);
      }
      const result = reply.result;
      if (!record(result) || result.epoch !== fact.ledgerEpoch || !decimal(result.cursor) ||
          !decimal(result.highWaterCursor) || !Array.isArray(result.events) ||
          result.highWaterCursor !== currentHead || BigInt(result.cursor) <= BigInt(state.cursor) ||
          BigInt(result.cursor) > BigInt(currentHead)) {
        throw new Error(`Native Secretary ledger page is incomplete: ${JSON.stringify(reply)}`);
      }
      const before = BigInt(state.cursor);
      let eventCursor = before;
      for (const event of result.events) {
        if (!record(event) || !decimal(event.cursor) || BigInt(event.cursor) <= eventCursor ||
            BigInt(event.cursor) > BigInt(result.cursor)) {
          throw new Error(`Native Secretary ledger event order is invalid: ${JSON.stringify(event)}`);
        }
        eventCursor = BigInt(event.cursor);
        if (eventCursor <= BigInt(state.target)) secretaryEvent(state, event, fact.threadId!);
      }
      state.cursor = BigInt(result.cursor) > BigInt(state.target) ? state.target : result.cursor;
      state.headHint = result.highWaterCursor;
      transcript = { ...state, messages: [...state.messages], ids: new Set(state.ids),
        groups: new Map(state.groups), statuses: [...state.statuses],
        turnFirstMessage: new Map(state.turnFirstMessage),
        vendorUserFacts: [...state.vendorUserFacts], vendorGroups: new Map(state.vendorGroups) };
    }
    state.complete = !pageGap && state.cursor === state.target;
    transcript = state;
    let verifiedSend: { requestId: string; body: string; hGeneration: string } | null = null;
    for (const send of sends) {
      if (send.binding.sessionId !== fact.sessionId) continue;
      const matching = originalInputs.find((item) => item.requestId === send.requestId &&
        item.generation === send.binding.hGeneration);
      if (matching) {
        if (matching.expectedRevision !== send.expectedRevision ||
            (matching.bodyState === "VERIFIED" && matching.body !== send.body)) {
          throw new Error("Native Secretary original USER input disagrees with the retained request.");
        }
        if (matching.receipt && matching.phase === "RECEIPTED" &&
            (matching.receiptStatus === "APPLIED" || matching.receiptStatus === "REPLAYED")) {
          const outcome = originalWriteReceipt(matching.receipt, "send", send.requestId,
            send.binding.sessionId, send.expectedRevision);
          if (outcome === "ACCEPTED") {
            send.status = "ACCEPTED";
            send.receipt = matching.receipt;
            send.reason = null;
            if (matching.bodyState === "VERIFIED" && matching.body !== null) {
              send.inputVerified = true;
              verifiedSend = { requestId: send.requestId, body: matching.body,
                hGeneration: matching.generation };
            }
          }
        }
      }
    }
    const inserts = new Map<number, ConversationItem[]>();
    for (const input of originalInputs) {
      if (input.operation !== "send" || input.bodyState !== "VERIFIED" || input.body === null ||
          input.phase !== "RECEIPTED" || !input.receipt ||
          !["APPLIED", "REPLAYED"].includes(input.receiptStatus ?? "") ||
          !input.turnId) continue;
      const at = state.turnFirstMessage.get(input.turnId);
      if (at === undefined) continue;
      const entries = inserts.get(at) ?? [];
      entries.push({ id: `h-user-${input.generation}-${input.requestId}`,
        kind: "message", role: "user", text: input.body });
      inserts.set(at, entries);
    }
    const messages: ConversationItem[] = [];
    for (let index = 0; index <= state.messages.length; index += 1) {
      messages.push(...(inserts.get(index) ?? []));
      if (index < state.messages.length) messages.push(state.messages[index]);
    }
    const verifiedInputs = originalInputs.filter((input) => input.operation === "send" &&
      input.bodyState === "VERIFIED" && input.body !== null &&
      input.phase === "RECEIPTED" && input.receipt &&
      ["APPLIED", "REPLAYED"].includes(input.receiptStatus ?? ""));
    const vendorUserFacts = state.vendorUserFacts.map((fact) => ({ ...fact,
      matchedOriginal: fact.turnId !== null && verifiedInputs.some((input) =>
        input.turnId === fact.turnId && input.body === fact.text) }));
    const historyGap = [pageGap, sourceGap, inputGap]
      .filter((value): value is string => Boolean(value)).join("；") || null;
    const binding = bindingOf(latest.configuration, latest.fact);
    const writer = !pageGap && state.complete && binding && latest.fact.historical === false &&
      latest.fact.claimState === "COMMITTED" && latest.fact.stoppedFact !== true &&
      latest.fact.runtimeAvailable === true &&
      (latest.fact.turnState === "IDLE" || latest.fact.turnState === "RUNNING")
      ? { binding, instanceId: binding.instanceId, model: binding.model,
          effort: binding.effort, permissionTier: binding.permissionTier,
          canSend: !sourceGap && !inputGap && latest.fact.turnState === "IDLE",
          canStop: latest.fact.turnState === "RUNNING" } : null;
    return { sessionId: fact.sessionId!, threadId: fact.threadId!,
      seatId: original.configuration.seatId, generation: latest.fact.generation!,
      revision: latest.fact.revision!, turnState: latest.fact.turnState!,
      historical: latest.fact.historical!, runtimeAvailable: latest.fact.runtimeAvailable!,
      messages, statuses: state.statuses, historyGap, writer,
      inputs: originalInputs, inputRowsEnded, vendorUserFacts, verifiedSend };
  };
  const currentWriter = async (binding: SecretaryBinding, turnState: "IDLE" | "RUNNING") => {
    const selected = confirmedConversation(await readSecretaryConfiguration());
    const actual = selected && bindingOf(selected.configuration, selected.fact);
    if (!selected || !actual || !sameSecretaryBinding(binding, actual) ||
        selected.fact.historical !== false || selected.fact.claimState !== "COMMITTED" ||
        selected.fact.stoppedFact === true || selected.fact.runtimeAvailable !== true ||
        selected.fact.turnState !== turnState) {
      throw new Error("Native Secretary writer differs from the conversation the User saw.");
    }
    return selected;
  };
  const runOriginalWrite = async (write: OriginalWrite): Promise<SecretaryWriteFact> => {
    try {
      const reply = await secretaryRawFrame(write.frame);
      const outcome = originalWriteReceipt(reply, write.operation, write.requestId,
        write.binding.sessionId, write.expectedRevision);
      if (outcome === "REJECTED") {
        throw new Error(rejectWrite(write, reply));
      }
      if (outcome === "ACCEPTED") {
        write.status = "ACCEPTED";
        write.receipt = reply;
        write.reason = null;
        return factOf(write);
      }
      write.reason = `Native Secretary ${write.operation} outcome is UNKNOWN: ${JSON.stringify(reply)}`;
      throw new Error(write.reason);
    } catch (cause) {
      if (write.status !== "ACCEPTED" && !write.reason) {
        write.reason = cause instanceof Error ? cause.message : String(cause);
      }
      throw cause;
    }
  };
  const newOriginalWrite = (binding: SecretaryBinding, operation: "send" | "stop",
    expectedRevision: string, body: string | null): OriginalWrite => {
    const requestId = `secretary_${crypto.randomUUID()}`;
    const payload = operation === "send" ? { body, generation: binding.hGeneration }
      : { seatId: binding.seatId, generation: binding.hGeneration };
    const frame = JSON.stringify({ schema: "gogoke.37.operations.v1", family: "K-SESSION",
      operation, requestId, targetId: binding.sessionId, domainId: "global",
      expectedRevision, payload });
    return { binding, operation, requestId, expectedRevision, body,
    frame, status: "UNKNOWN", receipt: null, reason: null, inputVerified: false };
  };
  const send = async (binding: SecretaryBinding, body: string): Promise<SecretaryWriteFact> => {
    if (!body.trim()) throw new Error("Secretary message is empty.");
    if (sends.some((item) => item.status === "UNKNOWN" &&
        sameSecretaryWriter(item.binding, binding))) {
      throw new Error("Previous Secretary send for this H writer is unconfirmed. Recheck its original request.");
    }
    if (sends.some((item) => item.status === "ACCEPTED" && !item.inputVerified &&
        item.body === body && sameSecretaryWriter(item.binding, binding))) {
      throw new Error("This exact Secretary message has an accepted original send receipt; edit the draft before a new send.");
    }
    await currentWriter(binding, "IDLE");
    const write = newOriginalWrite(binding, "send", binding.hRevision, body);
    sends.push(write);
    return runOriginalWrite(write);
  };
  const retrySend = async (requestId: string): Promise<SecretaryWriteFact> => {
    const original = sends.find((item) => item.requestId === requestId && item.status === "UNKNOWN");
    if (!original) throw new Error("There is no unconfirmed original Secretary send to recheck.");
    return runOriginalWrite(original);
  };
  const stop = async (binding: SecretaryBinding): Promise<SecretaryWriteFact> => {
    const original = stops.find((item) => item.status === "UNKNOWN" &&
      sameSecretaryWriter(item.binding, binding));
    if (original) return runOriginalWrite(original);
    await currentWriter(binding, "RUNNING");
    const write = newOriginalWrite(binding, "stop", binding.hRevision, null);
    stops.push(write);
    return runOriginalWrite(write);
  };
  const retryStop = async (requestId: string): Promise<SecretaryWriteFact> => {
    const original = stops.find((item) => item.requestId === requestId && item.status === "UNKNOWN");
    if (!original) {
      throw new Error("There is no unconfirmed original Secretary stop to recheck.");
    }
    return runOriginalWrite(original);
  };
  return { readSnapshot, readConversation, invalidateTranscript, writeFacts,
    send, retrySend, stop, retryStop,
    source: { read: async () => (await readSnapshot()).page,
    actions: { pauseRoutine: (id) => changeRoutine(id, "secretary-routine-pause"),
      deleteRoutine: (id) => changeRoutine(id, "secretary-routine-delete") } } };
}

/** An existing USER frame; native ingress authenticates and validates its family. */
export async function design37UserFrame(frame: object): Promise<unknown> {
  const raw = await invoke<string>("gogoke_design37_user_operation", { frame: JSON.stringify(frame) });
  if (typeof raw !== "string") throw new Error("Native USER reply is not a JSON frame.");
  return JSON.parse(raw) as unknown;
}

/** Existing native USER ingress. A model call never goes through this bridge. */
export async function design37UserConfiguration<T>(
  command: string,
  fields: Record<string, unknown> = {},
): Promise<T> {
  if ("schema" in fields || "command" in fields) {
    throw new Error("Design 37 configuration identity cannot be overridden.");
  }
  const frame = JSON.stringify({
    schema: "gogoke.37.owner-configuration.v1",
    command,
    ...fields,
  });
  const raw = await invoke<string>("gogoke_design37_user_operation", { frame });
  if (typeof raw !== "string") throw new Error("Native USER reply is not a JSON frame.");
  const result: unknown = JSON.parse(raw);
  if (result && typeof result === "object" && "status" in result &&
      !["APPLIED", "REPLAYED"].includes(String(result.status))) {
    throw new Error(`Native USER configuration failed: ${raw}`);
  }
  return result as T;
}

export type Design37ProjectPolicyHead =
  | { state: "ABSENT" }
  | { state: "PRESENT"; revision: string; currentStage: string | null };

/** Reads the original Owner policy without initializing a stage or granting calls. */
export async function readDesign37ProjectPolicyHead(domainId: string): Promise<Design37ProjectPolicyHead> {
  const reply: unknown = await design37UserConfiguration("policy-head-read", { domainId });
  if (!record(reply) || reply.schema !== "gogoke.37.project-policy-head.v1" || reply.domainId !== domainId) {
    throw new Error(`Native project policy head identity is invalid: ${JSON.stringify(reply)}`);
  }
  if (reply.state === "ABSENT" && !Object.hasOwn(reply, "revision") && !Object.hasOwn(reply, "currentStage")) {
    return { state: "ABSENT" };
  }
  if (reply.state === "PRESENT" && decimal(reply.revision) && reply.revision !== "0" &&
      (reply.currentStage === null || nonempty(reply.currentStage))) {
    return { state: "PRESENT", revision: reply.revision, currentStage: reply.currentStage };
  }
  throw new Error(`Native project policy head is malformed: ${JSON.stringify(reply)}`);
}

/** Called only by an explicit USER create/configure action, never by a read or model. */
export async function initializeDesign37ProjectPolicyMetadata(domainId: string): Promise<void> {
  const head = await readDesign37ProjectPolicyHead(domainId);
  if (head.state === "PRESENT") return;
  const requestId = `ui-${crypto.randomUUID()}`;
  const reply = await design37UserFrame({
    schema: "gogoke.37.owner-configuration.v1", command: "policy-metadata-initialize",
    domainId, requestId, expectedRevision: "0",
  });
  if (!record(reply) || reply.schema !== "gogoke.37.owner-configuration.v1" ||
      reply.command !== "policy-metadata-initialize" || reply.requestId !== requestId ||
      !((reply.status === "APPLIED" || reply.status === "REPLAYED") && reply.revision === "1" ||
        reply.status === "CONFLICT" && reply.reason === "Conflict")) {
    throw new Error(`Native USER policy metadata initialization failed: ${JSON.stringify(reply)}`);
  }
  // A proven concurrent initializer only permits a fresh read, never a write retry.
  const initialized = await readDesign37ProjectPolicyHead(domainId);
  if (initialized.state !== "PRESENT") {
    throw new Error("Native USER policy metadata is still absent after initialization.");
  }
}

/** An explicit USER choice; never supplies a stage name on the user's behalf. */
export async function setDesign37ProjectInitialStage(domainId: string, stage: string): Promise<void> {
  if (!nonempty(stage)) throw new Error("A user-defined initial stage is required.");
  const head = await readDesign37ProjectPolicyHead(domainId);
  if (head.state !== "PRESENT" || head.currentStage !== null) {
    throw new Error("Native project policy is absent or already has a stage.");
  }
  const requestId = `ui-${crypto.randomUUID()}`;
  const reply = await design37UserFrame({
    schema: "gogoke.37.owner-configuration.v1", command: "policy-stage-set-initial",
    domainId, requestId, stage, expectedRevision: head.revision,
  });
  const expectedRevision = (BigInt(head.revision) + 1n).toString();
  if (!record(reply) || reply.schema !== "gogoke.37.owner-configuration.v1" ||
      reply.command !== "policy-stage-set-initial" || reply.requestId !== requestId ||
      !(reply.status === "APPLIED" || reply.status === "REPLAYED") || reply.revision !== expectedRevision) {
    throw new Error(`Native USER initial stage configuration failed: ${JSON.stringify(reply)}`);
  }
}

type NativeSeatPageRow = {
  id: string;
  isLead?: boolean;
  instance: { id: string };
  allowed: { tune: boolean; changeInstance: boolean; remove: boolean };
  _revision: string;
  _incarnation: string;
  _settings: Record<string, unknown>;
};
type NativeSeatTune = { instanceId: string; model?: string; effort: string; permission: string };

/** Source shape is the existing G SeatsSource; the generic keeps this shared
 * bridge independent of a particular panel. Native tokens are never labels. */
export function createDesign37SeatsSource<Page>(domainId: string | null) {
  const read = async (): Promise<Page | null> => domainId
    ? design37UserConfiguration<Page | null>("seats-page-read", { domainId })
    : null;
  const current = async (id: string): Promise<NativeSeatPageRow> => {
    const page = await read() as (Page & { seats: NativeSeatPageRow[] }) | null;
    const row = page?.seats.find((seat) => seat.id === id);
    if (!row || typeof row._revision !== "string" || typeof row._incarnation !== "string") {
      throw new Error("Native seat mutation identity is unavailable.");
    }
    return row;
  };
  const operation = async (operation: string, targetId: string, expectedRevision: string,
    payload: Record<string, unknown>): Promise<string> => {
    if (!domainId) throw new Error("No project is selected.");
    const frame = JSON.stringify({ schema: "gogoke.37.operations.v1", family: "K-SEAT",
      operation, requestId: `ui-${crypto.randomUUID()}`, targetId, domainId, expectedRevision, payload });
    const raw = await invoke<string>("gogoke_design37_user_operation", { frame });
    const reply = JSON.parse(raw) as { status?: string; revision?: string };
    if (!["APPLIED", "REPLAYED"].includes(reply.status ?? "") || typeof reply.revision !== "string") {
      throw new Error(`Native seat operation failed: ${raw}`);
    }
    return reply.revision;
  };
  const tune = async (id: string, input: NativeSeatTune): Promise<void> => {
    let row = await current(id);
    if (!row.allowed.tune) throw new Error("Native host currently refuses tuning this seat.");
    const changingInstance = input.instanceId !== row.instance.id;
    if (changingInstance && !row.allowed.changeInstance) {
      throw new Error("Native host currently refuses changing this instance.");
    }
    if (changingInstance && (input.model === undefined || !input.model || !input.effort)) {
      throw new Error("The new instance has no verified selected model and effort; the original binding is unchanged.");
    }
    if (input.model !== undefined && input.model && input.effort) {
      // One native transaction replaces binding and complete configuration.
      // The same route can repair an earlier partial change on this instance.
      await operation("change-instance", id, row._revision, {
        instanceId: input.instanceId, model: input.model,
        effort: input.effort, permissionTier: input.permission,
      });
      return;
    }
    const fields: Array<[string, unknown]> = [["permissionTier", input.permission]];
    if (input.effort) fields.unshift([
      "reasoningEffort" in row._settings && !("effort" in row._settings)
        ? "reasoningEffort" : "effort",
      input.effort,
    ]);
    if (input.model !== undefined) fields.unshift(["model", input.model]);
    for (const [setting, value] of fields) {
      if (row._settings[setting] === value) continue;
      await operation("tune", id, row._revision, { setting, value });
      row = await current(id);
    }
  };
  return {
    read,
    actions: {
      tune,
      remove: async (id: string): Promise<void> => {
        const row = await current(id);
        if (!row.allowed.remove || row.isLead) throw new Error("Native host refuses reclaiming this seat.");
        await operation("reclaim", id, row._revision, {});
      },
      create: async (input: NativeSeatTune & { name: string; template: string }): Promise<void> => {
        if (!input.model || !input.effort) {
          throw new Error("A new seat requires a verified selected model and effort before creation.");
        }
        const id = `seat-${crypto.randomUUID()}`;
        const revision = await operation("create-from-template", id, "0", {
          layer: "USER", templateId: input.template,
        });
        await operation("bind-instance", id, revision, { instanceId: input.instanceId });
        const row = await current(id);
        await design37UserConfiguration("seat-rename", {
          domainId, seatId: id, incarnation: row._incarnation, name: input.name,
        });
        await tune(id, input);
      },
      setRange: async (input: { instanceIds: string[]; maxPermission: string; maxConcurrent: number }): Promise<void> => {
        const page = await read() as (Page & { seats: NativeSeatPageRow[] }) | null;
        const lead = page?.seats.find((seat) => seat.isLead);
        if (!lead || !lead.allowed.tune) throw new Error("The native project lead cannot be configured now.");
        const scope = lead._settings.orchestrationScope;
        if (!scope || typeof scope !== "object" || Array.isArray(scope)) {
          throw new Error("Native orchestration scope is unavailable.");
        }
        if (!domainId) throw new Error("No project is selected.");
        await initializeDesign37ProjectPolicyMetadata(domainId);
        await operation("set-orchestration-bounds", lead.id, lead._revision, {
          instanceIds: input.instanceIds,
          models: (scope as Record<string, unknown>).models,
          reasoningEfforts: (scope as Record<string, unknown>).reasoningEfforts,
          maxPermissionTier: input.maxPermission,
          maxConcurrent: input.maxConcurrent,
        });
      },
    },
  };
}
import { open, save } from "@tauri-apps/plugin-dialog";
import type { Options as NotificationOptions } from "@tauri-apps/plugin-notification";
import type {
  AppSettings,
  CodexUpdateResult,
  CodexDoctorResult,
  DictationModelStatus,
  DictationSessionState,
  LocalUsageSnapshot,
  TcpDaemonStatus,
  TailscaleDaemonCommandPreview,
  TailscaleStatus,
  TrayRecentThreadEntry,
  TraySessionUsage,
  WorkspaceInfo,
  AppMention,
  WorkspaceSettings,
} from "../types";
import type {
  GitFileDiff,
  GitFileStatus,
  GitCommitDiff,
  GitHubIssuesResponse,
  GitHubPullRequestComment,
  GitHubPullRequestDiff,
  GitHubPullRequestsResponse,
  GitLogResponse,
  ReviewTarget,
} from "../types";

function isMissingTauriInvokeError(error: unknown) {
  return (
    error instanceof TypeError &&
    (error.message.includes("reading 'invoke'") ||
      error.message.includes("reading \"invoke\""))
  );
}

export async function pickWorkspacePath(): Promise<string | null> {
  const selection = await open({ directory: true, multiple: false });
  if (!selection || Array.isArray(selection)) {
    return null;
  }
  return selection;
}

export async function pickWorkspacePaths(): Promise<string[]> {
  const selection = await open({ directory: true, multiple: true });
  if (!selection) {
    return [];
  }
  return Array.isArray(selection) ? selection : [selection];
}

export async function pickImageFiles(): Promise<string[]> {
  const selection = await open({
    multiple: true,
    filters: [
      {
        name: "Images",
        extensions: [
          "png",
          "jpg",
          "jpeg",
          "gif",
          "webp",
          "bmp",
          "tiff",
          "tif",
          "heic",
          "heif",
        ],
      },
    ],
  });
  if (!selection) {
    return [];
  }
  return Array.isArray(selection) ? selection : [selection];
}

export async function exportMarkdownFile(
  content: string,
  defaultFileName = "plan.md",
): Promise<string | null> {
  const selection = await save({
    title: "Export plan as Markdown",
    defaultPath: defaultFileName,
    filters: [
      {
        name: "Markdown",
        extensions: ["md"],
      },
    ],
  });
  if (!selection) {
    return null;
  }
  await invoke("write_text_file", { path: selection, content });
  return selection;
}

export async function listWorkspaces(): Promise<WorkspaceInfo[]> {
  try {
    return await invoke<WorkspaceInfo[]>("list_workspaces");
  } catch (error) {
    if (isMissingTauriInvokeError(error)) {
      // In non-Tauri environments (e.g., Electron/web previews), the invoke
      // bridge may be missing. Treat this as "no workspaces" instead of crashing.
      console.warn("Tauri invoke bridge unavailable; returning empty workspaces list.");
      return [];
    }
    throw error;
  }
}

export async function getCodexConfigPath(): Promise<string> {
  return invoke<string>("get_codex_config_path");
}

export type TextFileResponse = {
  exists: boolean;
  content: string;
  truncated: boolean;
};

export type GlobalAgentsResponse = TextFileResponse;
export type GlobalCodexConfigResponse = TextFileResponse;
export type AgentMdResponse = TextFileResponse;
export type AgentSummary = {
  name: string;
  description: string | null;
  developerInstructions: string | null;
  configFile: string;
  resolvedPath: string;
  managedByApp: boolean;
  fileExists: boolean;
};

export type AgentsSettings = {
  configPath: string;
  multiAgentEnabled: boolean;
  maxThreads: number;
  maxDepth: number;
  agents: AgentSummary[];
};

export type SetAgentsCoreInput = {
  multiAgentEnabled: boolean;
  maxThreads: number;
  maxDepth: number;
};

export type CreateAgentInput = {
  name: string;
  description?: string | null;
  developerInstructions?: string | null;
  template?: "blank" | string | null;
  model?: string | null;
  reasoningEffort?: string | null;
};

export type UpdateAgentInput = {
  originalName: string;
  name: string;
  description?: string | null;
  developerInstructions?: string | null;
  renameManagedFile?: boolean;
};

export type DeleteAgentInput = {
  name: string;
  deleteManagedFile?: boolean;
};

type FileScope = "workspace" | "global";
type FileKind = "agents" | "config";

async function fileRead(
  scope: FileScope,
  kind: FileKind,
  workspaceId?: string,
): Promise<TextFileResponse> {
  return invoke<TextFileResponse>("file_read", { scope, kind, workspaceId });
}

async function fileWrite(
  scope: FileScope,
  kind: FileKind,
  content: string,
  workspaceId?: string,
): Promise<void> {
  return invoke("file_write", { scope, kind, workspaceId, content });
}

export async function readImageAsDataUrl(path: string): Promise<string> {
  return invoke<string>("read_image_as_data_url", { path });
}

export async function readGlobalAgentsMd(): Promise<GlobalAgentsResponse> {
  return fileRead("global", "agents");
}

export async function writeGlobalAgentsMd(content: string): Promise<void> {
  return fileWrite("global", "agents", content);
}

export async function readGlobalCodexConfigToml(): Promise<GlobalCodexConfigResponse> {
  return fileRead("global", "config");
}

export async function writeGlobalCodexConfigToml(content: string): Promise<void> {
  return fileWrite("global", "config", content);
}

export async function getAgentsSettings(): Promise<AgentsSettings> {
  return invoke<AgentsSettings>("get_agents_settings");
}

export async function setAgentsCoreSettings(
  input: SetAgentsCoreInput,
): Promise<AgentsSettings> {
  return invoke<AgentsSettings>("set_agents_core_settings", { input });
}

export async function createAgent(input: CreateAgentInput): Promise<AgentsSettings> {
  return invoke<AgentsSettings>("create_agent", { input });
}

export async function updateAgent(input: UpdateAgentInput): Promise<AgentsSettings> {
  return invoke<AgentsSettings>("update_agent", { input });
}

export async function deleteAgent(input: DeleteAgentInput): Promise<AgentsSettings> {
  return invoke<AgentsSettings>("delete_agent", { input });
}

export async function readAgentConfigToml(agentName: string): Promise<string> {
  return invoke<string>("read_agent_config_toml", { agentName });
}

export async function writeAgentConfigToml(
  agentName: string,
  content: string,
): Promise<void> {
  return invoke("write_agent_config_toml", { agentName, content });
}

export async function getConfigModel(workspaceId: string): Promise<string | null> {
  const response = await invoke<{ model?: string | null }>("get_config_model", {
    workspaceId,
  });
  const model = response?.model;
  if (typeof model !== "string") {
    return null;
  }
  const trimmed = model.trim();
  return trimmed.length > 0 ? trimmed : null;
}

export async function addWorkspace(path: string): Promise<WorkspaceInfo> {
  return invoke<WorkspaceInfo>("add_workspace", { path });
}

export async function addWorkspaceFromGitUrl(
  url: string,
  destinationPath: string,
  targetFolderName: string | null,
): Promise<WorkspaceInfo> {
  return invoke<WorkspaceInfo>("add_workspace_from_git_url", {
    url,
    destinationPath,
    targetFolderName,
  });
}

export async function isWorkspacePathDir(path: string): Promise<boolean> {
  return invoke<boolean>("is_workspace_path_dir", { path });
}

export async function addClone(
  sourceWorkspaceId: string,
  copiesFolder: string,
  copyName: string,
): Promise<WorkspaceInfo> {
  return invoke<WorkspaceInfo>("add_clone", {
    sourceWorkspaceId,
    copiesFolder,
    copyName,
  });
}

export async function addWorktree(
  parentId: string,
  branch: string,
  name: string | null,
  copyAgentsMd = true,
): Promise<WorkspaceInfo> {
  return invoke<WorkspaceInfo>("add_worktree", { parentId, branch, name, copyAgentsMd });
}

export type WorktreeSetupStatus = {
  shouldRun: boolean;
  script: string | null;
};

export async function getWorktreeSetupStatus(
  workspaceId: string,
): Promise<WorktreeSetupStatus> {
  return invoke<WorktreeSetupStatus>("worktree_setup_status", { workspaceId });
}

export async function markWorktreeSetupRan(workspaceId: string): Promise<void> {
  return invoke("worktree_setup_mark_ran", { workspaceId });
}

export async function updateWorkspaceSettings(
  id: string,
  settings: WorkspaceSettings,
): Promise<WorkspaceInfo> {
  return invoke<WorkspaceInfo>("update_workspace_settings", { id, settings });
}

export async function removeWorkspace(id: string): Promise<void> {
  return invoke("remove_workspace", { id });
}

export async function removeWorktree(id: string): Promise<void> {
  return invoke("remove_worktree", { id });
}

export async function renameWorktree(
  id: string,
  branch: string,
): Promise<WorkspaceInfo> {
  return invoke<WorkspaceInfo>("rename_worktree", { id, branch });
}

export async function renameWorktreeUpstream(
  id: string,
  oldBranch: string,
  newBranch: string,
): Promise<void> {
  return invoke("rename_worktree_upstream", { id, oldBranch, newBranch });
}

export async function applyWorktreeChanges(workspaceId: string): Promise<void> {
  return invoke("apply_worktree_changes", { workspaceId });
}

export async function openWorkspaceIn(
  path: string,
  options: {
    appName?: string | null;
    command?: string | null;
    args?: string[];
    line?: number | null;
    column?: number | null;
  },
): Promise<void> {
  return invoke("open_workspace_in", {
    path,
    app: options.appName ?? null,
    command: options.command ?? null,
    args: options.args ?? [],
    line: options.line ?? null,
    column: options.column ?? null,
  });
}

export async function getOpenAppIcon(appName: string): Promise<string | null> {
  return invoke<string | null>("get_open_app_icon", { appName });
}

export async function connectWorkspace(id: string): Promise<void> {
  return invoke("connect_workspace", { id });
}

export async function setWorkspaceRuntimeCodexArgs(
  workspaceId: string,
  codexArgs: string | null,
): Promise<{ appliedCodexArgs: string | null; respawned: boolean }> {
  return invoke("set_workspace_runtime_codex_args", {
    workspaceId,
    codexArgs,
  });
}

export type NativeConversationAssociation = {
  domainId: string;
  sessionId: string;
  seatId: string;
  incarnation: string;
  authorizationGeneration: string;
  bindingGeneration: string;
  instanceId: string;
};

type NativeVisibleIntent = {
  nativeRequestId: string;
  workspaceId: string;
  expectedAssociation: NativeConversationAssociation;
  command: string;
  payload: Record<string, unknown>;
  error: string | null;
};

type NativeVisibleJournal = {
  version: 1;
  pending: NativeVisibleIntent | null;
  acceptedRequestIds: string[];
  lastAccepted: (NativeVisibleIntent & { response: unknown }) | null;
  rejectedRequestIds: string[];
  lastRejected: (NativeVisibleIntent & { reason: string }) | null;
  notDispatchedRequestIds: string[];
  lastNotDispatched: (NativeVisibleIntent & { reason: string }) | null;
};

const nativeVisibleKey = (workspaceId: string) =>
  `gogoke.native-visible-original.${encodeURIComponent(workspaceId)}`;
const emptyNativeVisibleJournal = (): NativeVisibleJournal =>
  ({ version: 1, pending: null, acceptedRequestIds: [], lastAccepted: null,
    rejectedRequestIds: [], lastRejected: null,
    notDispatchedRequestIds: [], lastNotDispatched: null });

function sameNativeAssociation(left: NativeConversationAssociation, right: NativeConversationAssociation) {
  return (Object.keys(left) as Array<keyof NativeConversationAssociation>).every(
    (key) => left[key] === right[key],
  );
}

function exactNativeAssociation(value: unknown): NativeConversationAssociation {
  const keys = ["domainId", "sessionId", "seatId", "incarnation",
    "authorizationGeneration", "bindingGeneration", "instanceId"] as const;
  if (!record(value) || Object.keys(value).length !== keys.length ||
      keys.some((key) => !nonempty(value[key]))) {
    throw new Error("Native visible conversation association is incomplete.");
  }
  return value as NativeConversationAssociation;
}

function nativeVisibleJournal(workspaceId: string): NativeVisibleJournal {
  const raw = window.localStorage.getItem(nativeVisibleKey(workspaceId));
  if (raw === null) return emptyNativeVisibleJournal();
  const parsed: unknown = JSON.parse(raw);
  if (!record(parsed) || parsed.version !== 1 ||
      !Array.isArray(parsed.acceptedRequestIds) ||
      !parsed.acceptedRequestIds.every(nonempty) ||
      !Array.isArray(parsed.rejectedRequestIds) ||
      !parsed.rejectedRequestIds.every(nonempty) ||
      !Array.isArray(parsed.notDispatchedRequestIds) ||
      !parsed.notDispatchedRequestIds.every(nonempty) ||
      (parsed.lastAccepted !== null && !record(parsed.lastAccepted)) ||
      (parsed.lastRejected !== null && !record(parsed.lastRejected)) ||
      (parsed.lastNotDispatched !== null && !record(parsed.lastNotDispatched)) ||
      (parsed.pending !== null && !record(parsed.pending))) {
    throw new Error("The original native visible request journal is invalid.");
  }
  if (parsed.pending) {
    const pending = parsed.pending;
    if (pending.workspaceId !== workspaceId || !nonempty(pending.nativeRequestId) ||
        !nonempty(pending.command) || !record(pending.payload)) {
      throw new Error("The pending native visible request identity is invalid.");
    }
    exactNativeAssociation(pending.expectedAssociation);
  }
  return parsed as NativeVisibleJournal;
}

function saveNativeVisibleJournal(workspaceId: string, journal: NativeVisibleJournal) {
  const serialized = JSON.stringify(journal);
  window.localStorage.setItem(nativeVisibleKey(workspaceId), serialized);
  if (window.localStorage.getItem(nativeVisibleKey(workspaceId)) !== serialized) {
    throw new Error("The original native visible request was not retained before dispatch.");
  }
}

function hostTerminalVisibleRejection(cause: unknown, nativeRequestId: string): string | null {
  const message = cause instanceof Error ? cause.message : String(cause);
  return message.startsWith(`GOGOKE_VISIBLE_DENIED:${nativeRequestId}:`) ||
    message.startsWith(`GOGOKE_VISIBLE_UNSUPPORTED:${nativeRequestId}:`)
    ? message : null;
}

function nativeVisibleNotDispatched(cause: unknown, nativeRequestId: string): string | null {
  const message = cause instanceof Error ? cause.message : String(cause);
  return message.startsWith(`GOGOKE_VISIBLE_NOT_DISPATCHED:${nativeRequestId}:`)
    ? message : null;
}

function settleNativeVisibleRejection(
  workspaceId: string, journal: NativeVisibleJournal,
  intent: NativeVisibleIntent, reason: string,
) {
  journal.rejectedRequestIds.push(intent.nativeRequestId);
  journal.lastRejected = { ...intent, error: reason, reason };
  journal.pending = null;
  saveNativeVisibleJournal(workspaceId, journal);
}

async function withNativeVisibleLock<T>(workspaceId: string, work: () => Promise<T>): Promise<T> {
  if (!navigator.locks?.request) {
    throw new Error("Native visible request serialization is unavailable.");
  }
  return navigator.locks.request(`gogoke-native-visible-${workspaceId}`, { mode: "exclusive" }, work);
}

export function pendingNativeVisibleIntent(workspaceId: string): NativeVisibleIntent | null {
  return nativeVisibleJournal(workspaceId).pending;
}

export function lastRejectedNativeVisibleIntent(workspaceId: string) {
  return nativeVisibleJournal(workspaceId).lastRejected;
}

export function lastNotDispatchedNativeVisibleIntent(workspaceId: string) {
  return nativeVisibleJournal(workspaceId).lastNotDispatched;
}

async function nativeVisibleTransport(workspaceId: string, observe = false): Promise<{
  state: "NATIVE" | "LEGACY" | "DISCONNECTED" | "REMOTE";
  association: NativeConversationAssociation | null;
  eventReadError?: string;
  stopConfirmed?: boolean;
}> {
  const value: unknown = await invoke("native_visible_transport", { workspaceId, observe });
  if (!record(value) || value.schema !== "gogoke.37.visible-conversation.v1" ||
      value.workspaceId !== workspaceId ||
      !["NATIVE", "LEGACY", "DISCONNECTED", "REMOTE"].includes(String(value.state))) {
    throw new Error("Actual native visible transport could not be identified.");
  }
  if (value.state === "NATIVE") {
    if (value.nativeEventReadError !== null && value.nativeEventReadError !== undefined &&
        (typeof value.nativeEventReadError !== "string" || !value.nativeEventReadError)) {
      throw new Error("Native event read failure has no original reason.");
    }
    if (value.nativeStopConfirmed !== undefined && typeof value.nativeStopConfirmed !== "boolean") {
      throw new Error("Native stop observation is malformed.");
    }
    return { state: "NATIVE", association: exactNativeAssociation(value.association),
      eventReadError: typeof value.nativeEventReadError === "string" ? value.nativeEventReadError : undefined,
      stopConfirmed: value.nativeStopConfirmed === true };
  }
  if (value.association !== null) {
    throw new Error("Non-native visible transport reported a native association.");
  }
  return { state: value.state as "LEGACY" | "DISCONNECTED" | "REMOTE", association: null };
}

async function confirmedNativeVisibleAssociation(
  workspaceId: string, actual: NativeConversationAssociation,
): Promise<NativeConversationAssociation> {
  const route = await design37UserFrame({ schema: "gogoke.37.owner-configuration.v1",
    command: "visible-conversation-route", workspaceId });
  if (!record(route) || route.schema !== "gogoke.37.visible-conversation.v1" ||
      route.workspaceId !== workspaceId || route.state !== "NATIVE" ||
      !sameNativeAssociation(actual, exactNativeAssociation(route.association))) {
    throw new Error(`Native visible conversation route is unresolved: ${JSON.stringify(route)}`);
  }
  return actual;
}

/** Identify the actual attachment; a saved choice never substitutes for a live transport. */
export async function nativeConversationAssociation(
  workspaceId: string,
): Promise<NativeConversationAssociation | null> {
  if (typeof window === "undefined" || !("__TAURI_INTERNALS__" in window)) return null;
  const transport = await nativeVisibleTransport(workspaceId);
  if (transport.state === "LEGACY" || transport.state === "REMOTE") return null;
  if (transport.state === "NATIVE" && transport.association) {
    return confirmedNativeVisibleAssociation(workspaceId, transport.association);
  }
  const route = await design37UserFrame({ schema: "gogoke.37.owner-configuration.v1",
    command: "visible-conversation-route", workspaceId });
  if (!record(route) || route.schema !== "gogoke.37.visible-conversation.v1" ||
      route.workspaceId !== workspaceId || route.state !== "NATIVE") {
    throw new Error(`Native visible conversation route is unresolved: ${JSON.stringify(route)}`);
  }
  exactNativeAssociation(route.association);
  throw new Error("The selected native conversation is not attached; connect the workspace before reading or sending.");
}

async function invokeVisibleWrite<T>(
  command: string, payload: Record<string, unknown>, nativeRequestId?: string,
): Promise<T> {
  const workspaceId = payload.workspaceId;
  if (typeof workspaceId !== "string" || !workspaceId) {
    throw new Error("Visible write requires a workspace ID.");
  }
  if (typeof window === "undefined" || !("__TAURI_INTERNALS__" in window)) {
    return invoke<T>(command, nativeRequestId === undefined ? payload : { ...payload, nativeRequestId });
  }
  const initialTransport = await nativeVisibleTransport(workspaceId);
  if (initialTransport.state === "LEGACY" || initialTransport.state === "REMOTE") {
    return invoke<T>(command, nativeRequestId === undefined ? payload : { ...payload, nativeRequestId });
  }
  if (initialTransport.state !== "NATIVE") {
    throw new Error("Native visible conversation transport is disconnected.");
  }
  return withNativeVisibleLock(workspaceId, async () => {
    const physicalStop = command === "stop_native_visible_session";
    const current = await nativeVisibleTransport(workspaceId);
    // A stopped attachment cannot restart its old observer. The host's original
    // StopFact permits resume while retaining the error; H/A still verifies it.
    const stoppedResume = command === "resume_thread" && current.stopConfirmed === true;
    const transport = physicalStop || stoppedResume
      ? current : await nativeVisibleTransport(workspaceId, true);
    if (transport.state !== "NATIVE" || !transport.association) {
      throw new Error("Native visible conversation transport changed before dispatch.");
    }
    if (!physicalStop && !stoppedResume && transport.eventReadError) {
      throw new Error(`GOGOKE_NATIVE_EVENT_READ_UNKNOWN:${transport.eventReadError}`);
    }
    const association = await confirmedNativeVisibleAssociation(workspaceId, transport.association);
    const journal = nativeVisibleJournal(workspaceId);
    if (journal.pending) {
      throw new Error(`Original native visible request ${journal.pending.nativeRequestId} is unresolved: ${journal.pending.error ?? "read its original host result before another write"}`);
    }
    const id = nativeRequestId ?? `visible_${crypto.randomUUID()}`;
    if (!/^[A-Za-z][A-Za-z0-9_-]{0,63}$/.test(id) ||
        journal.acceptedRequestIds.includes(id) || journal.rejectedRequestIds.includes(id) ||
        journal.notDispatchedRequestIds.includes(id)) {
      throw new Error("Native visible request ID is invalid or already used.");
    }
    const intent: NativeVisibleIntent = { nativeRequestId: id, workspaceId,
      expectedAssociation: association, command, payload: structuredClone(payload), error: null };
    journal.pending = intent;
    saveNativeVisibleJournal(workspaceId, journal);
    let response: T;
    try {
      response = await invoke<T>(command, { ...payload,
        nativeRequestId: id, expectedAssociation: association });
    } catch (cause) {
      intent.error = cause instanceof Error ? cause.message : String(cause);
      const notDispatched = nativeVisibleNotDispatched(cause, id);
      if (notDispatched) {
        journal.notDispatchedRequestIds.push(id);
        journal.lastNotDispatched = { ...intent, reason: notDispatched };
        journal.pending = null;
        saveNativeVisibleJournal(workspaceId, journal);
        throw cause;
      }
      const rejection = hostTerminalVisibleRejection(cause, id);
      if (rejection) {
        settleNativeVisibleRejection(workspaceId, journal, intent, rejection);
        throw cause;
      }
      journal.pending = intent;
      saveNativeVisibleJournal(workspaceId, journal);
      throw cause;
    }
    journal.acceptedRequestIds.push(id);
    journal.lastAccepted = { ...intent, response };
    journal.pending = null;
    saveNativeVisibleJournal(workspaceId, journal);
    return response;
  });
}

export async function recoverNativeVisibleRequest(
  workspaceId: string,
  nativeRequestId: string,
  expectedAssociation: NativeConversationAssociation,
) {
  return withNativeVisibleLock(workspaceId, async () => {
    const journal = nativeVisibleJournal(workspaceId);
    const pending = journal.pending;
    if (!pending || pending.nativeRequestId !== nativeRequestId ||
        !sameNativeAssociation(pending.expectedAssociation, exactNativeAssociation(expectedAssociation))) {
      throw new Error("No matching original native visible request is retained for recovery.");
    }
    let response: unknown;
    try {
      response = await invoke<unknown>("recover_native_visible_request", {
        workspaceId, nativeRequestId, expectedAssociation: pending.expectedAssociation,
      });
    } catch (cause) {
      pending.error = cause instanceof Error ? cause.message : String(cause);
      const rejection = hostTerminalVisibleRejection(cause, nativeRequestId);
      if (rejection) {
        settleNativeVisibleRejection(workspaceId, journal, pending, rejection);
        throw cause;
      }
      saveNativeVisibleJournal(workspaceId, journal);
      throw cause;
    }
    journal.acceptedRequestIds.push(nativeRequestId);
    journal.lastAccepted = { ...pending, response };
    journal.pending = null;
    saveNativeVisibleJournal(workspaceId, journal);
    return response;
  });
}

export async function recoverPendingNativeVisibleIntent(workspaceId: string) {
  const pending = pendingNativeVisibleIntent(workspaceId);
  if (!pending) return null;
  return recoverNativeVisibleRequest(workspaceId, pending.nativeRequestId, pending.expectedAssociation);
}

export async function startThread(workspaceId: string, nativeRequestId?: string) {
  return invokeVisibleWrite<any>("start_thread", { workspaceId }, nativeRequestId);
}

export async function stopNativeVisibleSession(workspaceId: string, nativeRequestId?: string) {
  return invokeVisibleWrite<unknown>("stop_native_visible_session", { workspaceId }, nativeRequestId);
}

export async function forkThread(workspaceId: string, threadId: string) {
  return invoke<any>("fork_thread", { workspaceId, threadId });
}

export async function compactThread(workspaceId: string, threadId: string) {
  return invoke<any>("compact_thread", { workspaceId, threadId });
}

function isInlineImageUrl(image: string) {
  return (
    image.startsWith("data:") ||
    image.startsWith("http://") ||
    image.startsWith("https://")
  );
}

async function convertImagesToDataUrls(images: string[]): Promise<string[]> {
  return Promise.all(
    images.map(async (image) => {
      if (isInlineImageUrl(image)) {
        return image;
      }
      return readImageAsDataUrl(image);
    }),
  );
}

async function normalizeImagesForRpc(images?: string[]): Promise<string[] | null> {
  if (images == null) {
    return null;
  }
  if (images.length === 0) {
    return [];
  }
  const hasPathImages = images.some((image) => !isInlineImageUrl(image));
  if (!hasPathImages) {
    return images;
  }
  let settings: AppSettings;
  let mobileRuntime: boolean;
  try {
    [settings, mobileRuntime] = await Promise.all([getAppSettings(), isMobileRuntime()]);
  } catch (error) {
    if (isMissingTauriInvokeError(error)) {
      return images;
    }
    throw error;
  }
  if (settings.backendMode !== "remote" && !mobileRuntime) {
    return images;
  }
  return convertImagesToDataUrls(images);
}

export async function sendUserMessage(
  workspaceId: string,
  threadId: string,
  text: string,
  options?: {
    model?: string | null;
    effort?: string | null;
    serviceTier?: "fast" | "flex" | null | undefined;
    accessMode?: "read-only" | "current" | "full-access";
    images?: string[];
    collaborationMode?: Record<string, unknown> | null;
    appMentions?: AppMention[];
    nativeRequestId?: string;
  },
) {
  const images = await normalizeImagesForRpc(options?.images);
  const payload: Record<string, unknown> = {
    workspaceId,
    threadId,
    text,
    model: options?.model ?? null,
    effort: options?.effort ?? null,
    accessMode: options?.accessMode ?? null,
    images,
  };
  if (options?.serviceTier !== undefined) {
    payload.serviceTier = options.serviceTier;
  }
  if (options?.collaborationMode) {
    payload.collaborationMode = options.collaborationMode;
  }
  if (options?.appMentions && options.appMentions.length > 0) {
    payload.appMentions = options.appMentions;
  }
  return invokeVisibleWrite("send_user_message", payload, options?.nativeRequestId);
}

export async function interruptTurn(
  workspaceId: string,
  threadId: string,
  turnId: string,
  nativeRequestId?: string,
) {
  return invokeVisibleWrite("turn_interrupt", { workspaceId, threadId, turnId }, nativeRequestId);
}

export async function steerTurn(
  workspaceId: string,
  threadId: string,
  turnId: string,
  text: string,
  images?: string[],
  appMentions?: AppMention[],
  nativeRequestId?: string,
) {
  const normalizedImages = await normalizeImagesForRpc(images);
  const payload: Record<string, unknown> = {
    workspaceId,
    threadId,
    turnId,
    text,
    images: normalizedImages,
  };
  if (appMentions && appMentions.length > 0) {
    payload.appMentions = appMentions;
  }
  return invokeVisibleWrite("turn_steer", payload, nativeRequestId);
}

export async function startReview(
  workspaceId: string,
  threadId: string,
  target: ReviewTarget,
  delivery?: "inline" | "detached",
) {
  const payload: Record<string, unknown> = { workspaceId, threadId, target };
  if (delivery) {
    payload.delivery = delivery;
  }
  return invoke("start_review", payload);
}

export async function respondToServerRequest(
  workspaceId: string,
  requestId: number | string,
  decision: "accept" | "decline",
  nativeRequestId?: string,
) {
  return invokeVisibleWrite("respond_to_server_request", {
    workspaceId,
    requestId,
    result: { decision },
  }, nativeRequestId);
}

export async function respondToUserInputRequest(
  workspaceId: string,
  requestId: number | string,
  answers: Record<string, { answers: string[] }>,
  nativeRequestId?: string,
) {
  return invokeVisibleWrite("respond_to_server_request", {
    workspaceId,
    requestId,
    result: { answers },
  }, nativeRequestId);
}

export async function rememberApprovalRule(
  workspaceId: string,
  command: string[],
) {
  return invoke("remember_approval_rule", { workspaceId, command });
}

export async function getGitStatus(workspace_id: string): Promise<{
  branchName: string;
  files: GitFileStatus[];
  stagedFiles: GitFileStatus[];
  unstagedFiles: GitFileStatus[];
  totalAdditions: number;
  totalDeletions: number;
}> {
  return invoke("get_git_status", { workspaceId: workspace_id });
}

export type InitGitRepoResponse =
  | { status: "initialized"; commitError?: string }
  | { status: "already_initialized" }
  | { status: "needs_confirmation"; entryCount: number };

export async function initGitRepo(
  workspaceId: string,
  branch: string,
  force = false,
): Promise<InitGitRepoResponse> {
  return invoke<InitGitRepoResponse>("init_git_repo", { workspaceId, branch, force });
}

export type CreateGitHubRepoResponse =
  | { status: "ok"; repo: string; remoteUrl?: string | null }
  | {
      status: "partial";
      repo: string;
      remoteUrl?: string | null;
      pushError?: string | null;
      defaultBranchError?: string | null;
    };

export async function createGitHubRepo(
  workspaceId: string,
  repo: string,
  visibility: "private" | "public",
  branch?: string | null,
): Promise<CreateGitHubRepoResponse> {
  return invoke<CreateGitHubRepoResponse>("create_github_repo", {
    workspaceId,
    repo,
    visibility,
    branch,
  });
}

export async function listGitRoots(
  workspace_id: string,
  depth: number,
): Promise<string[]> {
  return invoke("list_git_roots", { workspaceId: workspace_id, depth });
}

export async function getGitDiffs(
  workspace_id: string,
): Promise<GitFileDiff[]> {
  return invoke("get_git_diffs", { workspaceId: workspace_id });
}

export async function getGitLog(
  workspace_id: string,
  limit = 40,
): Promise<GitLogResponse> {
  return invoke("get_git_log", { workspaceId: workspace_id, limit });
}

export async function getGitCommitDiff(
  workspace_id: string,
  sha: string,
): Promise<GitCommitDiff[]> {
  return invoke("get_git_commit_diff", { workspaceId: workspace_id, sha });
}

export async function getGitRemote(workspace_id: string): Promise<string | null> {
  return invoke("get_git_remote", { workspaceId: workspace_id });
}

export async function stageGitFile(workspaceId: string, path: string) {
  return invoke("stage_git_file", { workspaceId, path });
}

export async function stageGitAll(workspaceId: string): Promise<void> {
  return invoke("stage_git_all", { workspaceId });
}

export async function unstageGitFile(workspaceId: string, path: string) {
  return invoke("unstage_git_file", { workspaceId, path });
}

export async function revertGitFile(workspaceId: string, path: string) {
  return invoke("revert_git_file", { workspaceId, path });
}

export async function revertGitAll(workspaceId: string) {
  return invoke("revert_git_all", { workspaceId });
}

export async function commitGit(
  workspaceId: string,
  message: string,
): Promise<void> {
  return invoke("commit_git", { workspaceId, message });
}

export async function pushGit(workspaceId: string): Promise<void> {
  return invoke("push_git", { workspaceId });
}

export async function pullGit(workspaceId: string): Promise<void> {
  return invoke("pull_git", { workspaceId });
}

export async function fetchGit(workspaceId: string): Promise<void> {
  return invoke("fetch_git", { workspaceId });
}

export async function syncGit(workspaceId: string): Promise<void> {
  return invoke("sync_git", { workspaceId });
}

export async function getGitHubIssues(
  workspace_id: string,
): Promise<GitHubIssuesResponse> {
  return invoke("get_github_issues", { workspaceId: workspace_id });
}

export async function getGitHubPullRequests(
  workspace_id: string,
): Promise<GitHubPullRequestsResponse> {
  return invoke("get_github_pull_requests", { workspaceId: workspace_id });
}

export async function getGitHubPullRequestDiff(
  workspace_id: string,
  prNumber: number,
): Promise<GitHubPullRequestDiff[]> {
  return invoke("get_github_pull_request_diff", {
    workspaceId: workspace_id,
    prNumber,
  });
}

export async function getGitHubPullRequestComments(
  workspace_id: string,
  prNumber: number,
): Promise<GitHubPullRequestComment[]> {
  return invoke("get_github_pull_request_comments", {
    workspaceId: workspace_id,
    prNumber,
  });
}

export async function checkoutGitHubPullRequest(
  workspace_id: string,
  prNumber: number,
): Promise<void> {
  return invoke("checkout_github_pull_request", {
    workspaceId: workspace_id,
    prNumber,
  });
}

export async function localUsageSnapshot(
  days?: number,
  workspacePath?: string | null,
): Promise<LocalUsageSnapshot> {
  const payload: { days: number; workspacePath?: string } = { days: days ?? 30 };
  if (workspacePath) {
    payload.workspacePath = workspacePath;
  }
  return invoke("local_usage_snapshot", payload);
}

export async function getModelList(workspaceId: string) {
  return invoke<any>("model_list", { workspaceId });
}

export async function getExperimentalFeatureList(
  workspaceId: string,
  cursor?: string | null,
  limit?: number | null,
) {
  return invoke<any>("experimental_feature_list", { workspaceId, cursor, limit });
}

export async function setCodexFeatureFlag(
  featureKey: string,
  enabled: boolean,
): Promise<void> {
  return invoke("set_codex_feature_flag", { featureKey, enabled });
}

export async function generateRunMetadata(workspaceId: string, prompt: string) {
  return invoke<{ title: string; worktreeName: string }>("generate_run_metadata", {
    workspaceId,
    prompt,
  });
}

export async function getCollaborationModes(workspaceId: string) {
  return invoke<any>("collaboration_mode_list", { workspaceId });
}

export async function getAccountRateLimits(workspaceId: string) {
  return invoke<any>("account_rate_limits", { workspaceId });
}

export async function getAccountInfo(workspaceId: string) {
  return invoke<any>("account_read", { workspaceId });
}

export async function runCodexLogin(workspaceId: string) {
  return invoke<{ loginId: string; authUrl: string; raw?: unknown }>("codex_login", {
    workspaceId,
  });
}

export async function cancelCodexLogin(workspaceId: string) {
  return invoke<{ canceled: boolean; status?: string; raw?: unknown }>(
    "codex_login_cancel",
    { workspaceId },
  );
}

export async function getSkillsList(workspaceId: string) {
  return invoke<any>("skills_list", { workspaceId });
}

export async function getAppsList(
  workspaceId: string,
  cursor?: string | null,
  limit?: number | null,
  threadId?: string | null,
) {
  return invoke<any>("apps_list", { workspaceId, cursor, limit, threadId });
}

export async function getPromptsList(workspaceId: string) {
  return invoke<any>("prompts_list", { workspaceId });
}

export async function getWorkspacePromptsDir(workspaceId: string) {
  return invoke<string>("prompts_workspace_dir", { workspaceId });
}

export async function getGlobalPromptsDir(workspaceId: string) {
  return invoke<string>("prompts_global_dir", { workspaceId });
}

export async function createPrompt(
  workspaceId: string,
  data: {
    scope: "workspace" | "global";
    name: string;
    description?: string | null;
    argumentHint?: string | null;
    content: string;
  },
) {
  return invoke<any>("prompts_create", {
    workspaceId,
    scope: data.scope,
    name: data.name,
    description: data.description ?? null,
    argumentHint: data.argumentHint ?? null,
    content: data.content,
  });
}

export async function updatePrompt(
  workspaceId: string,
  data: {
    path: string;
    name: string;
    description?: string | null;
    argumentHint?: string | null;
    content: string;
  },
) {
  return invoke<any>("prompts_update", {
    workspaceId,
    path: data.path,
    name: data.name,
    description: data.description ?? null,
    argumentHint: data.argumentHint ?? null,
    content: data.content,
  });
}

export async function deletePrompt(workspaceId: string, path: string) {
  return invoke<any>("prompts_delete", { workspaceId, path });
}

export async function movePrompt(
  workspaceId: string,
  data: { path: string; scope: "workspace" | "global" },
) {
  return invoke<any>("prompts_move", {
    workspaceId,
    path: data.path,
    scope: data.scope,
  });
}

export async function getAppSettings(): Promise<AppSettings> {
  return invoke<AppSettings>("get_app_settings");
}

export async function isMobileRuntime(): Promise<boolean> {
  return invoke<boolean>("is_mobile_runtime");
}

export async function updateAppSettings(settings: AppSettings): Promise<AppSettings> {
  return invoke<AppSettings>("update_app_settings", { settings });
}

export async function tailscaleStatus(): Promise<TailscaleStatus> {
  return invoke<TailscaleStatus>("tailscale_status");
}

export async function tailscaleDaemonCommandPreview(): Promise<TailscaleDaemonCommandPreview> {
  return invoke<TailscaleDaemonCommandPreview>("tailscale_daemon_command_preview");
}

export async function tailscaleDaemonStart(): Promise<TcpDaemonStatus> {
  return invoke<TcpDaemonStatus>("tailscale_daemon_start");
}

export async function tailscaleDaemonStop(): Promise<TcpDaemonStatus> {
  return invoke<TcpDaemonStatus>("tailscale_daemon_stop");
}

export async function tailscaleDaemonStatus(): Promise<TcpDaemonStatus> {
  return invoke<TcpDaemonStatus>("tailscale_daemon_status");
}

type MenuAcceleratorUpdate = {
  id: string;
  accelerator: string | null;
};

export async function setMenuAccelerators(
  updates: MenuAcceleratorUpdate[],
): Promise<void> {
  return invoke("menu_set_accelerators", { updates });
}

export async function runCodexDoctor(
  codexBin: string | null,
  codexArgs: string | null,
): Promise<CodexDoctorResult> {
  return invoke<CodexDoctorResult>("codex_doctor", { codexBin, codexArgs });
}

export async function runCodexUpdate(
  codexBin: string | null,
  codexArgs: string | null,
): Promise<CodexUpdateResult> {
  return invoke<CodexUpdateResult>("codex_update", { codexBin, codexArgs });
}

export async function getWorkspaceFiles(workspaceId: string) {
  return invoke<string[]>("list_workspace_files", { workspaceId });
}

export async function readWorkspaceFile(
  workspaceId: string,
  path: string,
): Promise<{ content: string; truncated: boolean }> {
  return invoke<{ content: string; truncated: boolean }>("read_workspace_file", {
    workspaceId,
    path,
  });
}

export async function readAgentMd(workspaceId: string): Promise<AgentMdResponse> {
  return fileRead("workspace", "agents", workspaceId);
}

export async function writeAgentMd(workspaceId: string, content: string): Promise<void> {
  return fileWrite("workspace", "agents", content, workspaceId);
}

export async function listGitBranches(workspaceId: string) {
  return invoke<any>("list_git_branches", { workspaceId });
}

export async function checkoutGitBranch(workspaceId: string, name: string) {
  return invoke("checkout_git_branch", { workspaceId, name });
}

export async function createGitBranch(workspaceId: string, name: string) {
  return invoke("create_git_branch", { workspaceId, name });
}

function withModelId(modelId?: string | null) {
  return modelId ? { modelId } : {};
}

export async function getDictationModelStatus(
  modelId?: string | null,
): Promise<DictationModelStatus> {
  return invoke<DictationModelStatus>(
    "dictation_model_status",
    withModelId(modelId),
  );
}

export async function downloadDictationModel(
  modelId?: string | null,
): Promise<DictationModelStatus> {
  return invoke<DictationModelStatus>(
    "dictation_download_model",
    withModelId(modelId),
  );
}

export async function cancelDictationDownload(
  modelId?: string | null,
): Promise<DictationModelStatus> {
  return invoke<DictationModelStatus>(
    "dictation_cancel_download",
    withModelId(modelId),
  );
}

export async function removeDictationModel(
  modelId?: string | null,
): Promise<DictationModelStatus> {
  return invoke<DictationModelStatus>(
    "dictation_remove_model",
    withModelId(modelId),
  );
}

export async function startDictation(
  preferredLanguage: string | null,
): Promise<DictationSessionState> {
  return invoke("dictation_start", { preferredLanguage });
}

export async function requestDictationPermission(): Promise<boolean> {
  return invoke("dictation_request_permission");
}

export async function stopDictation(): Promise<DictationSessionState> {
  return invoke("dictation_stop");
}

export async function cancelDictation(): Promise<DictationSessionState> {
  return invoke("dictation_cancel");
}

export async function openTerminalSession(
  workspaceId: string,
  terminalId: string,
  cols: number,
  rows: number,
): Promise<{ id: string }> {
  return invoke("terminal_open", { workspaceId, terminalId, cols, rows });
}

export async function writeTerminalSession(
  workspaceId: string,
  terminalId: string,
  data: string,
): Promise<void> {
  return invoke("terminal_write", { workspaceId, terminalId, data });
}

export async function resizeTerminalSession(
  workspaceId: string,
  terminalId: string,
  cols: number,
  rows: number,
): Promise<void> {
  return invoke("terminal_resize", { workspaceId, terminalId, cols, rows });
}

export async function closeTerminalSession(
  workspaceId: string,
  terminalId: string,
): Promise<void> {
  return invoke("terminal_close", { workspaceId, terminalId });
}

export async function listThreads(
  workspaceId: string,
  cursor?: string | null,
  limit?: number | null,
  sortKey?: "created_at" | "updated_at" | null,
) {
  return invoke<any>("list_threads", { workspaceId, cursor, limit, sortKey });
}

export async function listMcpServerStatus(
  workspaceId: string,
  cursor?: string | null,
  limit?: number | null,
) {
  return invoke<any>("list_mcp_server_status", { workspaceId, cursor, limit });
}

export async function resumeThread(workspaceId: string, threadId: string, nativeRequestId?: string) {
  return invokeVisibleWrite<any>("resume_thread", { workspaceId, threadId }, nativeRequestId);
}

export async function readThread(workspaceId: string, threadId: string) {
  return invoke<any>("read_thread", { workspaceId, threadId });
}

export async function threadLiveSubscribe(workspaceId: string, threadId: string) {
  return invoke<any>("thread_live_subscribe", { workspaceId, threadId });
}

export async function threadLiveUnsubscribe(workspaceId: string, threadId: string) {
  return invoke<any>("thread_live_unsubscribe", { workspaceId, threadId });
}

export async function archiveThread(workspaceId: string, threadId: string) {
  return invoke<any>("archive_thread", { workspaceId, threadId });
}

export async function setThreadName(
  workspaceId: string,
  threadId: string,
  name: string,
) {
  return invoke<any>("set_thread_name", { workspaceId, threadId, name });
}

export async function setTrayRecentThreads(entries: TrayRecentThreadEntry[]) {
  return invoke<void>("set_tray_recent_threads", { entries });
}

export async function setTraySessionUsage(usage: TraySessionUsage | null) {
  return invoke<void>("set_tray_session_usage", { usage });
}

export async function generateCommitMessage(
  workspaceId: string,
  commitMessageModelId: string | null,
): Promise<string> {
  return invoke("generate_commit_message", { workspaceId, commitMessageModelId });
}

export type GeneratedAgentConfiguration = {
  description: string;
  developerInstructions: string;
};

export async function generateAgentDescription(
  workspaceId: string,
  description: string,
): Promise<GeneratedAgentConfiguration> {
  return invoke("generate_agent_description", { workspaceId, description });
}

export type AppBuildType = "debug" | "release";

export async function getAppBuildType(): Promise<AppBuildType> {
  return invoke<AppBuildType>("app_build_type");
}

export type GogokeUpdateOffer = {
  version: string;
  releaseType: "full" | "resources";
  asset: string;
  sha256: string;
  publishedAt: string;
  notesUrl: string;
  notes: string;
};

export async function checkGogokeUpdate(): Promise<GogokeUpdateOffer | null> {
  return invoke<GogokeUpdateOffer | null>("gogoke_update_check");
}

export async function installGogokeUpdate(version: string): Promise<void> {
  return invoke<void>("gogoke_update_install", { version });
}

export async function signalGogokeUpdateReady(): Promise<void> {
  const resourceSetId = new URL(window.location.href).searchParams.get("gogoke-resource-set");
  return invoke<void>("gogoke_update_signal_ready", { resourceSetId });
}

export type GogokeUpdateFailureNotice = {
  kind: "failure" | "cleanup_pending";
  message: string;
};

export async function takeGogokeUpdateFailure(): Promise<GogokeUpdateFailureNotice | null> {
  return invoke<GogokeUpdateFailureNotice | null>("gogoke_update_take_failure");
}

export async function sendNotification(
  title: string,
  body: string,
  options?: {
    id?: number;
    group?: string;
    actionTypeId?: string;
    sound?: string;
    autoCancel?: boolean;
    extra?: Record<string, unknown>;
  },
): Promise<void> {
  const macosDebugBuild = await invoke<boolean>("is_macos_debug_build").catch(
    () => false,
  );
  const attemptFallback = async () => {
    try {
      await invoke("send_notification_fallback", { title, body });
      return true;
    } catch (error) {
      console.warn("Notification fallback failed.", { error });
      return false;
    }
  };

  // In dev builds on macOS, the notification plugin can silently fail because
  // the process is not a bundled app. Prefer the native AppleScript fallback.
  if (macosDebugBuild) {
    await attemptFallback();
    return;
  }

  try {
    const notification = await import("@tauri-apps/plugin-notification");
    let permissionGranted = await notification.isPermissionGranted();
    if (!permissionGranted) {
      const permission = await notification.requestPermission();
      permissionGranted = permission === "granted";
      if (!permissionGranted) {
        console.warn("Notification permission not granted.", { permission });
        await attemptFallback();
        return;
      }
    }
    if (permissionGranted) {
      const payload: NotificationOptions = { title, body };
      if (options?.id !== undefined) {
        payload.id = options.id;
      }
      if (options?.group !== undefined) {
        payload.group = options.group;
      }
      if (options?.actionTypeId !== undefined) {
        payload.actionTypeId = options.actionTypeId;
      }
      if (options?.sound !== undefined) {
        payload.sound = options.sound;
      }
      if (options?.autoCancel !== undefined) {
        payload.autoCancel = options.autoCancel;
      }
      if (options?.extra !== undefined) {
        payload.extra = options.extra;
      }
      await notification.sendNotification(payload);
      return;
    }
  } catch (error) {
    console.warn("Notification plugin failed.", { error });
  }

  await attemptFallback();
}


export type GogokeProductGoalRequest = {
  goal: { id: string; title: string };
  runControlledTask?: true;
  publishTestDraft?: true;
  fixtureDriverId?: string;
  ledgerMergePullNumber?: number;
  ledger: {
    repository: string;
    commit: string;
    path: string;
    contentHash: string;
  };
};

export type GogokeProductGoalView = GogokeProductGoalRequest & {
  caller: {
    admitted: true;
    policyRevision: string;
    principalId: string;
    profileId: string;
    revocationHead: string;
    role: "controller";
    seatId: string;
  };
  nativeHost: {
    reachable: true;
    elapsedMicros: number;
  };
  ledgerReadback: {
    state: "COMMITTED_BYTES_VERIFIED_NOT_ADOPTED";
    gitBlob: string;
  };
  ledgerMerge?: {
    state: "PR_MERGE_ACCEPTED_FACT_VERIFIED";
    pullNumber: number;
    mergeCommit: string;
    mergedBy: string;
  };
  controlledTask?: {
    state: "VALIDATED_TEST_RESULT_NOT_ADOPTED";
    sourceCommit: string;
    sourceBlob: string;
    reportSha256: string;
    modelId: string;
    relativePath: string;
    embeddedBytesSha256: string;
    actionCompletionRef: string;
    manifestHash: string;
    decisionReceiptId: string;
    objectiveOutcomeContentHash: string;
    objectiveOutcomeReceiptId: string;
    evaluationContentHash: string;
    evaluationReceiptId: string;
    metricsHash: string;
    dreamRunContentHash: string;
    dreamProposalContentHash: string;
    dreamProposalState: "DRAFT_TEST_ONLY_NOT_ACTIVATED";
    fixtureDriverBinding?: {
      driverId: string;
      adapterVersion: "1.0.0";
      runtimeInstanceId: string;
      launchDigestSha256: string;
    };
  };
  testLedgerDraft?: {
    state: "DRAFT_COMMITTED_NOT_ADOPTED";
    repository: "taiyun668/gogoke";
    branch: "s1-r4-ledger-test/r2-02";
    commit: string;
    path: string;
    gitBlob: string;
    contentHash: string;
  };
  acceptance: "TEST_FIXTURE_NOT_ADOPTED";
};

export async function runGogokeR2GoalProbe(
  request: GogokeProductGoalRequest,
): Promise<GogokeProductGoalView> {
  return invoke<GogokeProductGoalView>("gogoke_r2_goal_probe", { request });
}
