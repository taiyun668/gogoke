// Native bootstrap creates a HOME before the first UI request. Track that
// producer from the closed pre-launch source, then use only the real USER pipe.
import path from 'node:path';
import fs from 'node:fs';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
const digest = value => createHash('sha256').update(value, 'utf8').digest('hex');
const loadedPath = fileURLToPath(import.meta.url);
const sourceHash = () => createHash('sha256').update(fs.readFileSync(loadedPath)).digest('hex');
const loadedHash = sourceHash();
export function startupLifecycleSource() {
  if (sourceHash() !== loadedHash) throw Error('Loaded startup lifecycle instrument source changed');
  return { path: loadedPath, sha256: loadedHash };
}
const nonempty = value => typeof value === 'string' && value.length > 0;
const exactReply = row => row?.rawFrame === JSON.stringify(row.request) &&
  row.receipt?.requestId === row.request.requestId &&
  row.receipt?.targetId === row.request.targetId &&
  row.receipt?.family === row.request.family && row.receipt?.operation === row.request.operation;
const closed = row => ['APPLIED', 'REPLAYED'].includes(row?.closeReceipt?.status) &&
  row.closeReceipt.result?.state === 'CLOSED' &&
  row.closeReceipt.previousRevision === '1' && row.closeReceipt.revision === '2';
const check = (ok, reason) => { if (!ok) throw Error(reason); };

export function createStartupHomeLifecycle(product, manifest, save, runReadback, baseline) {
  startupLifecycleSource();
  const journal = product.journal;
  manifest.startupAttempts = [];
  manifest.authorizedRepairs = [];
  let current = null;
  let launches = 0;
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
  const closeRow = async row => {
    if (closed(row)) return;
    check(row.closeDispatchState === 'NOT_DISPATCHED',
      'Original startup/repair close unresolved; no resend');
    const payload = row.sourceKind === 'AUTHORIZED_REPAIR'
      ? { sessionId: row.sessionId, historicalHomeId: row.homeId }
      : { sessionId: row.sessionId, originalCreateRequestId: row.originalCreateRequestId };
    await product.operation('K-INSTANCE', 'temporary-home-close', row.instanceId,
      payload, '1', ['APPLIED', 'REPLAYED', 'UNKNOWN'], row.domainId);
    check(closed(row), 'Startup/repair F receipt did not prove CLOSED(2)');
  };
  return {
    initializeRepairs(report) {
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
      startupLifecycleSource();
      check(baseline(), 'Closed original HOME baseline required before product launch');
      check(!current || closed(current), 'Previous startup HOME is not closed; no successor launch');
      const { report, reference } = await runReadback('startup', null, 'startup-' + launches++);
      const plan = report.startup;
      check(plan && ['NONE', 'RESUME_EXPECTED'].includes(plan.state),
        'Native startup source is unresolved; product launch blocked: ' + (plan?.reason ?? 'missing source'));
      current = null;
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
      }
      journal.startupHomeLaunchSource = reference;
      save(); product.save();
    },
    async afterLaunch() {
      startupLifecycleSource();
      const metadata = await readSecretary();
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
      caseFor(row).state = closed(row) ? 'CASE_HOME_CLOSE_RECEIPTS_COMPLETE' : 'CASE_TEARDOWN_INCOMPLETE';
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
      for (const row of manifest.authorizedRepairs) {
        try { await closeRow(row); }
        catch (error) { errors.push(String(error?.stack ?? error)); }
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
      return matches(manifest.startupAttempts, facts?.startupHomes ?? []) &&
        matches(manifest.authorizedRepairs, facts?.authorizedRepairs ?? []);
    },
    hasTrackedHomes: () => all().length > 0,
    hasCreatedHomes: () => manifest.startupAttempts.length > 0,
  };
}
