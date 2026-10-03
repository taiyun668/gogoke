#!/usr/bin/env node
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const SCRIPT_DIR = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(SCRIPT_DIR, "../..");
const BUNDLE_SCHEMA = "gogoke.cli-protocol-capture.v1";
const REPORT_SCHEMA = "gogoke.cli-protocol-diff.v1";
const REDACTED = "[REDACTED]";
const VALID_DIRECTIONS = new Set(["in", "out"]);
const SEMANTIC_STRING_KEYS = new Set(["method", "type", "status", "sessionUpdate", "codexMethod", "role", "kind", "state", "phase", "approvalPolicy", "collaborationMode", "schema", "rpcRole"]);

function fail(message) {
  throw new Error(message);
}

function parseArgs(argv) {
  const command = argv[0];
  if (command !== "import" && command !== "compare") {
    fail("expected command: import or compare");
  }
  const options = new Map();
  for (let index = 1; index < argv.length; index += 1) {
    const flag = argv[index];
    if (!flag.startsWith("--") || index + 1 >= argv.length) {
      fail("arguments must be --name value pairs");
    }
    const key = flag.slice(2);
    if (options.has(key)) fail(`duplicate option --${key}`);
    options.set(key, argv[++index]);
  }
  return { command, options };
}

function requireOption(options, name) {
  const value = options.get(name);
  if (!value) fail(`missing required option --${name}`);
  return value;
}

function parseSha(value, name, lengths = [64]) {
  const normalized = value.toLowerCase();
  if (!lengths.includes(normalized.length) || !/^[a-f0-9]+$/.test(normalized)) {
    fail(`--${name} must be a hexadecimal SHA (${lengths.join(" or ")} characters)`);
  }
  return normalized;
}

function isInside(root, candidate) {
  const relative = path.relative(root, candidate);
  return relative === "" || (!relative.startsWith(`..${path.sep}`) && relative !== ".." && !path.isAbsolute(relative));
}

function assertOutsideRepository(candidate, description, mustExist) {
  const absolute = path.resolve(candidate);
  let resolved;
  try {
    if (mustExist) {
      resolved = fs.realpathSync(absolute);
    } else {
      const parent = fs.realpathSync(path.dirname(absolute));
      resolved = path.join(parent, path.basename(absolute));
    }
  } catch {
    fail(`${description} path is unavailable`);
  }
  let repoReal;
  try {
    repoReal = fs.realpathSync(REPO_ROOT);
  } catch {
    fail("repository root is unavailable");
  }
  if (isInside(repoReal, resolved)) {
    fail(`${description} must stay outside the repository`);
  }
  return absolute;
}

function readPrivateJson(candidate, description) {
  const absolute = assertOutsideRepository(candidate, description, true);
  let bytes;
  try {
    bytes = fs.readFileSync(absolute);
  } catch {
    fail(`${description} could not be read`);
  }
  let json;
  try {
    json = JSON.parse(bytes.toString("utf8"));
  } catch {
    fail(`${description} is not valid UTF-8 JSON`);
  }
  return { json, sha256: sha256(bytes) };
}

function sha256(bytes) {
  return crypto.createHash("sha256").update(bytes).digest("hex");
}

function jsonPointerPart(value) {
  return String(value).replaceAll("~", "~0").replaceAll("/", "~1");
}

function safeStringValue(value) {
  return /(?:https?:\/\/|file:\/\/|[a-z]:[\\/]|\\\\[^\\]+\\|\bBearer\s+|\bsk-[A-Za-z0-9_-]{8,})/i.test(value);
}

function sensitiveKeyReason(key) {
  if (/^(?:text|message|prompt|reason|output|command|instructions?)$/i.test(key)) return "free-text";
  if (/(?:token|secret|password|credential|authorization|oauth|device.?code|verification.?code|api.?key|access.?key)/i.test(key)) return "credential-or-auth";
  if (/(?:^|_)(?:url|uri|path|cwd|home|user|username|email|account)(?:$|_)/i.test(key) || /(?:url|uri|path|cwd|home|user|username|email|account)$/i.test(key) || /^(?:user|username|account)(?:id|name|identifier)$/i.test(key)) return "machine-or-account-location";
  return null;
}

function idCategory(key, pointer, parent) {
  const normalized = key.replaceAll("_", "").toLowerCase();
  const named = new Map([
    ["sessionid", "session"], ["threadid", "thread"], ["turnid", "turn"],
    ["itemid", "item"], ["requestid", "request"], ["callid", "call"], ["userid", "user"], ["accountid", "account"],
    ["generation", "generation"], ["sourceepoch", "epoch"],
    ["sourcecursor", "cursor"], ["rawsourcecursor", "cursor"],
    ["domainid", "domain"], ["targetid", "target"], ["seatid", "seat"],
    ["homeid", "home"], ["instanceid", "instance"], ["bindingid", "binding"],
    ["operationid", "operation"], ["attemptid", "attempt"],
  ]);
  if (named.has(normalized)) return named.get(normalized);
  if (key.toLowerCase() === "id") {
    if (parent && typeof parent === "object") {
      if (Object.hasOwn(parent, "method") || Object.hasOwn(parent, "result") || Object.hasOwn(parent, "error")) return "rpc";
    }
    if (/\.turn(?:\.|\[)/i.test(pointer)) return "turn";
    if (/\.item(?:\.|\[)/i.test(pointer)) return "item";
    return "id";
  }
  return null;
}

class AliasRegistry {
  #aliases = new Map();
  #next = new Map();

  alias(category, raw) {
    const value = String(raw);
    const typed = category === "rpc" || category === "claude-control" || category === "claude-user";
    const key = `${category}\u0000${typed ? `${typeof raw}:` : ""}${value}`;
    if (!this.#aliases.has(key)) {
      const ordinal = (this.#next.get(category) || 0) + 1;
      this.#next.set(category, ordinal);
      this.#aliases.set(key, `<${category.toUpperCase()}_${ordinal}>`);
    }
    return this.#aliases.get(key);
  }
}

function sanitize(value, context, aliases, redactions, pointer = "", parent = null, key = "") {
  if (Array.isArray(value)) {
    return value.map((item, index) => sanitize(item, context, aliases, redactions, `${pointer}/${index}`, value, String(index)));
  }
  if (value && typeof value === "object") {
    const out = {};
    for (const [childKey, childValue] of Object.entries(value)) {
      const childPointer = `${pointer}/${jsonPointerPart(childKey)}`;
      const category = idCategory(childKey, childPointer, value);
      if (category && (typeof childValue === "string" || typeof childValue === "number")) {
        out[childKey] = aliases.alias(category, childValue);
        continue;
      }
      const reason = sensitiveKeyReason(childKey);
      if (reason && childValue !== null) {
        redactions.push({ source: context, pointer: childPointer, reason });
        out[childKey] = REDACTED;
        continue;
      }
      out[childKey] = sanitize(childValue, context, aliases, redactions, childPointer, value, childKey);
    }
    return out;
  }
  if (typeof value === "string") {
    if (safeStringValue(value)) {
      redactions.push({ source: context, pointer, reason: "sensitive-pattern" });
      return REDACTED;
    }
    if (SEMANTIC_STRING_KEYS.has(key)) return value;
    redactions.push({ source: context, pointer, reason: "non-semantic-string" });
    return REDACTED;
  }
  return value;
}

function extractStringValues(value, keyPattern, result = []) {
  if (Array.isArray(value)) {
    for (const item of value) extractStringValues(item, keyPattern, result);
  } else if (value && typeof value === "object") {
    for (const [key, child] of Object.entries(value)) {
      if (keyPattern.test(key) && typeof child === "string") result.push(child);
      else extractStringValues(child, keyPattern, result);
    }
  }
  return result;
}

function classifyFailureEvidence(evidence) {
  if (evidence?.schema !== "gogoke.37.private-cli-direct-tool-error.v1" || !Array.isArray(evidence.matches)) {
    fail("failure evidence has an unsupported private-capture schema");
  }
  const outputs = extractStringValues(evidence.matches, /^(?:output|message)$/i);
  const joined = outputs.join("\n").toLowerCase();
  if (/failed to spawn code-mode host/.test(joined) && /access is denied/.test(joined) && /os error\s*5/.test(joined)) {
    return { classification: "code_mode_host_spawn_access_denied", matchedOutputFields: outputs.length };
  }
  return { classification: "real_cli_tool_error_unclassified", matchedOutputFields: outputs.length };
}

function claudeAssociation(parsed, direction) {
  if (parsed.type === "control_request" && (typeof parsed.request_id === "string" || typeof parsed.request_id === "number")) {
    return { namespace: "claude-control", role: "request", id: parsed.request_id };
  }
  if (parsed.type === "control_response" && parsed.response && typeof parsed.response === "object"
      && (typeof parsed.response.request_id === "string" || typeof parsed.response.request_id === "number")) {
    return { namespace: "claude-control", role: "response", id: parsed.response.request_id };
  }
  if (parsed.type !== "user" || (typeof parsed.uuid !== "string" && typeof parsed.uuid !== "number")
      || parsed.isSynthetic === true || parsed.tool_use_result !== undefined || parsed.parent_tool_use_id != null
      || !parsed.message || typeof parsed.message !== "object" || parsed.message.role !== "user") return null;
  const content = parsed.message.content;
  const humanContent = typeof content === "string" || (Array.isArray(content) && content.length > 0
    && content.every(part => part && typeof part === "object" && part.type !== "tool_result"));
  return humanContent ? { namespace: "claude-user", role: direction === "out" ? "input" : "echo", id: parsed.uuid } : null;
}

function normalizeRawFrames(document, direction, aliases, redactions) {
  if (!["gogoke.37.private-direct-frames.v1", "gogoke.37.private-e2e-ledger.v1"].includes(document?.schema) || !Array.isArray(document.frames)) {
    fail("raw frame input has an unsupported private-capture schema");
  }
  if (document.count !== undefined && document.count !== document.frames.length) fail("raw frame count does not match its payload");
  if (document.credentialReads !== false) fail("raw frame input does not attest credentialReads=false");
  if (document.databaseWrites !== false) fail("raw frame input does not attest databaseWrites=false");
  if (document.commands !== undefined && !Array.isArray(document.commands)) fail("commands must be an array when present");
  const commandRows = (document.commands || []).map(command => ({ ...command, direction: command.direction || "out", sourceKind: "command" }));
  const actualCommands = commandRows.filter(command => ["WRITTEN", "OBSERVED"].includes(String(command.phase || "").toUpperCase()));
  const records = [
    ...document.frames.map(frame => ({ ...frame, sourceKind: "frame" })),
    ...actualCommands,
  ];
  const laneSequences = { inbound: 0, outbound: 0 };
  const cursorReferences = new Map();
  const normalizedRecords = records.map((frame, index) => {
    if (typeof frame.originalFrame !== "string") fail("raw frame is missing originalFrame bytes");
    const fromSource = frame.direction;
    const fallbackDirection = frame.sourceKind === "command" ? "out" : direction;
    const actualDirection = fromSource || fallbackDirection;
    if (!VALID_DIRECTIONS.has(actualDirection)) fail("raw frame direction is absent or invalid; pass --direction in/out");
    if (frame.sourceKind === "command" && actualDirection !== "out") fail("command records must have outbound direction");
    if (frame.sourceKind === "frame" && fromSource && direction && fromSource !== direction) fail("--direction conflicts with a frame direction");
    let parsed;
    try { parsed = JSON.parse(frame.originalFrame); } catch { fail("a raw originalFrame is not valid JSON"); }
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) fail("a raw originalFrame is not a JSON object");
    const protocolMessage = sanitize(parsed, `${frame.sourceKind}:${index + 1}`, aliases, redactions, "");
    const claude = claudeAssociation(parsed, actualDirection);
    const hasRpcId = !claude && Object.hasOwn(parsed, "id");
    const rpcRole = !hasRpcId ? "notification" : Object.hasOwn(parsed, "method") ? "request" : (Object.hasOwn(parsed, "result") || Object.hasOwn(parsed, "error")) ? "response" : "message-with-id";
    const rpcIdAlias = hasRpcId && (typeof parsed.id === "string" || typeof parsed.id === "number")
      ? aliases.alias(idCategory("id", "$.id", parsed) || "rpc", parsed.id)
      : null;
    const lane = actualDirection === "in" ? "inbound" : "outbound";
    const sequence = ++laneSequences[lane];
    const sessionAlias = frame.sessionId == null ? null : aliases.alias("session", frame.sessionId);
    const operationAlias = frame.operationId == null ? null : aliases.alias("operation", frame.operationId);
    const generationAlias = frame.generation == null ? null : aliases.alias("generation", frame.generation);
    const epochAlias = frame.sourceEpoch == null ? null : aliases.alias("epoch", frame.sourceEpoch);
    const ticketAlias = frame.processTicket == null ? null : aliases.alias("ticket", frame.processTicket);
    const custodianNonceAlias = frame.custodianNonce == null ? null : aliases.alias("custodian", frame.custodianNonce);
    const cursorAlias = frame.sourceCursor == null ? null : aliases.alias("cursor", frame.sourceCursor);
    const reference = { lane, sequence };
    if (frame.sourceKind === "frame" && frame.sourceCursor != null) {
      const matches = cursorReferences.get(String(frame.sourceCursor)) || [];
      matches.push({ ...reference, sessionId: frame.sessionId ?? null, operationId: frame.operationId ?? null, generation: frame.generation ?? null, sourceEpoch: frame.sourceEpoch ?? null });
      cursorReferences.set(String(frame.sourceCursor), matches);
    }
    return {
      lane,
      sequence,
      sourceKind: frame.sourceKind,
      journalState: typeof frame.state === "string" ? frame.state : "UNKNOWN",
      sourceEpoch: aliases.alias("epoch", frame.sourceEpoch ?? "missing"),
      sourceCursor: aliases.alias("cursor", frame.sourceCursor ?? "missing"),
      sessionAlias,
      operationAlias,
      generationAlias,
      sourceEpochAlias: epochAlias,
      ticketAlias,
      custodianNonceAlias,
      stepId: frame.stepId == null ? null : aliases.alias("step", frame.stepId),
      phase: frame.sourceKind === "command" ? String(frame.phase).toUpperCase() : null,
      rpcRole,
      rpcIdAlias,
      rpcIdType: rpcIdAlias === null ? null : typeof parsed.id,
      ...(claude ? { protocolNamespace: claude.namespace, protocolRole: claude.role,
        protocolIdAlias: aliases.alias(claude.namespace, claude.id), protocolIdType: typeof claude.id } : {}),
      message: protocolMessage,
    };
  });
  const directionCounts = { in: normalizedRecords.filter(frame => frame.lane === "inbound").length, out: normalizedRecords.filter(frame => frame.lane === "outbound").length };
  return {
    frames: normalizedRecords,
    sourceFrameCount: document.frames.length,
    sourceCommandCount: commandRows.length,
    unprovenCommandCount: commandRows.length - actualCommands.length,
    directionCounts,
    cursorReferences,
    ordering: "per-direction stream order only; cross-direction interleaving unavailable",
  };
}

function physicalScope(frame) {
  const custody = frame.ticketAlias != null && frame.custodianNonceAlias != null;
  const values = [frame.sessionAlias, frame.operationAlias, frame.generationAlias,
    ...(custody ? [frame.ticketAlias, frame.custodianNonceAlias] : [frame.sourceEpochAlias])];
  return values.some(value => value === null) ? null : { custody };
}

function samePhysicalScope(request, response, scope) {
  return response.sessionAlias === request.sessionAlias && response.operationAlias === request.operationAlias
    && response.generationAlias === request.generationAlias
    && (scope.custody ? response.ticketAlias === request.ticketAlias && response.custodianNonceAlias === request.custodianNonceAlias
      : response.sourceEpochAlias === request.sourceEpochAlias);
}

function correlateFrames(frames, namespace = "rpc") {
  const isRequest = frame => namespace === "rpc" ? frame.rpcRole === "request" && frame.rpcIdAlias
    : frame.protocolNamespace === namespace && ["request", "input"].includes(frame.protocolRole) && frame.protocolIdAlias;
  const isResponse = frame => namespace === "rpc" ? frame.rpcRole === "response" && frame.rpcIdAlias
    : frame.protocolNamespace === namespace && ["response", "echo"].includes(frame.protocolRole) && frame.protocolIdAlias;
  const idOf = frame => namespace === "rpc" ? frame.rpcIdAlias : frame.protocolIdAlias;
  const requests = frames.filter(isRequest);
  const responses = frames.filter(isResponse);
  const pairs = [];
  const unmatchedRequests = [];
  const unmatchedResponses = [];
  const usedResponses = new Set();
  for (const request of requests) {
    const scope = physicalScope(request);
    if (!scope) {
      unmatchedRequests.push({ lane: request.lane, sequence: request.sequence, reason: "missing-physical-scope" });
      continue;
    }
    const candidates = responses.filter(candidate => idOf(candidate) === idOf(request) && candidate.lane !== request.lane
      && samePhysicalScope(request, candidate, scope));
    if (candidates.length !== 1 || usedResponses.has(candidates[0] ?? null)) {
      unmatchedRequests.push({ lane: request.lane, sequence: request.sequence, reason: candidates.length > 1 ? "ambiguous-response" : candidates.length === 0 ? "no-opposite-response-in-scope" : "response-already-associated" });
      continue;
    }
    const response = candidates[0];
    usedResponses.add(response);
    pairs.push({ request: { lane: request.lane, sequence: request.sequence }, response: { lane: response.lane, sequence: response.sequence },
      ...(namespace === "rpc" ? { rpcIdAlias: request.rpcIdAlias, rpcIdType: request.rpcIdType }
        : { protocolNamespace: namespace, protocolIdAlias: request.protocolIdAlias, protocolIdType: request.protocolIdType }),
      scope: { session: request.sessionAlias, operation: request.operationAlias, generation: request.generationAlias, sourceEpoch: request.sourceEpochAlias, ticket: request.ticketAlias, custodianNonce: request.custodianNonceAlias },
      basis: scope.custody ? "same-native-process-custody" : "recorded-source-epoch" });
  }
  for (const response of responses) if (!usedResponses.has(response)) unmatchedResponses.push({ lane: response.lane, sequence: response.sequence });
  return {
    requests: requests.length,
    responses: responses.length,
    matchedPairs: pairs.length,
    unmatchedRequests,
    unmatchedResponses,
    pairs,
    completeness: !frames.some(frame => frame.lane === "inbound") || !frames.some(frame => frame.lane === "outbound")
      ? "incomplete-direction-capture"
      : unmatchedRequests.length || unmatchedResponses.length ? "two-direction-capture-with-unresolved-associations" : "two-direction-capture",
  };
}

function normalizeLedgerOutput(document, cursorReferences, aliases, redactions) {
  const isLegacyBatch = document?.schema === "gogoke.37.actual-product-api-batch.v1" && Array.isArray(document.events);
  const isDirectReadback = document?.schema === "gogoke.37.private-e2e-ledger.v1" && Array.isArray(document.sessions);
  if (!isLegacyBatch && !isDirectReadback) {
    fail("normalized output has an unsupported product-ledger schema");
  }
  const sourceRows = isLegacyBatch
    ? document.events.map(event => ({ event, rawCursor: event?._meta?.rawSourceCursor, sourceEpoch: event?._meta?.rawSourceEpoch, sessionId: event?._meta?.sessionId ?? event?.sessionId }))
    : document.sessions.flatMap(session => (Array.isArray(session.normalized) ? session.normalized : []).map(row => ({
      event: row.update, rawCursor: row.update?._meta?.rawSourceCursor, sourceEpoch: row.sourceEpoch,
      sessionId: session.sessionId, operationId: row.operationId, generation: row.generation,
      ledgerCursor: row.cursor, ledgerSourceCursor: row.ledgerSourceCursor ?? row.sourceCursor,
    })));
  let exactScoped = 0;
  let uniqueCursorFallback = 0;
  let ambiguous = 0;
  let unmatched = 0;
  const events = sourceRows.map((row, index) => {
    const candidates = row.rawCursor != null ? cursorReferences.get(String(row.rawCursor)) || [] : [];
    const filtered = candidates.filter(candidate =>
      (row.sourceEpoch == null || String(candidate.sourceEpoch) === String(row.sourceEpoch))
      && (row.sessionId == null || String(candidate.sessionId) === String(row.sessionId))
      && (row.operationId == null || String(candidate.operationId) === String(row.operationId))
      && (row.generation == null || String(candidate.generation) === String(row.generation)));
    const sourceFrame = filtered.length === 1 ? { lane: filtered[0].lane, sequence: filtered[0].sequence } : null;
    let linkMethod = null;
    if (sourceFrame) {
      if (row.sourceEpoch != null) { exactScoped += 1; linkMethod = "sourceEpoch+sourceCursor"; }
      else if (row.sessionId != null || row.operationId != null || row.generation != null) { exactScoped += 1; linkMethod = "availableScope+sourceCursor"; }
      else { uniqueCursorFallback += 1; linkMethod = "unique-sourceCursor-fallback"; }
    } else if (filtered.length > 1 || (candidates.length > 1 && row.sourceEpoch == null && row.sessionId == null && row.operationId == null && row.generation == null)) {
      ambiguous += 1;
    } else {
      unmatched += 1;
    }
    return {
      sequence: index + 1,
      sourceFrame,
      linkMethod,
      sourceEpochAlias: row.sourceEpoch == null ? null : aliases.alias("epoch", row.sourceEpoch),
      rawSourceCursorAlias: row.rawCursor == null ? null : aliases.alias("cursor", row.rawCursor),
      ledgerCursorAlias: row.ledgerCursor == null ? null : aliases.alias("ledger-cursor", row.ledgerCursor),
      ledgerSourceCursorAlias: row.ledgerSourceCursor == null ? null : aliases.alias("ledger-source-cursor", row.ledgerSourceCursor),
      sessionAlias: row.sessionId == null ? null : aliases.alias("session", row.sessionId),
      event: sanitize(row.event, `normalized-event:${index + 1}`, aliases, redactions, ""),
    };
  });
  return { events, count: events.length, linkedToRawFrames: events.filter(event => event.sourceFrame !== null).length, linkCoverage: { exactScoped, uniqueCursorFallback, ambiguous, unmatched, complete: ambiguous === 0 && unmatched === 0 } };
}

function fieldTypes(value, prefix = "$") {
  const fields = new Map();
  function walk(node, current) {
    if (Array.isArray(node)) {
      fields.set(`${current}[]`, "array-item");
      for (const item of node) walk(item, `${current}[]`);
      return;
    }
    if (node && typeof node === "object") {
      fields.set(current, "object");
      for (const [key, child] of Object.entries(node)) walk(child, `${current}/${jsonPointerPart(key)}`);
      return;
    }
    fields.set(current, node === null ? "null" : typeof node);
  }
  walk(value, prefix);
  return fields;
}

function collectMethods(frames) {
  return frames.map(frame => typeof frame.message?.method === "string" ? frame.message.method : "<json-rpc-response-or-unknown>");
}

function collectItemTypes(frames) {
  const found = [];
  const visit = value => {
    if (Array.isArray(value)) { for (const item of value) visit(item); return; }
    if (!value || typeof value !== "object") return;
    if (typeof value.type === "string") found.push(value.type);
    for (const child of Object.values(value)) visit(child);
  };
  for (const frame of frames) visit(frame.message?.params ?? {});
  return found;
}

function writePrivateJson(candidate, description, value) {
  const absolute = path.resolve(candidate);
  let repoReal;
  try { repoReal = fs.realpathSync(REPO_ROOT); } catch { fail("repository root is unavailable"); }
  let ancestor = path.dirname(absolute);
  while (!fs.existsSync(ancestor)) {
    const parent = path.dirname(ancestor);
    if (parent === ancestor) fail(`${description} directory is unavailable`);
    ancestor = parent;
  }
  let nearestReal;
  try { nearestReal = fs.realpathSync(ancestor); } catch { fail(`${description} directory is unavailable`); }
  if (isInside(repoReal, nearestReal) || isInside(repoReal, absolute)) {
    fail(`${description} must stay outside the repository`);
  }
  try { fs.mkdirSync(path.dirname(absolute), { recursive: true }); } catch { fail(`${description} directory could not be created`); }
  let outputParent;
  try { outputParent = fs.realpathSync(path.dirname(absolute)); } catch { fail(`${description} directory is unavailable`); }
  if (isInside(repoReal, outputParent)) fail(`${description} must stay outside the repository`);
  try { fs.writeFileSync(absolute, `${JSON.stringify(value, null, 2)}\n`, { encoding: "utf8", flag: "wx" }); }
  catch { fail(`${description} could not be created without replacing an existing file`); }
}

function ensureNoSensitiveResidue(value) {
  const leaks = [];
  const walk = (node, pointer = "$") => {
    if (Array.isArray(node)) { node.forEach((item, index) => walk(item, `${pointer}/${index}`)); return; }
    if (!node || typeof node !== "object") {
      if (typeof node === "string" && safeStringValue(node)) leaks.push(pointer);
      return;
    }
    for (const [key, child] of Object.entries(node)) walk(child, `${pointer}/${jsonPointerPart(key)}`);
  };
  walk(value);
  if (leaks.length) fail("privacy scan rejected a sanitized bundle; no output was written");
}

function importCapture(options) {
  const rawInput = readPrivateJson(requireOption(options, "frames"), "raw frame input");
  const normalizedInput = readPrivateJson(requireOption(options, "normalized"), "normalized output input");
  const failurePath = options.get("failure-evidence");
  const failureInput = failurePath ? readPrivateJson(failurePath, "failure evidence input") : null;
  const direction = options.get("direction");
  if (direction && !VALID_DIRECTIONS.has(direction)) fail("--direction must be in or out");
  const outcome = requireOption(options, "outcome");
  if (!["success", "failed", "unknown"].includes(outcome)) fail("--outcome must be success, failed, or unknown");
  const cliVersion = requireOption(options, "cli-version");
  if (!/^\d+\.\d+\.\d+(?:[-+][A-Za-z0-9.-]+)?$/.test(cliVersion)) fail("--cli-version must be a version string");
  const captureId = requireOption(options, "capture-id");
  if (!/^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/.test(captureId)) fail("--capture-id must be a short identifier");

  if (outcome === "failed" && !failureInput) fail("failed outcome requires --failure-evidence with the original CLI tool error payload");
  const failureClass = failureInput ? classifyFailureEvidence(failureInput.json) : { classification: "not-provided", matchedOutputFields: 0 };
  if (outcome === "success" && failureClass.classification === "code_mode_host_spawn_access_denied") {
    fail("capture outcome success conflicts with original tool failure evidence");
  }
  if (outcome === "failed" && failureClass.matchedOutputFields === 0) {
    fail("failed outcome needs the original CLI tool error payload");
  }

  const aliases = new AliasRegistry();
  const redactions = [];
  const normalizedRaw = normalizeRawFrames(rawInput.json, direction, aliases, redactions);
  const outputEvents = normalizeLedgerOutput(normalizedInput.json, normalizedRaw.cursorReferences, aliases, redactions);
  const outCount = normalizedRaw.directionCounts.out;
  const inCount = normalizedRaw.directionCounts.in;
  const directionComplete = outCount > 0 && inCount > 0;
  const baselineStatus = outcome === "failed" ? "NOT_READY_FAILED_CAPTURE" : !directionComplete ? "NOT_READY_MISSING_DIRECTION" : "REVIEW_REQUIRED";
  const correlation = correlateFrames(normalizedRaw.frames);
  const claudeControlCorrelation = correlateFrames(normalizedRaw.frames, "claude-control");
  const claudeUserEchoCorrelation = correlateFrames(normalizedRaw.frames, "claude-user");

  // Attach only the raw cursor-to-frame correlation data, never original frame text.
  const capture = {
    schema: BUNDLE_SCHEMA,
    manifest: {
      captureId,
      cliVersion,
      cliBinarySha256: parseSha(requireOption(options, "binary-sha256"), "binary-sha256"),
      codeModeHostSha256: parseSha(requireOption(options, "helper-sha256"), "helper-sha256"),
      officialSourceSha: parseSha(requireOption(options, "source-sha"), "source-sha", [40, 64]),
      outcome,
      failureClass: failureClass.classification,
      baselineStatus,
      acceptance: "NOT_ASSESSED",
      rawWasCopied: false,
      inputPathsStored: false,
      inputSha256: {
        privateFrames: rawInput.sha256,
        normalizedOutput: normalizedInput.sha256,
        privateToolError: failureInput?.sha256 ?? null,
      },
      directionCoverage: { out: outCount, in: inCount, complete: directionComplete, missing: [...(outCount ? [] : ["out"]), ...(inCount ? [] : ["in"])] },
      sourceRecordCounts: { frames: normalizedRaw.sourceFrameCount, commands: normalizedRaw.sourceCommandCount, excludedUnprovenCommands: normalizedRaw.unprovenCommandCount },
      ordering: normalizedRaw.ordering,
      requestResponseCorrelation: correlation,
      claudeControlCorrelation,
      claudeUserEchoCorrelation,
      normalizedFrameLinks: { matched: outputEvents.linkedToRawFrames, totalEvents: outputEvents.count, ...outputEvents.linkCoverage },
      redactions: { count: redactions.length, fields: redactions },
      evidenceProvenance: "Controller-provided private direct frame capture and matching normalized-output/failure records",
    },
    observations: {
      frames: normalizedRaw.frames,
      normalizedEvents: outputEvents.events,
    },
  };
  ensureNoSensitiveResidue(capture);
  writePrivateJson(requireOption(options, "out"), "sanitized private capture bundle", capture);
  process.stdout.write(`capture imported: outcome=${outcome}; frames=${normalizedRaw.frames.length}; directions=out:${outCount},in:${inCount}; normalizedEvents=${outputEvents.count}; baseline=${baselineStatus}; acceptance=NOT_ASSESSED\n`);
}

function setDifference(left, right) {
  const rightSet = new Set(right);
  return [...new Set(left)].filter(value => !rightSet.has(value)).sort();
}

function multisetDifference(left, right) {
  const counts = new Map();
  for (const item of right) counts.set(item, (counts.get(item) || 0) + 1);
  const removed = [];
  for (const item of left) {
    const count = counts.get(item) || 0;
    if (count) counts.set(item, count - 1);
    else removed.push(item);
  }
  const reverseCounts = new Map();
  for (const item of left) reverseCounts.set(item, (reverseCounts.get(item) || 0) + 1);
  const added = [];
  for (const item of right) {
    const count = reverseCounts.get(item) || 0;
    if (count) reverseCounts.set(item, count - 1);
    else added.push(item);
  }
  return { added, removed };
}

function frameSequencesByLane(frames) {
  const sequence = lane => frames.filter(frame => frame.lane === lane).sort((a, b) => a.sequence - b.sequence)
    .map(frame => `${frame.sourceKind}:${frame.message?.method || (frame.rpcRole === "response" ? "<response>" : "<unknown>")}`);
  return { inbound: sequence("inbound"), outbound: sequence("outbound") };
}

function normalizedSequence(events) {
  return events.map(entry => {
    const event = entry.event || {};
    const method = event._meta?.codexMethod || "<missing-method>";
    const update = event.sessionUpdate || "<missing-update>";
    const contentType = event.content?.type || "<no-content>";
    return `${method}|${update}|${contentType}`;
  });
}

function changedPositions(left, right) {
  const changes = [];
  const count = Math.max(left.length, right.length);
  for (let index = 0; index < count; index += 1) {
    if (left[index] !== right[index]) changes.push({ sequence: index + 1, baseline: left[index] ?? null, candidate: right[index] ?? null });
  }
  return changes;
}

function semanticValues(value) {
  const values = new Map();
  const walk = (node, pointer = "$", key = "") => {
    if (Array.isArray(node)) {
      node.forEach((item, index) => walk(item, `${pointer}/${index}`, key));
      return;
    }
    if (node && typeof node === "object") {
      for (const [childKey, child] of Object.entries(node)) {
        walk(child, `${pointer}/${jsonPointerPart(childKey)}`, childKey);
      }
      return;
    }
    if (node === null || node === REDACTED || /(?:time|timestamp|duration|elapsed|createdat|updatedat|emittedat)/i.test(key)) return;
    if (typeof node === "string" && !SEMANTIC_STRING_KEYS.has(key)) return;
    if (["string", "number", "boolean"].includes(typeof node)) values.set(pointer, node);
  };
  walk(value);
  return values;
}

function semanticValueChanges(left, right) {
  const baseline = semanticValues(left);
  const candidate = semanticValues(right);
  const paths = [...new Set([...baseline.keys(), ...candidate.keys()])].sort();
  return paths.filter(pointer => baseline.get(pointer) !== candidate.get(pointer)).map(pointer => ({
    path: pointer,
    baseline: baseline.has(pointer) ? baseline.get(pointer) : null,
    candidate: candidate.has(pointer) ? candidate.get(pointer) : null,
  }));
}

function compareBundles(options) {
  const base = readPrivateJson(requireOption(options, "baseline"), "baseline bundle").json;
  const next = readPrivateJson(requireOption(options, "candidate"), "candidate bundle").json;
  if (base?.schema !== BUNDLE_SCHEMA || next?.schema !== BUNDLE_SCHEMA) fail("baseline and candidate must be sanitized CLI protocol capture bundles");
  const baseFrames = base.observations?.frames;
  const nextFrames = next.observations?.frames;
  const baseEvents = base.observations?.normalizedEvents;
  const nextEvents = next.observations?.normalizedEvents;
  if (!Array.isArray(baseFrames) || !Array.isArray(nextFrames) || !Array.isArray(baseEvents) || !Array.isArray(nextEvents)) fail("capture bundle observations are incomplete");

  const baseFrameMethods = collectMethods(baseFrames);
  const nextFrameMethods = collectMethods(nextFrames);
  const baseEventMethods = normalizedSequence(baseEvents);
  const nextEventMethods = normalizedSequence(nextEvents);
  const baseFrameFields = fieldTypes(baseFrames);
  const nextFrameFields = fieldTypes(nextFrames);
  const baseEventFields = fieldTypes(baseEvents);
  const nextEventFields = fieldTypes(nextEvents);
  const changedTypes = [...new Set([...baseFrameFields.keys(), ...nextFrameFields.keys(), ...baseEventFields.keys(), ...nextEventFields.keys()])]
    .sort()
    .filter(pointer => baseFrameFields.get(pointer) !== nextFrameFields.get(pointer) || baseEventFields.get(pointer) !== nextEventFields.get(pointer))
    .map(pointer => ({ path: pointer, baselineFrameType: baseFrameFields.get(pointer) ?? null, candidateFrameType: nextFrameFields.get(pointer) ?? null, baselineEventType: baseEventFields.get(pointer) ?? null, candidateEventType: nextEventFields.get(pointer) ?? null }));
  const framePaths = [...new Set([...baseFrameFields.keys(), ...nextFrameFields.keys()])].sort();
  const eventPaths = [...new Set([...baseEventFields.keys(), ...nextEventFields.keys()])].sort();
  const frameFieldChanges = { added: setDifference(framePaths.filter(value => nextFrameFields.has(value)), framePaths.filter(value => baseFrameFields.has(value))), removed: setDifference(framePaths.filter(value => baseFrameFields.has(value)), framePaths.filter(value => nextFrameFields.has(value))) };
  const eventFieldChanges = { added: setDifference(eventPaths.filter(value => nextEventFields.has(value)), eventPaths.filter(value => baseEventFields.has(value))), removed: setDifference(eventPaths.filter(value => baseEventFields.has(value)), eventPaths.filter(value => nextEventFields.has(value))) };
  const frameMethodChanges = multisetDifference(baseFrameMethods, nextFrameMethods);
  const frameTypeChanges = multisetDifference(collectItemTypes(baseFrames), collectItemTypes(nextFrames));
  const baseFrameStreams = frameSequencesByLane(baseFrames);
  const nextFrameStreams = frameSequencesByLane(nextFrames);
  const frameOrderChanges = {
    inbound: changedPositions(baseFrameStreams.inbound, nextFrameStreams.inbound),
    outbound: changedPositions(baseFrameStreams.outbound, nextFrameStreams.outbound),
  };
  const eventOrderChanges = changedPositions(baseEventMethods, nextEventMethods);
  const semanticChanges = semanticValueChanges(
    { frames: baseFrames, normalizedEvents: baseEvents },
    { frames: nextFrames, normalizedEvents: nextEvents },
  );
  const report = {
    schema: REPORT_SCHEMA,
    baseline: { cliVersion: base.manifest.cliVersion, outcome: base.manifest.outcome, baselineStatus: base.manifest.baselineStatus },
    candidate: { cliVersion: next.manifest.cliVersion, outcome: next.manifest.outcome, baselineStatus: next.manifest.baselineStatus },
    differences: {
      frameMethods: frameMethodChanges,
      frameTypes: frameTypeChanges,
      frameFields: frameFieldChanges,
      normalizedEventFields: eventFieldChanges,
      changedFieldTypes: changedTypes,
      frameSequence: { baseline: baseFrameStreams, candidate: nextFrameStreams, changedPositions: frameOrderChanges },
      normalizedEventSequence: { baseline: baseEventMethods, candidate: nextEventMethods, changedPositions: eventOrderChanges },
      semanticValueChanges: semanticChanges,
      responseAssociations: { baseline: base.manifest.requestResponseCorrelation, candidate: next.manifest.requestResponseCorrelation },
      claudeControlAssociations: { baseline: base.manifest.claudeControlCorrelation ?? null, candidate: next.manifest.claudeControlCorrelation ?? null },
      claudeUserEchoes: { baseline: base.manifest.claudeUserEchoCorrelation ?? null, candidate: next.manifest.claudeUserEchoCorrelation ?? null },
      directionCoverage: { baseline: base.manifest.directionCoverage, candidate: next.manifest.directionCoverage },
      redactionCounts: { baseline: base.manifest.redactions?.count ?? null, candidate: next.manifest.redactions?.count ?? null },
    },
    disposition: frameOrderChanges.inbound.length || frameOrderChanges.outbound.length || eventOrderChanges.length || semanticChanges.length || frameMethodChanges.added.length || frameMethodChanges.removed.length || frameTypeChanges.added.length || frameTypeChanges.removed.length || frameFieldChanges.added.length || frameFieldChanges.removed.length || eventFieldChanges.added.length || eventFieldChanges.removed.length || changedTypes.length ? "DIFFERENCES_RECORDED" : "NO_STRUCTURAL_DIFFERENCE_OBSERVED",
    acceptance: "NOT_ASSESSED",
    note: "A comparison report records observed changes only. It never grants PASS or makes a failed/incomplete capture an accepted baseline.",
  };
  ensureNoSensitiveResidue(report);
  writePrivateJson(requireOption(options, "out"), "private comparison report", report);
  process.stdout.write(`comparison recorded: ${report.disposition}; acceptance=NOT_ASSESSED; frameMethodsAdded=${frameMethodChanges.added.length}; frameMethodsRemoved=${frameMethodChanges.removed.length}; inboundOrderChanges=${frameOrderChanges.inbound.length}; outboundOrderChanges=${frameOrderChanges.outbound.length}; normalizedSequenceChanges=${eventOrderChanges.length}\n`);
}

function main() {
  const { command, options } = parseArgs(process.argv.slice(2));
  if (command === "import") importCapture(options);
  else compareBundles(options);
}

try {
  main();
} catch (error) {
  const message = error instanceof Error ? error.message : "operation failed";
  process.stderr.write(`cli-protocol-golden: ${message}\n`);
  process.exitCode = 2;
}
