// Shared per-case ownership and finally flow for temporary SESSION HOME rows.
// This module owns request provenance only; native remains the sole HOME writer.
import fs from 'node:fs';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { createStartupHomeLifecycle } from './installed-startup-home-lifecycle.mjs';

const digest = value => createHash('sha256').update(value, 'utf8').digest('hex');
const fileDigest = file => createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const nonempty = value => typeof value === 'string' && value.length > 0;
const createHomeFor = request => request?.family === 'K-SESSION' &&
  ['admission-reserve', 'resume'].includes(request.operation);

export function assertSupportedTemporaryHomeOperation(request) {
  if (request?.family === 'K-SESSION' && ['compact', 'renew-session'].includes(request.operation)) {
    throw Error('Temporary-HOME operation blocked: compact/renew-session creation ownership and closure are not instrumented');
  }
}

function persist(manifestPath, manifest) {
  const temporary = `${manifestPath}.tmp`;
  fs.writeFileSync(temporary, `${JSON.stringify(manifest, null, 2)}\n`, { flag: 'w' });
  fs.renameSync(temporary, manifestPath);
}

function stopProof(record) {
  const request = record?.request;
  const receipt = record?.receipt;
  const exactReply = request && record.rawFrame === JSON.stringify(request) &&
    receipt?.requestId === request.requestId && receipt?.targetId === request.targetId &&
    receipt?.family === request.family && receipt?.operation === request.operation;
  return exactReply && request.family === 'K-SESSION' && request.operation === 'stop' &&
    ['APPLIED', 'REPLAYED'].includes(receipt.status) &&
    typeof receipt.result?.stopFact === 'string' && /^sha256:[a-f0-9]{64}$/.test(receipt.result.stopFact)
    ? receipt.result.stopFact : null;
}

function matchesRequestReply(record, family, operation, allowed) {
  const request = record?.request;
  const receipt = record?.receipt;
  return request?.family === family && request?.operation === operation &&
    record.rawFrame === JSON.stringify(request) && receipt?.requestId === request.requestId &&
    receipt?.targetId === request.targetId && receipt?.family === family &&
    receipt?.operation === operation && allowed.includes(receipt.status);
}

function closeReceipt(record, requestId, targetId) {
  const receipt = record?.receipt;
  const result = receipt?.result;
  return receipt?.requestId === requestId && receipt?.targetId === targetId &&
    receipt?.family === 'K-INSTANCE' && receipt?.operation === 'temporary-home-close' &&
    ['APPLIED', 'REPLAYED'].includes(receipt.status) &&
    String(receipt.previousRevision) === '1' && String(receipt.revision) === '2' &&
    nonempty(result?.homeId) && nonempty(result?.nativeReceiptId) &&
    nonempty(result?.proofKind) && nonempty(result?.sourceFingerprint) &&
    result?.state === 'CLOSED';
}

export function createTemporaryHomeLifecycle(product, manifestPath) {
  if (!path.isAbsolute(manifestPath) || fs.existsSync(manifestPath) || fs.existsSync(`${manifestPath}.tmp`)) {
    throw Error('Fresh private temporary-HOME attempt manifest path required');
  }
  const journal = product.journal;
  const domainId = product.config.domainId;
  const config = product.config;
  const readbackConfig = config.temporaryHomeReadback;
  let activeCaseId = null;
  const manifest = {
    schema: 'gogoke.37.temporary-home-attempts.v1',
    caseId: journal.caseId,
    domainId,
    state: 'RUNNING',
    cases: [],
    producers: [],
    attempts: [],
    notAcceptance: true,
  };
  persist(manifestPath, manifest);

  const save = () => persist(manifestPath, manifest);
  let baselineReference = null;
  const producerFor = request => manifest.producers.find(row =>
    row.domainId === request.domainId && row.sessionId === request.targetId);
  const attemptFor = requestId => manifest.attempts.find(row => row.originalCreateRequestId === requestId);
  const requireReadbackPins = () => {
    if (!path.isAbsolute(config.python ?? '') || !path.isAbsolute(config.stateRoot ?? '') ||
        !path.isAbsolute(config.evidenceDirectory ?? '') || !readbackConfig ||
        !path.isAbsolute(readbackConfig.path ?? '') || !/^[a-f0-9]{64}$/.test(readbackConfig.sha256 ?? '') ||
        !path.isAbsolute(readbackConfig.corePath ?? '') || !/^[a-f0-9]{64}$/.test(readbackConfig.coreSha256 ?? '') ||
        fileDigest(readbackConfig.path) !== readbackConfig.sha256 ||
        fileDigest(readbackConfig.corePath) !== readbackConfig.coreSha256) {
      throw Error('Pinned temporary-HOME closed-database reader and core instrument are required');
    }
    return { path: readbackConfig.path, sha256: readbackConfig.sha256,
      corePath: readbackConfig.corePath, coreSha256: readbackConfig.coreSha256 };
  };
  const runReadback = async (mode, baseline = null, phase = mode) => {
    const pins = requireReadbackPins();
    if (!/^(baseline|verify|startup-\d+)$/.test(phase)) throw Error('Invalid shared readback phase');
    const inputPath = path.join(config.evidenceDirectory, `temporary-home-readback-${phase}-input.json`);
    const outputPath = path.join(config.evidenceDirectory, `temporary-home-readback-${phase}.json`);
    const snapshotPath = path.join(config.evidenceDirectory, `temporary-home-readback-${phase}-manifest.json`);
    if (fs.existsSync(inputPath) || fs.existsSync(outputPath) || fs.existsSync(snapshotPath)) {
      throw Error(`Temporary-HOME ${phase} readback evidence already exists; no overwrite`);
    }
    // Keep the bytes used by this read even when its failure is subsequently
    // recorded in the live manifest. A pin to that mutable file is not custody.
    fs.writeFileSync(snapshotPath, fs.readFileSync(manifestPath), { flag: 'wx' });
    const input = {
      schema: 'gogoke.37.temporary-home-readback-input.v1', mode, phase,
      stateRoot: config.stateRoot,
      readerSha256: pins.sha256,
      core: { path: pins.corePath, sha256: pins.coreSha256 },
      manifest: { path: snapshotPath, sha256: fileDigest(snapshotPath) },
      manifestOrigin: manifestPath,
      authorizedRepairs: mode === 'baseline' ? (readbackConfig.authorizedRepairs ?? []) : [],
      baseline,
    };
    fs.writeFileSync(inputPath, `${JSON.stringify(input, null, 2)}\n`, { flag: 'wx' });
    await new Promise((resolve, reject) => {
      const child = spawn(config.python, [pins.path, '--input', inputPath, '--output', outputPath], {
        windowsHide: true, stdio: ['ignore', 'ignore', 'pipe'],
      });
      let stderr = '';
      child.stderr.on('data', bytes => { stderr = (stderr + bytes).slice(-8192); });
      child.once('error', reject);
      child.once('exit', code => code === 0 ? resolve() :
        reject(Error(`Temporary-HOME ${phase} shared readback exit=${code}: ${stderr}`)));
    });
    if (fileDigest(pins.path) !== pins.sha256 || fileDigest(pins.corePath) !== pins.coreSha256 ||
        !fs.existsSync(outputPath)) {
      throw Error(`Temporary-HOME ${phase} reader/core identity changed or output is absent`);
    }
    const report = JSON.parse(fs.readFileSync(outputPath, 'utf8').replace(/^\uFEFF/, ''));
    const expectedState = mode === 'baseline' ? 'BASELINE_MEASURED' :
      mode === 'startup' ? 'STARTUP_SOURCE_MEASURED' :
      (manifest.attempts.length || startup.hasCreatedHomes() ? 'EXACT_OWN_HOMES_CLOSED' : 'NO_HOME_CREATE_DISPATCHED');
    if (report.schema !== 'gogoke.37.temporary-home-readback.v1' || report.mode !== mode ||
        report.acceptance !== false || report.state !== expectedState) {
      throw Error(`Temporary-HOME ${phase} shared readback was not qualified: ${report.state ?? 'missing state'}`);
    }
    const reference = { file: path.basename(outputPath), sha256: fileDigest(outputPath),
      input: path.basename(inputPath), inputSha256: fileDigest(inputPath),
      readerSha256: pins.sha256, coreSha256: pins.coreSha256 };
    journal.temporaryHomeReadback ??= {};
    journal.temporaryHomeReadback[phase] = reference;
    product.save();
    return { report, reference };
  };
  const hEntries = (sessionId, operation, generation) => journal.operations.filter(row =>
    row.request?.family === 'K-SESSION' && row.request.targetId === sessionId &&
    row.request.domainId === domainId &&
    row.request.operation === operation &&
    (generation === undefined || String(row.request.payload?.generation ?? '') === String(generation)));

  const startup = createStartupHomeLifecycle(product, manifest, save, runReadback, () => baselineReference);
  save();
  const lifecycle = {
    manifestPath,
    manifest,
    async captureBaseline() {
      if (manifest.attempts.length !== 0) throw Error('Temporary-HOME baseline must precede every create attempt');
      const { report, reference } = await runReadback('baseline', null);
      baselineReference = { path: path.join(config.evidenceDirectory, reference.file),
        sha256: reference.sha256 };
      startup.initializeRepairs(report);
      return report;
    },
    beforeProductLaunch: () => startup.beforeLaunch(),
    afterProductLaunch: () => startup.afterLaunch(),
    beginCase(caseId) {
      if (!nonempty(caseId) || activeCaseId !== null || manifest.cases.some(row => row.caseId === caseId)) {
        throw Error('Unique temporary-HOME case anchor required');
      }
      manifest.cases.push({ caseId, state: 'RUNNING', attempts: [] });
      activeCaseId = caseId;
      save();
    },
    registerProducer(session, instanceId, caseId = journal.caseId) {
      if (!session || !nonempty(session.id) || !nonempty(instanceId) ||
          !nonempty(session.seatId) || !nonempty(caseId) || caseId !== activeCaseId ||
          !Array.isArray(journal.sessions) ||
          !journal.sessions.includes(session)) {
        throw Error('Temporary-HOME producer requires the exact journaled session and configured instance');
      }
      if (manifest.producers.some(row => row.sessionId === session.id)) {
        throw Error('Temporary-HOME producer anchor already exists');
      }
      const producer = { caseId, domainId, sessionId: session.id, instanceId, seatId: session.seatId,
        source: 'CASE_CREATED_SESSION_BEFORE_NATIVE_CREATE_DISPATCH' };
      if (!manifest.cases.some(row => row.caseId === caseId)) {
        manifest.cases.push({ caseId, state: 'RUNNING', attempts: [] });
      }
      manifest.producers.push(producer);
      save();
      return producer;
    },
    beforeDispatch(record) {
      const request = record.request;
      assertSupportedTemporaryHomeOperation(request);
      if (createHomeFor(request)) {
        const producer = producerFor(request);
        if (!producer || producer.caseId !== activeCaseId || request.domainId !== domainId ||
            request.targetId !== producer.sessionId ||
            !nonempty(request.requestId) || typeof record.rawFrame !== 'string' ||
            record.rawFrame !== JSON.stringify(request) ||
            !nonempty(request.payload?.generation)) {
          throw Error('Unanchored temporary-HOME producer; native creation blocked before dispatch');
        }
        if (attemptFor(request.requestId)) throw Error('Duplicate original temporary-HOME create requestId');
        const attempt = {
          caseId: producer.caseId, domainId, sessionId: producer.sessionId, instanceId: producer.instanceId,
          operation: request.operation,
          producerGeneration: String(request.payload.generation),
          generation: request.operation === 'resume' ? null : String(request.payload.generation),
          originalCreateRequestId: request.requestId,
          originalCreateRawFrame: record.rawFrame,
          originalCreateRawSha256: digest(record.rawFrame),
          dispatchState: 'PERSISTED_BEFORE_DISPATCH',
          receipt: null,
          closeDispatchState: 'NOT_DISPATCHED',
          closeRequestId: null,
          closeRawFrame: null,
          closeReceipt: null,
          teardownState: 'PENDING',
        };
        manifest.attempts.push(attempt);
        manifest.cases.find(row => row.caseId === producer.caseId).attempts.push(request.requestId);
        record.temporaryHomeAttemptRequestId = request.requestId;
        save();
      } else if (request?.family === 'K-INSTANCE' && request.operation === 'temporary-home-close') {
        if (startup.beforeDispatch(record)) return;
        const sessionId = request.payload?.sessionId;
        const originalCreateRequestId = request.payload?.originalCreateRequestId;
        const attempt = attemptFor(originalCreateRequestId);
        if (!attempt || attempt.caseId !== activeCaseId || attempt.sessionId !== sessionId ||
            attempt.instanceId !== request.targetId ||
            attempt.domainId !== request.domainId || request.expectedRevision !== '1' ||
            Object.keys(request.payload ?? {}).sort().join('|') !== 'originalCreateRequestId|sessionId') {
          throw Error('Temporary-HOME close must select this case original create request and exact F1 revision');
        }
        if (attempt.closeDispatchState !== 'NOT_DISPATCHED') {
          throw Error('Original temporary-HOME close was already dispatched; no new requestId is allowed');
        }
        attempt.closeDispatchState = 'PERSISTED_BEFORE_DISPATCH';
        attempt.closeRequestId = request.requestId;
        attempt.closeRawFrame = record.rawFrame;
        attempt.closeRawSha256 = digest(record.rawFrame);
        record.temporaryHomeCloseRequestId = request.requestId;
        save();
      }
    },
    afterReceipt(record) {
      if (startup.afterReceipt(record)) return;
      const request = record.request;
      const attempt = request?.family === 'K-INSTANCE' && request.operation === 'temporary-home-close'
        ? attemptFor(request.payload?.originalCreateRequestId) : attemptFor(request?.requestId);
      if (!attempt) return;
      if (request.operation === 'temporary-home-close') {
        attempt.closeReceipt = record.receipt;
        attempt.closeDispatchState = 'RECEIPT_RECORDED';
        attempt.teardownState = closeReceipt(record, request.requestId, request.targetId)
          ? 'CLOSE_RECEIPT_CLOSED_SHARED_READBACK_REQUIRED' : 'CLOSE_REFUSED_OR_UNQUALIFIED';
      } else {
        attempt.receipt = record.receipt;
        attempt.dispatchState = 'RECEIPT_RECORDED';
        if (request.operation === 'resume' && nonempty(record.receipt?.result?.newGeneration)) {
          attempt.generation = String(record.receipt.result.newGeneration);
        }
      }
      save();
    },
    afterRejectedReceipt(record) {
      if (startup.afterReceipt(record)) return;
      const request = record.request;
      const attempt = request?.family === 'K-INSTANCE' && request.operation === 'temporary-home-close'
        ? attemptFor(request.payload?.originalCreateRequestId) : attemptFor(request?.requestId);
      if (!attempt) return;
      if (request.operation === 'temporary-home-close') {
        attempt.closeReceipt = record.receipt;
        attempt.closeDispatchState = 'RECEIPT_RECORDED';
        attempt.teardownState = `CASE_TEARDOWN_INCOMPLETE_CLOSE_${record.receipt?.status ?? 'REJECTED'}`;
      } else {
        attempt.receipt = record.receipt;
        attempt.dispatchState = 'RECEIPT_RECORDED';
      }
      save();
    },
    afterDispatchError(record, error) {
      if (startup.afterError(record, error)) return;
      if (record.request?.family === 'K-INSTANCE' && record.request.operation === 'temporary-home-close') {
        const closeAttempt = attemptFor(record.request.payload?.originalCreateRequestId);
        if (closeAttempt) {
          closeAttempt.closeDispatchState = 'DISPATCH_RESULT_UNKNOWN';
          closeAttempt.closeDispatchError = String(error?.stack ?? error);
          closeAttempt.teardownState = 'CASE_TEARDOWN_INCOMPLETE_CLOSE_FAILED_OR_UNKNOWN';
          save();
        }
        return;
      }
      const attempt = attemptFor(record.request?.requestId);
      if (!attempt) return;
      attempt.dispatchState = 'DISPATCH_RESULT_UNKNOWN';
      attempt.dispatchError = String(error?.stack ?? error);
      save();
    },
    beforeReconciliation(originalRecord) {
      const request = originalRecord?.request;
      assertSupportedTemporaryHomeOperation(request);
      const attempt = attemptFor(request?.requestId);
      const producer = request && producerFor(request);
      if (!attempt || !nonempty(activeCaseId) || request.operation !== 'resume' ||
          attempt.caseId !== activeCaseId || producer?.caseId !== activeCaseId ||
          producer.instanceId !== attempt.instanceId || request.domainId !== attempt.domainId ||
          request.targetId !== attempt.sessionId ||
          !journal.operations.includes(originalRecord) ||
          originalRecord.rawFrame !== JSON.stringify(request) ||
          originalRecord.rawFrame !== attempt.originalCreateRawFrame ||
          digest(originalRecord.rawFrame) !== attempt.originalCreateRawSha256 ||
          !matchesRequestReply(originalRecord, 'K-SESSION', 'resume', ['UNKNOWN']) ||
          attempt.receipt?.status !== 'UNKNOWN') {
        throw Error('Temporary-HOME reconciliation requires this active case exact original UNKNOWN resume attempt');
      }
    },
    afterReconciliation(originalRecord, observation) {
      const attempt = attemptFor(originalRecord.request?.requestId);
      if (!attempt) throw Error('Temporary-HOME reconciliation lost its original creation attempt');
      attempt.receipt = observation.receipt;
      attempt.dispatchState = 'ORIGINAL_REQUEST_RECONCILED';
      if (originalRecord.request.operation === 'resume' && nonempty(observation.receipt?.result?.newGeneration)) {
        attempt.generation = String(observation.receipt.result.newGeneration);
      }
      save();
    },
    async settleAttempts(attempts) {
      const errors = [];
      const previousCaseId = activeCaseId;
      for (const attempt of attempts) {
        activeCaseId = attempt.caseId;
        if (attempt.teardownState.startsWith('CLOSED')) continue;
        if (!nonempty(attempt.generation)) {
          attempt.teardownState = 'CASE_TEARDOWN_INCOMPLETE_ORIGINAL_RESUME_GENERATION_UNKNOWN';
          errors.push(`${attempt.originalCreateRequestId}: no original native new-generation receipt`);
          save();
          continue;
        }
        const session = journal.sessions.find(row => row.id === attempt.sessionId);
        const producer = manifest.producers.find(row => row.sessionId === attempt.sessionId);
        if (!session || !producer || product.socket?.readyState !== WebSocket.OPEN) {
          attempt.teardownState = 'CASE_TEARDOWN_INCOMPLETE_HOST_OR_PRODUCER_UNAVAILABLE';
          errors.push(`${attempt.originalCreateRequestId}: host/producer unavailable`);
          save();
          continue;
        }

        let stops = hEntries(attempt.sessionId, 'stop', attempt.generation);
        if (stops.length === 0) {
          const opened = hEntries(attempt.sessionId, 'open', attempt.generation);
          const resumed = journal.operations.filter(row => row.request?.family === 'K-SESSION' &&
            row.request.domainId === attempt.domainId && row.request.targetId === attempt.sessionId &&
            row.request.operation === 'resume' &&
            String(row.receipt?.result?.newGeneration ?? '') === String(attempt.generation));
          const currentGeneration = String(session.generation ?? '');
          const startProven = (opened.length === 1 &&
            matchesRequestReply(opened[0], 'K-SESSION', 'open', ['APPLIED', 'REPLAYED'])) ||
            (resumed.length === 1 &&
              matchesRequestReply(resumed[0], 'K-SESSION', 'resume', ['APPLIED', 'REPLAYED']));
          if (!startProven ||
              currentGeneration !== String(attempt.generation) || !nonempty(session.revision)) {
            attempt.teardownState = 'CASE_TEARDOWN_INCOMPLETE_NO_PROVEN_ORIGINAL_H_STOP';
            errors.push(`${attempt.originalCreateRequestId}: no safe original H stop basis`);
            save();
            continue;
          }
          try {
            const stopped = await product.operation('K-SESSION', 'stop', attempt.sessionId,
              { generation: String(attempt.generation), seatId: producer.seatId },
              session.revision, ['APPLIED', 'REPLAYED']);
            session.revision = stopped.revision;
            product.save();
            stops = hEntries(attempt.sessionId, 'stop', attempt.generation);
          } catch (error) {
            attempt.teardownState = 'CASE_TEARDOWN_INCOMPLETE_H_STOP_FAILED_OR_UNKNOWN';
            attempt.teardownError = String(error?.stack ?? error);
            errors.push(`${attempt.originalCreateRequestId}: H stop failed or unknown`);
            save();
            continue;
          }
        }
        if (stops.length !== 1 || !stopProof(stops[0])) {
          attempt.teardownState = 'CASE_TEARDOWN_INCOMPLETE_H_STOP_NOT_PROVEN';
          errors.push(`${attempt.originalCreateRequestId}: existing H stop is not a proven receipt`);
          save();
          continue;
        }
        attempt.hStopSource = {
          requestId: stops[0].request.requestId,
          rawFrame: stops[0].rawFrame,
          rawSha256: digest(stops[0].rawFrame),
          stopFact: stopProof(stops[0]),
        };
        save();
        if (attempt.closeDispatchState !== 'NOT_DISPATCHED') {
          if (!closeReceipt({ receipt: attempt.closeReceipt }, attempt.closeRequestId, attempt.instanceId)) {
            attempt.teardownState = 'CASE_TEARDOWN_INCOMPLETE_CLOSE_ALREADY_DISPATCHED';
            errors.push(`${attempt.originalCreateRequestId}: original close unresolved; no replay`);
          }
          save();
          continue;
        }
        try {
          await product.operation('K-INSTANCE', 'temporary-home-close', attempt.instanceId,
            { sessionId: attempt.sessionId, originalCreateRequestId: attempt.originalCreateRequestId },
            '1', ['APPLIED', 'REPLAYED']);
          const close = journal.operations.at(-1);
          if (!closeReceipt(close, attempt.closeRequestId, attempt.instanceId)) {
            attempt.teardownState = 'CASE_TEARDOWN_INCOMPLETE_CLOSE_RECEIPT_INVALID';
            errors.push(`${attempt.originalCreateRequestId}: close receipt did not prove CLOSED(2)`);
          }
        } catch (error) {
          attempt.teardownState = attempt.closeReceipt?.status
            ? `CASE_TEARDOWN_INCOMPLETE_CLOSE_${attempt.closeReceipt.status}`
            : 'CASE_TEARDOWN_INCOMPLETE_CLOSE_FAILED_OR_UNKNOWN';
          attempt.teardownError = String(error?.stack ?? error);
          errors.push(`${attempt.originalCreateRequestId}: close failed or unknown`);
        }
        save();
      }
      manifest.state = errors.length ? 'CASE_TEARDOWN_INCOMPLETE' :
        'CLOSE_RECEIPTS_COMPLETE_SHARED_CLOSED_READBACK_REQUIRED';
      manifest.errors = errors;
      save();
      activeCaseId = previousCaseId;
      journal.temporaryHomeLifecycle = { manifest: path.basename(manifestPath),
        state: manifest.state, attempts: attempts.length,
        closedReceipts: attempts.filter(row => row.teardownState ===
          'CLOSE_RECEIPT_CLOSED_SHARED_READBACK_REQUIRED').length,
        independentSharedReadback: 'NOT_VERIFIED' };
      product.save();
      return { complete: errors.length === 0, errors };
    },
    async finishCase(caseId) {
      const caseRow = manifest.cases.find(row => row.caseId === caseId);
      if (!caseRow) throw Error('Temporary-HOME case was not begun or anchored');
      const attempts = manifest.attempts.filter(row => row.caseId === caseId);
      if (attempts.length === 0) {
        caseRow.state = 'NO_HOME_CREATE_DISPATCHED';
        if (activeCaseId === caseId) activeCaseId = null;
        save();
        return { complete: true, state: caseRow.state, errors: [] };
      }
      const settled = await lifecycle.settleAttempts(attempts);
      const complete = settled.complete && attempts.every(row =>
        closeReceipt({ receipt: row.closeReceipt }, row.closeRequestId, row.instanceId));
      caseRow.state = complete ? 'CASE_HOME_CLOSE_RECEIPTS_COMPLETE' : 'CASE_TEARDOWN_INCOMPLETE';
      caseRow.errors = settled.errors;
      if (activeCaseId === caseId) activeCaseId = null;
      save();
      return { complete, state: caseRow.state, errors: settled.errors };
    },
    async beforeCaptionClose() {
      const own = await lifecycle.settleAttempts(manifest.attempts);
      const boot = await startup.settle();
      if (!own.complete || !boot.complete) {
        manifest.state = 'CASE_TEARDOWN_INCOMPLETE'; save();
      }
      return { complete: own.complete && boot.complete, errors: [...own.errors, ...boot.errors] };
    },
    applySharedClosedReadback(facts) {
      if (manifest.attempts.length === 0 && !startup.hasTrackedHomes()) {
        const unchanged = Array.isArray(facts?.temporaryHomes) && facts.temporaryHomes.length === 0 &&
          facts.homeInventoryUnchangedExceptOwnClosed === true;
        manifest.sharedReadback = unchanged ? 'NO_CREATE_HOME_INVENTORY_UNCHANGED' : 'NOT_QUALIFIED';
        manifest.state = unchanged ? 'NO_HOME_CREATE_DISPATCHED' : 'CASE_TEARDOWN_INCOMPLETE';
        save();
        journal.temporaryHomeLifecycle = { ...journal.temporaryHomeLifecycle,
          manifest: path.basename(manifestPath), state: manifest.state,
          independentSharedReadback: manifest.sharedReadback };
        product.save();
        return unchanged;
      }
      const homes = facts?.temporaryHomes;
      const valid = Array.isArray(homes) && manifest.attempts.every(attempt => {
        const receipt = attempt.closeReceipt?.result;
        const rows = homes.filter(row => row.originalCreateRequestId === attempt.originalCreateRequestId &&
          row.caseId === attempt.caseId && row.domainId === attempt.domainId && row.sessionId === attempt.sessionId &&
          row.instanceId === attempt.instanceId && row.state === 'CLOSED' && String(row.revision) === '2' &&
          row.closeRequestId === attempt.closeRequestId && row.nativeReceiptId === receipt?.nativeReceiptId &&
          row.homeId === receipt?.homeId && row.proofKind === receipt?.proofKind &&
          row.sourceFingerprint === receipt?.sourceFingerprint);
        return attempt.hStopSource && rows.length === 1 && closeReceipt({ receipt: attempt.closeReceipt },
          attempt.closeRequestId, attempt.instanceId);
      }) && homes.length === manifest.attempts.length &&
        (facts.homeInventoryMatchesDeclaredScope === true ||
          (manifest.authorizedRepairs.length === 0 && facts.homeInventoryUnchangedExceptOwnClosed === true)) &&
        startup.applyFacts(facts);
      manifest.sharedReadback = valid ? 'EXACT_OWN_HOMES_CLOSED' : 'NOT_QUALIFIED';
      manifest.state = valid ? 'CASE_TEARDOWN_COMPLETE_REVIEW_REQUIRED' : 'CASE_TEARDOWN_INCOMPLETE';
      for (const caseRow of manifest.cases) {
        caseRow.state = caseRow.state === 'NO_HOME_CREATE_DISPATCHED'
          ? (valid ? caseRow.state : 'CASE_TEARDOWN_INCOMPLETE')
          : (valid ? 'CASE_TEARDOWN_COMPLETE_REVIEW_REQUIRED' : 'CASE_TEARDOWN_INCOMPLETE');
      }
      save();
      journal.temporaryHomeLifecycle = { ...journal.temporaryHomeLifecycle,
        state: manifest.state, independentSharedReadback: manifest.sharedReadback };
      product.save();
      return valid;
    },
    async verifyClosedReadback() {
      const latestClose = journal.closes?.at(-1);
      const actualClose = latestClose && product.endpoint && product.child &&
        latestClose.pid === product.endpoint.pid && latestClose.forceKill === false &&
        latestClose.exitCode === product.child.exitCode && product.child.exitCode !== null;
      if (!baselineReference || !actualClose || manifest.cases.some(row => row.state === 'RUNNING')) {
        manifest.state = 'CASE_TEARDOWN_INCOMPLETE';
        manifest.sharedReadback = 'NOT_RUN_BASELINE_OR_NORMAL_CAPTION_CLOSE_MISSING';
        save();
        journal.temporaryHomeLifecycle = { manifest: path.basename(manifestPath),
          state: manifest.state, independentSharedReadback: manifest.sharedReadback };
        product.save();
        return false;
      }
      try {
        const { report, reference } = await runReadback('verify', baselineReference);
        const valid = lifecycle.applySharedClosedReadback(report.facts);
        journal.temporaryHomeReadback.verify = reference;
        journal.temporaryHomeLifecycle = { ...journal.temporaryHomeLifecycle,
          state: valid ? manifest.state : 'CASE_TEARDOWN_INCOMPLETE',
          independentSharedReadback: valid ? manifest.sharedReadback : 'NOT_QUALIFIED' };
        if (!valid) manifest.state = 'CASE_TEARDOWN_INCOMPLETE';
        save(); product.save();
        return valid;
      } catch (error) {
        manifest.state = 'CASE_TEARDOWN_INCOMPLETE';
        manifest.sharedReadbackError = String(error?.stack ?? error);
        save();
        journal.temporaryHomeLifecycle = { manifest: path.basename(manifestPath),
          state: manifest.state, independentSharedReadback: 'NOT_QUALIFIED',
          error: manifest.sharedReadbackError };
        product.save();
        throw error;
      }
    },
    assertComplete() {
      if (manifest.state === 'NO_HOME_CREATE_DISPATCHED') return;
      if (manifest.state !== 'CASE_TEARDOWN_COMPLETE_REVIEW_REQUIRED') {
        throw Error('CASE_TEARDOWN_INCOMPLETE: own temporary HOME receipts or shared closed readback are missing');
      }
    },
  };
  product.temporaryHomeLifecycle = lifecycle;
  return lifecycle;
}

export const isTemporaryHomeCreateRequest = createHomeFor;
