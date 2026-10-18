// V06's User bounds and real model-origin child path on the installed product.
// Importing this module performs no product operation.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { id, sha256 } from './product-cdp.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const atom = value => typeof value === 'string' && /^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/.test(value);
const check = (value, reason) => { if (!value) throw Error(reason); };
const equal = (a, b) => JSON.stringify(a, (_, value) =>
  value && !Array.isArray(value) && typeof value === 'object'
    ? Object.fromEntries(Object.entries(value).sort(([left], [right]) => left.localeCompare(right)))
    : value) === JSON.stringify(b, (_, value) =>
  value && !Array.isArray(value) && typeof value === 'object'
    ? Object.fromEntries(Object.entries(value).sort(([left], [right]) => left.localeCompare(right)))
    : value);

export async function runV06NativeUserCases(product, config, journal, hostRoot) {
  const c = config.v06NativeUser;
  const unavailable = !c || !hostRoot || hostRoot.product !== product || hostRoot.journal !== journal ||
    typeof hostRoot.originalModelAttempt !== 'function' ||
    typeof hostRoot.normalCloseV06ReadbackRestart !== 'function';
  if (unavailable) return { state: 'NOT_RUN', reason: 'Original installed H model and normal-close readback callbacks are unavailable.', acceptance: false };
  check(process.platform === 'win32' && c.lifecycleOwnership === 'EXCLUSIVE_M2_V06_NATIVE_USER' &&
    config.repositoryId === 'gogokeSeatTestbed' && c.domainId === config.domainId &&
    c.parentSeatId === config.seatId && c.instanceId === config.instanceId &&
    ['templateId', 'directSeatId', 'childSeatId', 'deniedChildSeatId', 'model', 'effort', 'permissionTier']
      .every(key => atom(c[key])) &&
    new Set([c.parentSeatId, c.directSeatId, c.childSeatId, c.deniedChildSeatId]).size === 4 &&
    ['READ_ONLY', 'NO_NETWORK', 'ISOLATED_WRITE', 'NETWORKED_WRITE'].includes(c.permissionTier) &&
    product.tester?.page.url() === product.endpoint.url &&
    journal.connectionBackend?.agentActs === 0 && journal.connectionBackend?.telemetryDisabled === true &&
    !journal.v06NativeUser,
  'V06 requires exclusive real test identities, installed User ingress and original H callbacks');
  const otherSeats = new Set([config.childSeatId, config.sideChat?.sourceSeatId,
    config.sideChat?.sideSeatId, ...(config.providerCases ?? []).map(row => row.seatId)]);
  check([c.directSeatId, c.childSeatId, c.deniedChildSeatId].every(seat => !otherSeats.has(seat)),
    'V06 targets overlap an existing test seat');
  const record = { schema: 'gogoke.37.m2-v06-native-user.v1', state: 'RUNNING', acceptance: false,
    sourceCommit: config.sourceCommit, domainId: c.domainId, repositoryId: config.repositoryId,
    stateRoot: path.resolve(config.stateRoot), evidenceDirectory: path.resolve(config.evidenceDirectory),
    candidateVersion: config.version, candidateInstalledSha256: config.installedSha256,
    driverSha256: sha256(path.join(here, 'm2-v06-native-user.mjs')),
    readerSha256: sha256(path.join(here, 'm2-v06-readback.py')),
    fixture: { ...c }, operations: {}, modelAttempts: [], readbacks: [] };
  journal.v06NativeUser = record; product.save();

  const operation = async (name, target, payload, revision, allowed) => {
    const request = { schema: 'gogoke.37.operations.v1', family: 'K-SEAT', operation: name,
      requestId: id('m2V06'), domainId: c.domainId, targetId: target,
      expectedRevision: String(revision), payload };
    const rawFrame = JSON.stringify(request);
    const entry = { request, rawFrame, startedAt: new Date().toISOString(), receipt: null };
    journal.operations.push(entry); product.save();
    try {
      entry.rawReceipt = await product.evaluate(`window.__TAURI_INTERNALS__.invoke('gogoke_design37_user_operation',{frame:${JSON.stringify(rawFrame)}})`);
      entry.receipt = JSON.parse(entry.rawReceipt); entry.finishedAt = new Date().toISOString(); product.save();
    } catch (error) { entry.originalError = String(error?.stack ?? error); product.save(); throw error; }
    const reply = entry.receipt;
    check(reply.schema === request.schema && reply.family === 'K-SEAT' && reply.operation === name &&
      reply.targetId === target && reply.requestId === request.requestId &&
      /^\d+$/.test(reply.previousRevision) && /^\d+$/.test(reply.revision) &&
      allowed.includes(reply.status), `Original K-SEAT/${name} returned ${reply.status}; no replay`);
    record.operations[name] ??= [];
    record.operations[name].push(entry.request.requestId); product.save();
    return reply;
  };
  const card = async seatId => {
    let reply = await operation('state-card', seatId, {}, '0', ['APPLIED', 'STALE']);
    if (reply.status === 'STALE') reply = await operation('state-card', seatId, {}, reply.revision, ['APPLIED']);
    check(reply.status === 'APPLIED', `Original ${seatId} state-card was not read`);
    return reply;
  };
  const readback = async phase => {
    const reference = await hostRoot.normalCloseV06ReadbackRestart(phase);
    check(reference && path.basename(reference.file) === reference.file && /^[a-f0-9]{64}$/.test(reference.sha256),
      `Original ${phase} immutable readback reference is missing`);
    const file = path.resolve(config.evidenceDirectory, reference.file);
    check(path.dirname(file) === path.resolve(config.evidenceDirectory) && sha256(file) === reference.sha256,
      `Original ${phase} immutable readback bytes differ`);
    const proof = JSON.parse(fs.readFileSync(file, 'utf8').replace(/^\uFEFF/, ''));
    check(proof.schema === 'gogoke.37.private-m2-v06-readback.v1' && proof.phase === phase &&
      proof.caseId === journal.caseId && proof.sourceCommit === config.sourceCommit &&
      proof.readerSha256 === record.readerSha256 && proof.acceptance === false &&
      proof.measurementPreservedDatabaseBytes === true &&
      equal(proof.candidateInstalledSha256, config.installedSha256) &&
      proof.normalClose?.exitCode === 0 && proof.normalClose.forceKill === false,
    `Original ${phase} immutable readback is not this normally closed candidate`);
    record.readbacks.push({ phase, ...reference }); product.save();
    return proof;
  };

  await product.custody(); product.verifyBytes();
  const parent = await card(c.parentSeatId);
  check(parent.result.layer === 'USER' && parent.result.state === 'IDLE' &&
    parent.result.instanceId === c.instanceId && parent.result.settings?.model === c.model &&
    (parent.result.settings?.effort ?? parent.result.settings?.reasoningEffort) === c.effort &&
    parent.result.settings?.permissionTier === c.permissionTier,
  'Original parent must be IDLE on its actual test instance and configured model/permission');
  record.parentCard = parent; product.save();
  const baseline = await readback('baseline');
  check(baseline.parentRevision === parent.revision && baseline.childCount === 0 &&
    baseline.projectCap > 0 && baseline.hostLimit > 0 && baseline.targetsAbsent === true &&
    baseline.templateMatchesScope === true,
  'Original baseline does not establish empty child capacity and matching source template');
  record.baseline = record.readbacks.at(-1); record.baselineProof = baseline; product.save();

  const scope = { instanceIds: [c.instanceId], models: [c.model],
    reasoningEfforts: [c.effort], maxPermissionTier: c.permissionTier, maxConcurrent: 1 };
  const { maxConcurrent, ...missingCap } = scope;
  const deniedMissing = await operation('set-orchestration-bounds', c.parentSeatId,
    missingCap, parent.revision, ['DENIED']);
  check(deniedMissing.revision === parent.revision && deniedMissing.previousRevision === parent.revision,
    'Missing seat concurrency cap changed the original parent');
  const afterMissing = await card(c.parentSeatId);
  check(afterMissing.revision === parent.revision &&
    equal(afterMissing.result.settings, parent.result.settings),
  'Missing-cap refusal changed the live parent settings');
  const bounded = await operation('set-orchestration-bounds', c.parentSeatId,
    scope, parent.revision, ['APPLIED']);
  check(bounded.result.layer === 'USER' && bounded.result.state === 'IDLE' &&
    equal(bounded.result.settings?.orchestrationScope, scope) &&
    BigInt(bounded.revision) === BigInt(parent.revision) + 1n,
  'Original User bounds receipt did not set one direct-child slot');
  record.boundsReceipt = bounded; product.save();

  const direct = await operation('create-from-template', c.directSeatId,
    { layer: 'USER', templateId: c.templateId }, '0', ['APPLIED']);
  check(direct.revision === '1' && direct.result.layer === 'USER' && direct.result.state === 'IDLE' &&
    direct.result.templateId === c.templateId, 'Original User template direct-seat receipt differs');
  record.directReceipt = direct; product.save();

  const modelCreate = async (phase, targetId, expectedStatus) => {
    const args = { operation: 'create-from-template', targetId, expectedRevision: '0',
      payload: { layer: 'LEAD', templateId: c.templateId, instanceId: c.instanceId } };
    const prompt = `Use exactly one native gogoke_seat tool call with these exact JSON arguments: ${JSON.stringify(args)}. ` +
      'Do not call another tool or claim a result without the native tool response.';
    const observed = await hostRoot.originalModelAttempt({ phase, prompt, domainId: c.domainId,
      parentSeatId: c.parentSeatId, expectedArguments: args,
      sessionId: record.modelAttempts[0]?.sessionId ?? null });
    check(observed && atom(observed.sessionId) && observed.domainId === c.domainId &&
      observed.parentSeatId === c.parentSeatId && atom(observed.sendRequestId) && atom(observed.turnId) &&
      (!record.modelAttempts.length || observed.sessionId === record.modelAttempts[0].sessionId),
    'Original H callback did not return the same authenticated parent session and exact turn IDs');
    const attempt = { phase, expectedStatus, targetId, expectedArguments: args, prompt,
      domainId: c.domainId, parentSeatId: c.parentSeatId,
      sessionId: observed.sessionId, sendRequestId: observed.sendRequestId, turnId: observed.turnId };
    record.modelAttempts.push(attempt); product.save();
    return attempt;
  };
  await modelCreate('first-child', c.childSeatId, 'APPLIED');
  const child = await card(c.childSeatId);
  check(child.result.layer === 'LEAD' && child.result.state === 'IDLE' &&
    child.result.instanceId === c.instanceId && child.result.templateId === c.templateId,
  'Original model call did not create the exact first LEAD child');
  record.childCard = child; product.save();
  await modelCreate('over-cap-child', c.deniedChildSeatId, 'DENIED');
  record.state = 'FLOW_COMPLETE_FINAL_READBACK_REQUIRED'; product.save();
  const final = await readback('final');
  await product.custody(); product.verifyBytes();
  check(equal(final.rootIdentity, baseline.rootIdentity) &&
    equal(final.candidateIdentity, baseline.candidateIdentity) &&
    final.projectCap === baseline.projectCap && final.hostLimit > 0 &&
    final.childCount === 1 && final.deniedTargetAbsent === true &&
    final.originalModelReceipts === true && final.originalUserReceipts === true,
  'Original immutable V06 source/count/receipt readback is incomplete');
  record.state = 'READBACK_COMPLETE_REVIEW_REQUIRED'; product.save();
  return { state: record.state, acceptance: false, remainingV06: [
    'LEAD_BOUND_REFUSALS', 'BUSY_CHANGE_AND_STOP_RECLAIM', 'SHORT_TO_LONG', 'LOAD_CAP', 'HOST_CHOICES' ] };
}
