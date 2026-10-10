// Bounded continuation for V06 on the already-installed product.
// Importing this module performs no product operation. Every write is a
// journaled original User K-SEAT request; H activity is delegated only to the
// caller's already-open, identity-checked HostRoot callbacks.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { id, sha256 } from './product-cdp.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const atom = value => typeof value === 'string' && /^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/.test(value);
const check = (value, reason) => { if (!value) throw Error(reason); };

/**
 * `config.v06Remaining` is a private, source-bound fixture. `hostRoot` must
 * describe the same already-open installed-product session and provide:
 * - beginBusyWindow({seatId, domainId}) -> original H/session identity;
 * - stopAndReleaseBusyWindow(handle) -> actual H StopFact plus same-session
 *   admission-release receipts;
 * - originalModelAttempt({phase,prompt,domainId,parentSeatId,sessionId}) for
 *   each capacity child attempt, using the existing authenticated H session.
 * - verifySeatGrantFacts(fixture) -> original closed/readback count and scope;
 * - verifyAdmissionFacts(fixture) -> original active H-claim IDs and limits;
 * - runModelAdmissionBoundary(plan) -> independent reserve/commit exercise
 *   with original session lifecycle and StopFact/release cleanup.
 * No callback may log in, retry an uncertain operation, substitute USER for
 * H, or close the product. Root still owns the final normal close/readback.
 */
export async function runV06RemainingCases(product, config, journal, hostRoot) {
  const c = config.v06Remaining;
  const unavailable = !c || process.platform !== 'win32' || !product?.tester ||
    hostRoot?.product !== product || hostRoot?.journal !== journal ||
    typeof hostRoot.beginBusyWindow !== 'function' ||
    typeof hostRoot.stopAndReleaseBusyWindow !== 'function' ||
    typeof hostRoot.originalModelAttempt !== 'function' ||
    typeof hostRoot.verifySeatGrantFacts !== 'function' ||
    typeof hostRoot.normalCloseV06ReadbackRestart !== 'function';
  if (unavailable) return { state: 'NOT_RUN', acceptance: false,
    reason: 'Needs the original installed User bridge, existing H session, real busy/stop-release callbacks, and immutable normal-close reader.' };

  check(c.lifecycleOwnership === 'EXCLUSIVE_M2_V06_REMAINING' &&
    config.repositoryId === 'gogokeSeatTestbed' && c.domainId === config.domainId &&
    product.config.domainId === c.domainId && product.tester.page.url() === product.endpoint.url &&
    journal.connectionBackend?.agentActs === 0 && journal.connectionBackend?.telemetryDisabled === true &&
    !journal.v06Remaining && /^[a-f0-9]{40}$/.test(config.sourceCommit),
  'V06 continuation requires the original installed candidate, disjoint M2 fixture and no prior case record');

  const fixtureIds = [c.userParentSeatId, c.busyLeadSeatId, c.shortSeatId,
    c.capacityParentSeatId, ...(c.seatGrant?.targetSeatIds ?? [])];
  const admissionSeatIds = c.modelAdmission?.seatIds ?? [];
  const existingIds = new Set([config.seatId, config.childSeatId, config.v06NativeUser?.parentSeatId,
    config.v06NativeUser?.directSeatId, config.v06NativeUser?.childSeatId,
    config.v06NativeUser?.deniedChildSeatId, config.sideChat?.sourceSeatId,
    config.sideChat?.sideSeatId, ...(config.providerCases ?? []).map(row => row.seatId),
    ...(config.seatManagement?.projects ?? []).map(row => row.seatId),
    config.seatManagement?.leadSeat?.seatId].filter(Boolean));
  check(fixtureIds.every(atom) && new Set(fixtureIds).size === fixtureIds.length &&
    fixtureIds.every(value => !existingIds.has(value)) &&
    Array.isArray(admissionSeatIds) && admissionSeatIds.every(atom) &&
    new Set(admissionSeatIds).size === admissionSeatIds.length &&
    admissionSeatIds.every(value => ![c.userParentSeatId, c.busyLeadSeatId,
      c.shortSeatId, c.capacityParentSeatId].includes(value) && !existingIds.has(value)) &&
    c.userParentSeatId !== c.busyLeadSeatId && c.userParentSeatId !== c.capacityParentSeatId &&
    c.busyLead?.domainId === c.domainId && c.seatGrant?.domainId === c.domainId &&
    c.seatGrant?.parentSeatId === c.capacityParentSeatId && atom(c.seatGrant?.sessionId) &&
    atom(c.seatGrant?.templateId) && atom(c.seatGrant?.instanceId) &&
    Number.isInteger(c.seatGrant?.scopeCap) && c.seatGrant.scopeCap > 0 &&
    Number.isInteger(c.seatGrant?.baselineChildCount) && c.seatGrant.baselineChildCount >= 0 &&
    Array.isArray(c.seatGrant?.targetSeatIds) && c.seatGrant.targetSeatIds.length > 0 &&
    c.busyLead?.instanceId,
  'Fixture must bind disjoint original seat identities and source-backed OrchestrationScope child-count facts');

  const record = { schema: 'gogoke.37.m2-v06-remaining.v1', state: 'RUNNING', acceptance: false,
    sourceCommit: config.sourceCommit, domainId: c.domainId, repositoryId: config.repositoryId,
    stateRoot: path.resolve(config.stateRoot), evidenceDirectory: path.resolve(config.evidenceDirectory),
    candidateVersion: config.version, candidateInstalledSha256: config.installedSha256,
    driverSha256: sha256(path.join(here, 'm2-v06-remaining-win11.mjs')),
    readerSha256: sha256(path.join(here, 'm2-v06-readback.py')),
    fixture: structuredClone(c), operations: [], modelAttempts: [], readbacks: [], notRun: [] };
  journal.v06Remaining = record; product.save();
  await product.custody(); product.verifyBytes();
  const verifiedSeatGrant = await hostRoot.verifySeatGrantFacts(c.seatGrant);
  check(verifiedSeatGrant?.sourceBound === true &&
    verifiedSeatGrant.domainId === c.domainId &&
    verifiedSeatGrant.parentSeatId === c.seatGrant.parentSeatId &&
    verifiedSeatGrant.scopeCap === c.seatGrant.scopeCap &&
    verifiedSeatGrant.existingChildCount === c.seatGrant.baselineChildCount &&
    verifiedSeatGrant.countBasis === 'LEAD_NONRECLAIMED_CHILD_ROWS',
  'Seat grant capacity must come from the original OrchestrationScope and non-reclaimed child rows');
  const baselineRef = await hostRoot.normalCloseV06ReadbackRestart('baseline');
  const baseline = readProof(baselineRef, 'baseline');
  check(baseline.directCaseEvidence === false && baseline.sourceCommit === config.sourceCommit &&
    baseline.normalClose?.exitCode === 0 && baseline.normalClose.forceKill === false,
  'Original baseline does not prove a normally closed unchanged candidate before V06 writes');
  record.readbacks.push({ phase: 'baseline', ...baselineRef });
  record.baselineIdentity = { rootIdentity: baseline.rootIdentity,
    candidateIdentity: baseline.candidateIdentity,
    candidateInstalledSha256: baseline.candidateInstalledSha256 };
  product.save();
  await product.custody(); product.verifyBytes();

  const invoke = async (domainId, operation, targetId, payload, revision, allowed) => {
    const request = { schema: 'gogoke.37.operations.v1', family: 'K-SEAT', operation,
      requestId: id('m2V06R'), domainId, targetId, expectedRevision: String(revision), payload };
    const rawFrame = JSON.stringify(request);
    const entry = { request, rawFrame, startedAt: new Date().toISOString(), receipt: null };
    journal.operations.push(entry); product.save();
    try {
      entry.rawReceipt = await product.evaluate(`window.__TAURI_INTERNALS__.invoke('gogoke_design37_user_operation',{frame:${JSON.stringify(rawFrame)}})`);
      entry.receipt = JSON.parse(entry.rawReceipt); entry.finishedAt = new Date().toISOString(); product.save();
    } catch (error) { entry.originalError = String(error?.stack ?? error); product.save(); throw error; }
    const r = entry.receipt;
    check(r.schema === request.schema && r.family === request.family && r.operation === operation &&
      r.requestId === request.requestId && r.targetId === targetId && r.domainId === domainId &&
      allowed.includes(r.status), `Original ${operation} returned ${r.status}; request retained, no replay`);
    record.operations.push({ operation, domainId, targetId, requestId: request.requestId,
      status: r.status, previousRevision: r.previousRevision, revision: r.revision }); product.save();
    return r;
  };
  const card = async (seatId, domainId = c.domainId) => {
    let r = await invoke(domainId, 'state-card', seatId, {}, '0', ['APPLIED', 'STALE']);
    if (r.status === 'STALE') r = await invoke(domainId, 'state-card', seatId, {}, r.revision, ['APPLIED']);
    check(r.status === 'APPLIED',
      `Original state-card failed for ${seatId}`);
    return r;
  };

  // A genuine SHORT fixture is required. A LONG or absent seat is never
  // converted into a synthetic short case.
  const short = await card(c.shortSeatId);
  if (short.result.kind !== 'SHORT') {
    record.notRun.push({ axis: 'SHORT_TO_LONG', reason: `Original state-card kind=${short.result.kind}; no genuine SHORT fixture.` });
  } else if (short.result.state !== 'IDLE') {
    record.notRun.push({ axis: 'SHORT_TO_LONG', reason: `Original SHORT is ${short.result.state}; promotion requires IDLE.` });
  } else {
    const promoted = await invoke(c.domainId, 'short-to-long', c.shortSeatId, {}, short.revision, ['APPLIED']);
    check(promoted.result.kind === 'LONG' && promoted.result.state === 'IDLE' &&
      BigInt(promoted.revision) === BigInt(short.revision) + 1n,
    'Original short-to-long receipt did not promote exactly the IDLE SHORT seat');
    record.shortToLong = promoted; product.save();
  }

  // The H callback must keep the real session/process busy across both
  // attempted mutations; product code decides BUSY/STALE from the exact
  // state-card revision. No synthetic state toggle is permitted.
  const busyBefore = await card(c.busyLeadSeatId);
  check(busyBefore.result.layer === 'LEAD' && busyBefore.result.state === 'IDLE' &&
    busyBefore.result.instanceId === c.busyLead.instanceId,
  'Busy fixture is not the registered IDLE LEAD on the expected instance');
  const busyHandle = await hostRoot.beginBusyWindow({ seatId: c.busyLeadSeatId, domainId: c.domainId });
  check(busyHandle && atom(busyHandle.sessionId) && busyHandle.domainId === c.domainId &&
    busyHandle.seatId === c.busyLeadSeatId && atom(busyHandle.sendRequestId) && atom(busyHandle.turnId),
  'Original H callback did not establish the exact busy LEAD session');
  record.busyHandle = busyHandle; product.save();
  const busy = await card(c.busyLeadSeatId);
  check(busy.result.state === 'BUSY' && busy.result.layer === 'LEAD',
    'Original state-card did not observe the actual H-held LEAD as BUSY');
  const capacityParent = await card(c.capacityParentSeatId);
  check(capacityParent.result.layer === 'USER' && capacityParent.result.state === 'IDLE' &&
    capacityParent.result.settings?.orchestrationScope &&
    Number.isInteger(capacityParent.result.settings.orchestrationScope.maxConcurrent),
  'Seat grant parent lacks its original USER orchestration scope');
  const boundsArgs = { operation: 'set-orchestration-bounds', targetId: c.busyLeadSeatId,
    expectedRevision: busy.revision, payload: capacityParent.result.settings.orchestrationScope };
  const boundsPrompt = `Use exactly one native gogoke_seat tool call with these exact JSON arguments: ${JSON.stringify(boundsArgs)}. Do not call another tool or claim a result without its native response.`;
  const boundsAttempt = await hostRoot.originalModelAttempt({ phase: 'lead-bounds-refusal',
    prompt: boundsPrompt, domainId: c.domainId, parentSeatId: c.busyLeadSeatId,
    sessionId: busyHandle.sessionId, expectedArguments: boundsArgs });
  check(boundsAttempt && boundsAttempt.sessionId === busyHandle.sessionId &&
    boundsAttempt.domainId === c.domainId && boundsAttempt.parentSeatId === c.busyLeadSeatId &&
    atom(boundsAttempt.sendRequestId) && atom(boundsAttempt.turnId) &&
    boundsAttempt.nativeSeatReceipt?.status === 'DENIED' &&
    boundsAttempt.nativeSeatReceipt?.operation === 'set-orchestration-bounds' &&
    boundsAttempt.nativeSeatReceipt?.targetId === c.busyLeadSeatId,
  'Original LEAD H attempt to set bounds was not natively refused');
  const afterBoundsAttempt = await card(c.busyLeadSeatId);
  check(afterBoundsAttempt.result.state === 'BUSY' && afterBoundsAttempt.revision === busy.revision,
    'Denied LEAD bounds attempt changed the busy LEAD');
  record.modelAttempts.push({ phase: 'lead-bounds-refusal', targetId: c.busyLeadSeatId,
    args: boundsArgs, sessionId: boundsAttempt.sessionId, sendRequestId: boundsAttempt.sendRequestId,
    turnId: boundsAttempt.turnId, nativeSeatReceipt: boundsAttempt.nativeSeatReceipt }); product.save();
  const targetInstance = c.busyLead.changeInstanceId;
  check(atom(targetInstance) && targetInstance !== busy.result.instanceId &&
    atom(c.busyLead.model) && atom(c.busyLead.effort) && atom(c.busyLead.permissionTier),
  'Busy change request must use explicit valid fixture values and a different existing instance');
  const changeWhileBusy = await invoke(c.domainId, 'change-instance', c.busyLeadSeatId,
    { instanceId: targetInstance, model: c.busyLead.model, effort: c.busyLead.effort,
      permissionTier: c.busyLead.permissionTier }, busy.revision, ['CONFLICT']);
  const reclaimWhileBusy = await invoke(c.domainId, 'reclaim', c.busyLeadSeatId, {}, busy.revision, ['CONFLICT']);
  check(changeWhileBusy.revision === busy.revision && reclaimWhileBusy.revision === busy.revision,
    'Busy change/reclaim refusal changed the original LEAD revision');
  const stopped = await hostRoot.stopAndReleaseBusyWindow(busyHandle);
  const sessionReceipt = (requestId, operation) => journal.operations.filter(entry =>
    entry.request?.requestId === requestId && entry.receipt?.family === 'K-SESSION' &&
    entry.receipt?.operation === operation && entry.receipt?.targetId === busyHandle.sessionId &&
    ['APPLIED', 'REPLAYED'].includes(entry.receipt?.status));
  check(stopped?.stopFact === true && stopped?.sameSessionRelease === true &&
    stopped.sessionId === busyHandle.sessionId && stopped.seatId === c.busyLeadSeatId &&
    atom(stopped.stopRequestId) && atom(stopped.releaseRequestId) &&
    sessionReceipt(stopped.stopRequestId, 'stop').length === 1 &&
    sessionReceipt(stopped.releaseRequestId, 'admission-release').length === 1,
  'Original H stop/StopFact and same-session admission release were not confirmed');
  record.stopRelease = stopped; product.save();
  const idleLead = await card(c.busyLeadSeatId);
  check(idleLead.result.state === 'IDLE' && idleLead.result.layer === 'LEAD',
    'LEAD did not become IDLE after original H stop and release');
  const reclaimed = await invoke(c.domainId, 'reclaim', c.busyLeadSeatId, {}, idleLead.revision, ['APPLIED']);
  check(reclaimed.result.state === 'RECLAIMED' && reclaimed.result.layer === 'LEAD',
    'USER reclaim after real H stop/release did not apply to the same LEAD');
  record.busyChangeReclaim = { changeWhileBusy, reclaimWhileBusy, reclaimed }; product.save();

  // Seat creation is capped by OrchestrationScope.maxConcurrent and counts
  // non-reclaimed LEAD rows for this parent. Project/host/instance admission
  // limits are a separate axis and are never substituted here.
  const grant = c.seatGrant;
  const remainingSeats = grant.scopeCap - grant.baselineChildCount;
  if (remainingSeats < 1 || grant.targetSeatIds.length < remainingSeats + 1) {
    record.notRun.push({ axis: 'SEAT_GRANT_SCOPE_CAPACITY',
      reason: 'Source-backed non-reclaimed child count leaves no tested slot or enough disjoint targets.',
      scopeCap: grant.scopeCap, baselineChildCount: grant.baselineChildCount,
      targetCount: grant.targetSeatIds.length });
  } else {
    check(capacityParent.result.settings.orchestrationScope.maxConcurrent === grant.scopeCap &&
      grant.baselineChildCount <= grant.scopeCap,
    'Original USER scope differs from the verified direct-child count fixture');
    for (const targetId of grant.targetSeatIds.slice(0, remainingSeats + 1)) {
      const absent = await invoke(c.domainId, 'state-card', targetId, {}, '0', ['CONFLICT']);
      check(absent.status === 'CONFLICT', `Capacity target ${targetId} already exists; fixture is not empty`);
    }
    for (let i = 0; i < remainingSeats + 1; i++) {
      const targetId = grant.targetSeatIds[i];
      const expectedStatus = i < remainingSeats ? 'APPLIED' : 'DENIED';
      const args = { operation: 'create-from-template', targetId, expectedRevision: '0',
        payload: { layer: 'LEAD', templateId: grant.templateId, instanceId: grant.instanceId } };
      const prompt = `Use exactly one native gogoke_seat tool call with these exact JSON arguments: ${JSON.stringify(args)}. Do not call another tool or claim a result without its native response.`;
      const observed = await hostRoot.originalModelAttempt({ phase: `capacity-${i + 1}`, prompt,
        domainId: c.domainId, parentSeatId: c.capacityParentSeatId, sessionId: grant.sessionId,
        expectedArguments: args });
      check(observed && observed.sessionId === grant.sessionId && observed.domainId === c.domainId &&
        observed.parentSeatId === c.capacityParentSeatId && atom(observed.sendRequestId) &&
        atom(observed.turnId) && observed.nativeSeatReceipt?.status === expectedStatus &&
        observed.nativeSeatReceipt?.operation === 'create-from-template' &&
        observed.nativeSeatReceipt?.targetId === targetId,
      'Capacity attempt did not use the same original authenticated H session');
      record.modelAttempts.push({ phase: `capacity-${i + 1}`, targetId, expectedStatus, args,
        sessionId: observed.sessionId, sendRequestId: observed.sendRequestId, turnId: observed.turnId }); product.save();
      const after = await invoke(c.domainId, 'state-card', targetId, {}, '0', ['APPLIED', 'CONFLICT']);
      if (expectedStatus === 'APPLIED') check(after.status === 'APPLIED' &&
        after.result.layer === 'LEAD' && after.result.state === 'IDLE' &&
        after.result.templateId === grant.templateId && after.result.instanceId === grant.instanceId,
      `Within-scope child ${targetId} is not the exact LEAD target`);
      else check(after.status === 'CONFLICT',
        'Over-scope target exists; expected absent state-card conflict');
    }
    record.seatGrantCapacity = { scopeCap: grant.scopeCap,
      baselineChildCount: grant.baselineChildCount, attemptedCreates: remainingSeats + 1,
      created: remainingSeats, refusedAtScopeCap: true }; product.save();
  }

  // Session admission uses active H claims, not child-seat rows. Its effective
  // project limit is min(Owner project cap, measured host machine limit); the
  // target instance has a separate cap. Counts include RESERVED, COMMITTED,
  // STOPPED and UNKNOWN claims. The parent session must appear in project
  // active claims, and in instance claims when it uses the measured instance.
  const admission = c.modelAdmission;
  if (!admission || typeof hostRoot.verifyAdmissionFacts !== 'function' ||
      typeof hostRoot.runModelAdmissionBoundary !== 'function') {
    record.notRun.push({ axis: 'MODEL_ADMISSION_CAPACITY',
      reason: 'Independent original K-SESSION admission facts or reserve/commit lifecycle callback is unavailable.' });
  } else {
    const facts = await hostRoot.verifyAdmissionFacts(admission);
    check(facts?.sourceBound === true && facts.domainId === c.domainId &&
      Number.isInteger(facts.projectCap) && facts.projectCap > 0 &&
      Number.isInteger(facts.hostLimit) && facts.hostLimit > 0 &&
      Number.isInteger(facts.instanceLimit) && facts.instanceLimit > 0 &&
      Array.isArray(facts.projectActiveClaims) && Array.isArray(facts.instanceActiveClaims) &&
      facts.projectActiveClaims.every(row => row && atom(row.sessionId) && atom(row.instanceId) &&
        ['RESERVED', 'COMMITTED', 'STOPPED', 'UNKNOWN'].includes(row.state)) &&
      facts.instanceActiveClaims.every(row => row && atom(row.sessionId) && atom(row.instanceId) &&
        row.instanceId === admission.instanceId &&
        ['RESERVED', 'COMMITTED', 'STOPPED', 'UNKNOWN'].includes(row.state)) &&
      new Set(facts.projectActiveClaims.map(row => row.sessionId)).size === facts.projectActiveClaims.length &&
      new Set(facts.instanceActiveClaims.map(row => row.sessionId)).size === facts.instanceActiveClaims.length &&
      facts.projectActiveClaims.some(row => row.sessionId === admission.parentSessionId) &&
      (facts.parentInstanceId !== admission.instanceId ||
        facts.instanceActiveClaims.some(row => row.sessionId === admission.parentSessionId)),
    'Admission facts must be original active-claim IDs and include the parent session');
    const effectiveProjectParallel = Math.min(facts.projectCap, facts.hostLimit);
    const projectSlots = Math.max(0, effectiveProjectParallel - facts.projectActiveClaims.length);
    const instanceSlots = Math.max(0, facts.instanceLimit - facts.instanceActiveClaims.length);
    const availableAdmissionSlots = Math.min(projectSlots, instanceSlots);
    const enoughSeats = Array.isArray(admission.seatIds) &&
      admission.seatIds.length >= availableAdmissionSlots + 1 &&
      admission.seatIds.every(atom) && new Set(admission.seatIds).size === admission.seatIds.length;
    if (!enoughSeats) {
      record.notRun.push({ axis: 'MODEL_ADMISSION_CAPACITY',
        reason: 'No distinct original child-session seats cover the verified admission slots plus one refusal.',
        effectiveProjectParallel, projectActive: facts.projectActiveClaims.length,
        instanceLimit: facts.instanceLimit, instanceActive: facts.instanceActiveClaims.length,
        availableAdmissionSlots, targetCount: admission.seatIds?.length ?? 0 });
    } else {
      const outcome = await hostRoot.runModelAdmissionBoundary({ domainId: c.domainId,
        instanceId: admission.instanceId, parentSessionId: admission.parentSessionId,
        seatIds: admission.seatIds.slice(0, availableAdmissionSlots + 1),
        expectedAdmitted: availableAdmissionSlots, expectedRefused: 1,
        effectiveProjectParallel, instanceLimit: facts.instanceLimit,
        projectActiveClaims: facts.projectActiveClaims,
        instanceActiveClaims: facts.instanceActiveClaims });
      check(outcome?.sourceBound === true && outcome.parentIncluded === true &&
        outcome.admitted === availableAdmissionSlots && outcome.refused === 1 &&
        outcome.allCreatedSessionsStoppedAndReleased === true &&
        Array.isArray(outcome.attempts) && outcome.attempts.length === availableAdmissionSlots + 1 &&
        outcome.attempts.every((attempt, index) => attempt &&
          attempt.seatId === admission.seatIds[index] && atom(attempt.sessionId) &&
          atom(attempt.reserveRequestId) && attempt.reserveStatus ===
            (index < availableAdmissionSlots ? 'APPLIED' : 'DENIED')) &&
        new Set(outcome.attempts.map(attempt => attempt.sessionId)).size === outcome.attempts.length &&
        outcome.attempts.slice(0, availableAdmissionSlots).every(attempt =>
          atom(attempt.commitRequestId) && attempt.commitStatus === 'APPLIED' &&
          attempt.stopFact === true && atom(attempt.stopRequestId) &&
          attempt.sameSessionRelease === true && atom(attempt.releaseRequestId)) &&
        outcome.attempts.at(-1)?.commitRequestId == null &&
        outcome.attempts.at(-1)?.commitStatus == null &&
        outcome.attempts.at(-1)?.stopFact !== true,
      'Independent K-SESSION admission boundary did not return exact counts and stop/release cleanup');
      record.modelAdmissionCapacity = { projectCap: facts.projectCap, hostLimit: facts.hostLimit,
        effectiveProjectParallel, projectActive: facts.projectActiveClaims.length,
        instanceLimit: facts.instanceLimit, instanceActive: facts.instanceActiveClaims.length,
        availableAdmissionSlots, outcome }; product.save();
    }
  }

  record.state = 'FLOW_COMPLETE_FINAL_READBACK_REQUIRED'; product.save();
  await product.custody(); product.verifyBytes();
  const reference = await hostRoot.normalCloseV06ReadbackRestart('final');
  check(reference && path.basename(reference.file) === reference.file && /^[a-f0-9]{64}$/.test(reference.sha256),
    'Original normal-close immutable V06 readback reference is missing');
  const file = path.resolve(config.evidenceDirectory, reference.file);
  check(path.dirname(file) === path.resolve(config.evidenceDirectory) && sha256(file) === reference.sha256,
    'Original final readback bytes differ from the host reference');
  const proof = JSON.parse(fs.readFileSync(file, 'utf8').replace(/^\uFEFF/, ''));
  check(proof.schema === 'gogoke.37.private-m2-v06-readback.v1' && proof.phase === 'final' &&
    proof.caseId === journal.caseId && proof.sourceCommit === config.sourceCommit &&
    proof.acceptance === false && proof.measurementPreservedDatabaseBytes === true &&
    proof.normalClose?.exitCode === 0 && proof.normalClose.forceKill === false &&
    JSON.stringify(proof.rootIdentity) === JSON.stringify(baseline.rootIdentity) &&
    JSON.stringify(proof.candidateIdentity) === JSON.stringify(baseline.candidateIdentity) &&
    JSON.stringify(proof.candidateInstalledSha256) === JSON.stringify(baseline.candidateInstalledSha256),
  'Original final readback is not bound to the normally closed installed V06 candidate');
  record.readbacks.push({ phase: 'final', ...reference });
  record.finalProof = { sourceCommit: proof.sourceCommit, candidateIdentity: proof.candidateIdentity,
    rootIdentity: proof.rootIdentity, stopRelease: proof.stopRelease ?? null,
    acceptance: false };
  record.state = 'READBACK_COMPLETE_REVIEW_REQUIRED'; product.save();
  return { state: record.state, acceptance: false, notRun: record.notRun,
    completedAxes: ['SHORT_TO_LONG', 'BUSY_CHANGE_AND_STOP_RECLAIM',
      ...(record.seatGrantCapacity ? ['SEAT_GRANT_SCOPE_CAPACITY'] : []),
      ...(record.modelAdmissionCapacity ? ['MODEL_ADMISSION_CAPACITY'] : [])] };

  function readProof(reference, phase) {
    check(reference && path.basename(reference.file) === reference.file && /^[a-f0-9]{64}$/.test(reference.sha256),
      `Original ${phase} readback reference is missing or malformed`);
    const proofFile = path.resolve(config.evidenceDirectory, reference.file);
    check(path.dirname(proofFile) === path.resolve(config.evidenceDirectory) && sha256(proofFile) === reference.sha256,
      `Original ${phase} readback bytes differ from its host reference`);
    const value = JSON.parse(fs.readFileSync(proofFile, 'utf8').replace(/^\uFEFF/, ''));
    check(value.schema === 'gogoke.37.private-m2-v06-readback.v1' && value.phase === phase &&
      value.caseId === journal.caseId && value.sourceCommit === config.sourceCommit &&
      value.acceptance === false && value.measurementPreservedDatabaseBytes === true &&
      value.normalClose?.exitCode === 0 && value.normalClose.forceKill === false,
    `Original ${phase} readback is not this normally closed private V06 candidate`);
    return value;
  }
}
