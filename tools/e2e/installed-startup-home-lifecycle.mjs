// Native bootstrap creates a HOME before the first UI request. Track that
// producer from the closed pre-launch source, then use only the real USER pipe.
import path from 'node:path';
import fs from 'node:fs';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import { isDeepStrictEqual } from 'node:util';
const digest = value => createHash('sha256').update(value, 'utf8').digest('hex');
const loadedPath = fileURLToPath(import.meta.url);
const sourceHash = () => createHash('sha256').update(fs.readFileSync(loadedPath)).digest('hex');
const loadedHash = sourceHash();
export function startupLifecycleSource() {
  if (sourceHash() !== loadedHash) throw Error('Loaded startup lifecycle instrument source changed');
  return { path: loadedPath, sha256: loadedHash };
}
const nonempty = value => typeof value === 'string' && value.length > 0;
const exactReply = row => row?.request?.schema === 'gogoke.37.operations.v1' &&
  row.receipt?.schema === row.request.schema && row.rawFrame === JSON.stringify(row.request) &&
  row.receipt?.requestId === row.request.requestId &&
  row.receipt?.targetId === row.request.targetId &&
  row.receipt?.family === row.request.family && row.receipt?.operation === row.request.operation;
const closed = row => row?.closeReceipt?.schema === 'gogoke.37.operations.v1' &&
  ['APPLIED', 'REPLAYED'].includes(row.closeReceipt.status) &&
  row.closeReceipt.result?.state === 'CLOSED' &&
  row.closeReceipt.previousRevision === '1' && row.closeReceipt.revision === '2';
const refused = row => {
  if (row.sourceKind !== 'AUTHORIZED_REPAIR' || row.closeDispatchState !== 'RECEIPT_RECORDED') return false;
  try {
    const request = JSON.parse(row.closeRawFrame), receipt = row.closeReceipt;
    return row.closeRawFrame === JSON.stringify(request) && digest(row.closeRawFrame) === row.closeRawSha256 &&
      request.schema === 'gogoke.37.operations.v1' && request.family === 'K-INSTANCE' &&
      request.operation === 'temporary-home-close' && request.requestId === row.closeRequestId &&
      request.targetId === row.instanceId && request.domainId === row.domainId &&
      request.expectedRevision === '1' && Object.keys(request.payload).sort().join('|') === 'historicalHomeId|sessionId' &&
      request.payload.historicalHomeId === row.homeId && request.payload.sessionId === row.sessionId &&
      receipt?.schema === request.schema && receipt.family === request.family && receipt.operation === request.operation &&
      receipt.requestId === request.requestId && receipt.targetId === request.targetId &&
      ['DENIED', 'CONFLICT', 'STALE'].includes(receipt.status) &&
      receipt.previousRevision === '1' && receipt.revision === '1' && nonempty(receipt.result?.reason);
  } catch { return false; }
};
const check = (ok, reason) => { if (!ok) throw Error(reason); };

export function createStartupHomeLifecycle(product, manifest, save, runReadback, baseline, restore = null) {
  startupLifecycleSource();
  const journal = product.journal;
  const restored = restore !== null;
  if (!restored) {
    manifest.startupAttempts = [];
    manifest.noStartupHomes = [];
    manifest.authorizedRepairs = [];
  } else {
    check(Array.isArray(manifest.startupAttempts) && Array.isArray(manifest.noStartupHomes) &&
      Array.isArray(manifest.authorizedRepairs) && restore.startupAttempts === undefined,
    'Restored startup arrays must come from the bound manifest exactly once');
  }
  const restoredRow = (value, rows) => {
    if (value == null) return null;
    const matches = rows.filter(row => isDeepStrictEqual(row, value));
    check(matches.length === 1, 'Restored startup current row differs from the bound manifest');
    return matches[0];
  };
  let current = restoredRow(restore?.current, manifest.startupAttempts);
  let expectedNoCreate = restoredRow(restore?.expectedNoCreate, manifest.noStartupHomes);
  if (restored) check(!manifest.startupAttempts.some(row => !closed(row) && row !== current),
    'Restored startup state omits an unfinished HOME');
  let launches = Number.isSafeInteger(restore?.launches) ? restore.launches : 0;
  const all = () => [...manifest.startupAttempts, ...manifest.authorizedRepairs];
  const caseFor = row => manifest.cases.find(value => value.caseId === row.caseId);
  const select = request => {
    if (request?.family !== 'K-INSTANCE' || request.operation !== 'temporary-home-close') return null;
    return all().find(row => request.domainId === row.domainId &&
      request.targetId === row.instanceId && request.payload?.sessionId === row.sessionId &&
      (row.sourceKind === 'AUTHORIZED_REPAIR'
        ? request.payload.historicalHomeId === row.homeId
        : request.payload.originalCreateRequestId === row.originalCreateRequestId));
  };
  const readSecretary = async () => {
    const frame = { schema: 'gogoke.37.owner-configuration.v1', command: 'secretary-configuration-read' };
    const row = { frame, rawFrame: JSON.stringify(frame), startedAt: new Date().toISOString() };
    journal.startupHomeReads ??= []; journal.startupHomeReads.push(row); product.save();
    try {
      row.rawReply = await product.evaluate(
        "window.__TAURI_INTERNALS__.invoke('gogoke_design37_user_operation',{frame:" +
        JSON.stringify(row.rawFrame) + '})');
      row.reply = JSON.parse(row.rawReply);
      row.finishedAt = new Date().toISOString(); product.save();
      check(row.reply.schema === 'gogoke.37.secretary-configuration.v1',
        'Original USER secretary metadata schema differs');
      return row;
    } catch (error) {
      row.originalError = String(error?.stack ?? error); product.save(); throw error;
    }
  };
  const currentConversation = (row, metadata) => {
    const value = metadata.reply, conversation = value.conversation;
    check(value.state === 'DESIGNATED' && value.seatId === row.seatId &&
      value.incarnation === row.seatIncarnation && value.instanceId === row.instanceId &&
      conversation?.state === 'FOUND' && conversation.sessionId === row.sessionId &&
      String(conversation.generation) === row.generation &&
      /^\d+$/.test(String(conversation.revision)), 'Current startup E/H identity differs');
    return conversation;
  };
  const noStartupCreated = (row, metadata) => {
    const value = metadata?.reply, conversation = value?.conversation;
    const prefix = 'Original Secretary resume result: ';
    check(value?.state === 'DESIGNATED' && value.seatState === 'BUSY' && value.seatId === row.seatId &&
      value.incarnation === row.seatIncarnation && value.instanceId === row.instanceId &&
      conversation?.state === 'FOUND' && conversation.sessionId === row.sessionId &&
      String(conversation.generation) === row.oldGeneration &&
      String(conversation.revision) === row.oldRevision && conversation.claimState === 'STOPPED' &&
      conversation.runtimeAvailable === false && conversation.stoppedFact === true &&
      value.startupReason?.startsWith(prefix), 'Closed predecessor startup identity/refusal differs');
    const receipt = JSON.parse(value.startupReason.slice(prefix.length));
    check(Object.keys(receipt).sort().join('|') ===
      'family|operation|previousRevision|requestId|result|revision|schema|status|targetId' &&
      receipt.schema === 'gogoke.37.operations.v1' && receipt.family === 'K-SESSION' &&
      receipt.operation === 'resume' && receipt.requestId === row.expectedCreateRequestId &&
      receipt.targetId === row.sessionId && receipt.status === 'CONFLICT' &&
      receipt.previousRevision === row.oldRevision && receipt.revision === row.oldRevision &&
      receipt.result && typeof receipt.result === 'object' && !Array.isArray(receipt.result) &&
      Object.keys(receipt.result).length === 0,
    'Closed predecessor lacks its exact original native prewrite CONFLICT');
    return receipt;
  };
  const sameEndpoint = (left, right) => left && right &&
    ['pid', 'port', 'setId', 'generationId', 'readyPath', 'stderrPath', 'url']
      .every(key => left[key] === right[key]);
  const closeRow = async row => {
    if (closed(row) || refused(row)) return;
    check(row.closeDispatchState === 'NOT_DISPATCHED',
      'Original startup/repair close unresolved; no resend');
    const payload = row.sourceKind === 'AUTHORIZED_REPAIR'
      ? { sessionId: row.sessionId, historicalHomeId: row.homeId }
      : { sessionId: row.sessionId, originalCreateRequestId: row.originalCreateRequestId };
    await product.operation('K-INSTANCE', 'temporary-home-close', row.instanceId,
      payload, '1', row.sourceKind === 'AUTHORIZED_REPAIR'
        ? ['APPLIED', 'REPLAYED', 'UNKNOWN', 'DENIED', 'CONFLICT', 'STALE']
        : ['APPLIED', 'REPLAYED', 'UNKNOWN'], row.domainId);
    check(closed(row) || refused(row), 'Startup/repair F receipt did not prove CLOSED(2) or an exact repair refusal');
  };
  return {
    initializeRepairs(report) {
      if (restored) throw Error('Restored startup lifecycle cannot initialize new repairs');
      for (const intent of report.authorizedRepairs ?? []) {
        check(intent.beforeHome && intent.beforeHome.state === 'ACTIVE' &&
          String(intent.beforeHome.revision) === '1', 'Original explicit repair baseline missing');
        const caseId = journal.caseId + '_repair_' + manifest.authorizedRepairs.length;
        manifest.cases.push({ caseId, sourceKind: 'AUTHORIZED_REPAIR', state: 'RUNNING', attempts: [] });
        manifest.authorizedRepairs.push({ ...intent, caseId, sourceKind: 'AUTHORIZED_REPAIR',
          closeDispatchState: 'NOT_DISPATCHED', closeReceipt: null });
      }
      save();
    },
    async beforeLaunch() {
      if (restored) throw Error('Restored startup lifecycle cannot launch a new product');
      startupLifecycleSource();
      check(baseline(), 'Closed original HOME baseline required before product launch');
      check(!current || closed(current), 'Previous startup HOME is not closed; no successor launch');
      for (const row of manifest.noStartupHomes) noStartupCreated(row,row.bootstrap);
      const { report, reference } = await runReadback('startup', null, 'startup-' + launches++);
      const plan = report.startup;
      check(plan && ['NONE', 'RESUME_EXPECTED', 'CLOSED_PREDECESSOR'].includes(plan.state),
        'Native startup source is unresolved; product launch blocked: ' + (plan?.reason ?? 'missing source'));
      current = null;
      expectedNoCreate = null;
      if (plan.state === 'RESUME_EXPECTED') {
        const caseId = journal.caseId + '_startup_' + manifest.startupAttempts.length;
        manifest.cases.push({ caseId, sourceKind: 'NATIVE_STARTUP', state: 'RUNNING', attempts: [] });
        current = { ...plan, caseId, sourceKind: 'NATIVE_STARTUP',
          producerGeneration: plan.oldGeneration, generation: null,
          originalCreateRequestId: plan.expectedCreateRequestId,
          launchSource: { path: path.join(product.config.evidenceDirectory, reference.file),
            sha256: reference.sha256 },
          closeDispatchState: 'NOT_DISPATCHED', closeReceipt: null };
        manifest.startupAttempts.push(current);
      } else if (plan.state === 'CLOSED_PREDECESSOR') {
        const caseId = journal.caseId + '_no_startup_' + manifest.noStartupHomes.length;
        manifest.cases.push({caseId, sourceKind:'NATIVE_STARTUP_NO_CREATE', state:'RUNNING', attempts:[]});
        expectedNoCreate = {...plan, caseId, sourceKind:'NATIVE_STARTUP_NO_CREATE',
          launchSource:{path:path.join(product.config.evidenceDirectory,reference.file),sha256:reference.sha256}};
        manifest.noStartupHomes.push(expectedNoCreate);
      }
      journal.startupHomeLaunchSource = reference;
      save(); product.save();
    },
    async afterLaunch() {
      if (restored) throw Error('Restored startup lifecycle cannot observe a new launch');
      startupLifecycleSource();
      const metadata = await readSecretary();
      if (expectedNoCreate) {
        noStartupCreated(expectedNoCreate, metadata);
        expectedNoCreate.bootstrap = metadata;
        caseFor(expectedNoCreate).state = 'CASE_NO_STARTUP_HOME_PENDING_CLOSED_READBACK';
        save(); product.save();
        return;
      }
      if (!current) {
        check(metadata.reply.state === 'UNSET' && metadata.reply.conversation?.state === 'NONE',
          'Unplanned native startup designation appeared');
        return;
      }
      current.generation = current.expectedGeneration;
      const conversation = currentConversation(current, metadata);
      check(conversation.claimState === 'COMMITTED' && conversation.runtimeAvailable === true &&
        conversation.stoppedFact === null,
        'Expected native startup generation has no live original holder');
      current.bootstrap = metadata;
      current.revision = String(conversation.revision);
      save(); product.save();
    },
    async observeRetainedNoStartup() {
      if (!restored || !expectedNoCreate || manifest.startupAttempts.length !== 0 ||
          manifest.noStartupHomes.length !== 1 || expectedNoCreate.sourceKind !== 'NATIVE_STARTUP_NO_CREATE' ||
          expectedNoCreate.state !== 'CLOSED_PREDECESSOR' ||
          caseFor(expectedNoCreate)?.sourceKind !== 'NATIVE_STARTUP_NO_CREATE' ||
          Object.hasOwn(expectedNoCreate, 'bootstrap') ||
          Object.hasOwn(expectedNoCreate, 'retainedMetadataObservation') ||
          !nonempty(restore.originalManifestOrigin) ||
          !/^[a-f0-9]{64}$/.test(restore.originalManifestSha256 ?? '') ||
          !Array.isArray(journal.launches) || journal.launches.length !== 1 ||
          !Array.isArray(journal.operations) || journal.operations.length !== 0 ||
          (journal.startupHomeReads?.length ?? 0) !== 0 ||
          !String(journal.error ?? '').includes('ERR_MODULE_NOT_FOUND') ||
          !String(journal.error ?? '').includes("@e2e-dev/web") ||
          !product.socket || product.socket.readyState !== WebSocket.OPEN || product.child || product.tester ||
          journal.retainedAttachment?.observationMode !== 'ATTACHED_RETAINED_PRODUCT' ||
          journal.retainedAttachment.ui?.url !== journal.launches[0].url ||
          journal.retainedAttachment.ui?.home !== true || journal.retainedAttachment.ui?.tauri !== true ||
          !sameEndpoint(journal.launches[0], product.endpoint) ||
          !sameEndpoint(journal.launches[0], journal.currentEndpoint) ||
          !sameEndpoint(journal.launches[0], journal.retainedAttachment.endpoint) ||
          journal.launches[0].sourceCommit !== product.config.sourceCommit ||
          journal.launches[0].bootstrap?.setId !== journal.launches[0].setId ||
          journal.launches[0].bootstrap?.generationId !== journal.launches[0].generationId ||
          journal.launches[0].ui?.url !== journal.launches[0].url ||
          journal.launches[0].ui?.home !== true || journal.launches[0].ui?.tauri !== true) {
        throw Error('Retained no-startup metadata read requires the exact failed original launch and absent bootstrap');
      }
      startupLifecycleSource();
      const observation = { source: 'CURRENT_RETAINED_USER_CONFIGURATION_READ',
        originalBootstrapMissing: true,
        originalManifest: { path: restore.originalManifestOrigin, sha256: restore.originalManifestSha256 },
        originalLaunch: structuredClone(journal.launches[0]),
        state: 'PERSISTED_BEFORE_READ', startedAt: new Date().toISOString() };
      expectedNoCreate.retainedMetadataObservation = observation;
      save();
      try {
        const metadata = await readSecretary();
        noStartupCreated(expectedNoCreate, metadata);
        expectedNoCreate.bootstrap = metadata;
        observation.state = 'QUALIFIED_CURRENT_READ';
        observation.finishedAt = new Date().toISOString();
        caseFor(expectedNoCreate).state = 'CASE_NO_STARTUP_HOME_PENDING_CLOSED_READBACK';
        save(); product.save();
        return metadata;
      } catch (error) {
        observation.state = 'UNQUALIFIED_CURRENT_READ';
        observation.error = String(error?.stack ?? error);
        save(); product.save();
        throw error;
      }
    },
    beforeDispatch(record) {
      startupLifecycleSource();
      const row = select(record.request);
      if (!row) return false;
      const keys = row.sourceKind === 'AUTHORIZED_REPAIR'
        ? 'historicalHomeId|sessionId' : 'originalCreateRequestId|sessionId';
      check(record.rawFrame === JSON.stringify(record.request) &&
        record.request.expectedRevision === '1' &&
        Object.keys(record.request.payload).sort().join('|') === keys &&
        row.closeDispatchState === 'NOT_DISPATCHED',
      'Startup/repair close must match its exact anchored intent once');
      row.closeDispatchState = 'PERSISTED_BEFORE_DISPATCH';
      row.closeRequestId = record.request.requestId;
      row.closeRawFrame = record.rawFrame;
      // Hash is added by the shared caller before dispatch, never by a receipt.
      row.closeRawSha256 = digest(record.rawFrame);
      save();
      return true;
    },
    afterReceipt(record) {
      const row = select(record.request);
      if (!row) return false;
      row.closeReceipt = record.receipt;
      row.closeDispatchState = 'RECEIPT_RECORDED';
      caseFor(row).state = closed(row) ? 'CASE_HOME_CLOSE_RECEIPTS_COMPLETE' :
        refused(row) ? 'CASE_HOME_CLOSE_REFUSED' : 'CASE_TEARDOWN_INCOMPLETE';
      save(); return true;
    },
    afterError(record, error) {
      const row = select(record.request);
      if (!row) return false;
      row.closeDispatchState = 'DISPATCH_RESULT_UNKNOWN';
      row.closeError = String(error?.stack ?? error);
      caseFor(row).state = 'CASE_TEARDOWN_INCOMPLETE';
      save(); return true;
    },
    async settle() {
      startupLifecycleSource();
      const errors = [];
      for (const row of manifest.noStartupHomes) {
        try { noStartupCreated(row,row.bootstrap); }
        catch(error) { errors.push(String(error?.stack ?? error)); }
      }
      for (const row of manifest.authorizedRepairs) {
        if (errors.length) break;
        try { await closeRow(row); }
        catch (error) { errors.push(String(error?.stack ?? error)); break; }
      }
      if (current && !closed(current)) {
        try {
          check(current.bootstrap && nonempty(current.generation),
            'Startup observation did not prove a current generation');
          const stops = journal.operations.filter(row => row.request?.family === 'K-SESSION' &&
            row.request.operation === 'stop' && row.request.domainId === current.domainId &&
            row.request.targetId === current.sessionId &&
            String(row.request.payload?.generation) === current.generation);
          if (!stops.length) {
            const metadata = await readSecretary();
            const conversation = currentConversation(current, metadata);
            check(conversation.claimState === 'COMMITTED' && conversation.runtimeAvailable === true &&
              conversation.stoppedFact === null,
              'Startup stop lacks a current original live source');
            await product.operation('K-SESSION', 'stop', current.sessionId,
              { generation: current.generation, seatId: current.seatId },
              String(conversation.revision), ['APPLIED', 'REPLAYED', 'UNKNOWN'], current.domainId);
          }
          const stopped = journal.operations.filter(row => row.request?.family === 'K-SESSION' &&
            row.request.operation === 'stop' && row.request.domainId === current.domainId &&
            row.request.targetId === current.sessionId &&
            String(row.request.payload?.generation) === current.generation);
          check(stopped.length === 1 && exactReply(stopped[0]) &&
            ['APPLIED', 'REPLAYED'].includes(stopped[0].receipt.status) &&
            /^sha256:[a-f0-9]{64}$/.test(stopped[0].receipt.result?.stopFact ?? ''),
          'Original startup stop unresolved; no stop or close resend');
          current.hStopSource = { requestId: stopped[0].request.requestId,
            rawFrame: stopped[0].rawFrame,
            rawSha256: digest(stopped[0].rawFrame),
            stopFact: stopped[0].receipt.result.stopFact };
          save();
          await closeRow(current);
        } catch (error) { errors.push(String(error?.stack ?? error)); }
      }
      journal.startupHomeTeardown = { complete: errors.length === 0, errors,
        startupCount: manifest.startupAttempts.length, repairCount: manifest.authorizedRepairs.length };
      save(); product.save();
      return journal.startupHomeTeardown;
    },
    applyFacts(facts) {
      startupLifecycleSource();
      const matches = (attempts, rows) => Array.isArray(rows) && rows.length === attempts.length &&
        attempts.every(attempt => closed(attempt) && rows.filter(row =>
          row.caseId === attempt.caseId && row.domainId === attempt.domainId &&
          row.sessionId === attempt.sessionId && row.instanceId === attempt.instanceId &&
          row.closeRequestId === attempt.closeRequestId &&
          row.nativeReceiptId === attempt.closeReceipt.result.nativeReceiptId &&
          row.homeId === attempt.closeReceipt.result.homeId && row.state === 'CLOSED' &&
          String(row.revision) === '2' &&
          row.sourceFingerprint === attempt.closeReceipt.result.sourceFingerprint &&
          (attempt.sourceKind === 'AUTHORIZED_REPAIR'
            ? row.homeId === attempt.homeId
            : row.originalCreateRequestId === attempt.originalCreateRequestId)).length === 1);
      const refusals = manifest.authorizedRepairs.filter(refused);
      const refusedFacts = facts?.authorizedRepairRefusals ?? [];
      const noCreates = facts?.noStartupHomes ?? [];
      return matches(manifest.startupAttempts, facts?.startupHomes ?? []) &&
        matches(manifest.authorizedRepairs.filter(closed), facts?.authorizedRepairs ?? []) &&
        Array.isArray(noCreates) && noCreates.length === manifest.noStartupHomes.length &&
        manifest.noStartupHomes.every(attempt => {
          try { noStartupCreated(attempt,attempt.bootstrap); } catch { return false; }
          return noCreates.filter(row => row.caseId === attempt.caseId &&
            row.domainId === attempt.domainId && row.sessionId === attempt.sessionId &&
            row.instanceId === attempt.instanceId && row.homeId === attempt.baselineHomeId &&
            row.originalCreateRequestId === attempt.expectedCreateRequestId &&
            row.state === 'NOT_CREATED' && row.generation === attempt.oldGeneration &&
            row.revision === attempt.oldRevision && row.status === 'CONFLICT').length === 1;
        }) && Array.isArray(refusedFacts) && refusedFacts.length === refusals.length &&
        manifest.authorizedRepairs.every(row => closed(row) || refused(row)) &&
        refusals.every(attempt => refusedFacts.filter(row =>
          row.caseId === attempt.caseId && row.homeId === attempt.homeId &&
          row.domainId === attempt.domainId && row.sessionId === attempt.sessionId &&
          row.instanceId === attempt.instanceId && row.state === 'ACTIVE' && row.revision === '1' &&
          row.closeRequestId === attempt.closeRequestId && row.status === attempt.closeReceipt.status &&
          row.reason === attempt.closeReceipt.result.reason).length === 1);
    },
    hasTrackedHomes: () => all().length + manifest.noStartupHomes.length > 0,
    hasCreatedHomes: () => manifest.startupAttempts.length > 0,
  };
}
