// Shared per-case ownership and finally flow for temporary SESSION HOME rows.
// This module owns request provenance only; native remains the sole HOME writer.
import fs from 'node:fs';
import path from 'node:path';
import { createHash } from 'node:crypto';

const digest = value => createHash('sha256').update(value, 'utf8').digest('hex');
const nonempty = value => typeof value === 'string' && value.length > 0;
const createHomeFor = request => request?.family === 'K-SESSION' &&
  ['admission-reserve', 'resume'].includes(request.operation);

function persist(manifestPath, manifest) {
  const temporary = `${manifestPath}.tmp`;
  fs.writeFileSync(temporary, `${JSON.stringify(manifest, null, 2)}\n`, { flag: 'w' });
  fs.renameSync(temporary, manifestPath);
}

function stopProof(record) {
  return ['APPLIED', 'REPLAYED'].includes(record?.receipt?.status) &&
    typeof record.receipt.result?.stopFact === 'string' &&
    /^sha256:[a-f0-9]{64}$/.test(record.receipt.result.stopFact)
    ? record.receipt.result.stopFact : null;
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
  const manifest = {
    schema: 'gogoke.37.temporary-home-attempts.v1',
    caseId: journal.caseId,
    domainId,
    state: 'RUNNING',
    producers: [],
    attempts: [],
    notAcceptance: true,
  };
  persist(manifestPath, manifest);

  const save = () => persist(manifestPath, manifest);
  const producerFor = request => manifest.producers.find(row =>
    row.domainId === request.domainId && row.sessionId === request.targetId);
  const attemptFor = requestId => manifest.attempts.find(row => row.originalCreateRequestId === requestId);
  const hEntries = (sessionId, operation, generation) => journal.operations.filter(row =>
    row.request?.family === 'K-SESSION' && row.request.targetId === sessionId &&
    row.request.operation === operation &&
    (generation === undefined || String(row.request.payload?.generation ?? '') === String(generation)));

  const lifecycle = {
    manifestPath,
    manifest,
    registerProducer(session, instanceId) {
      if (!session || !nonempty(session.id) || !nonempty(instanceId) ||
          !nonempty(session.seatId) || !Array.isArray(journal.sessions) ||
          !journal.sessions.includes(session)) {
        throw Error('Temporary-HOME producer requires the exact journaled session and configured instance');
      }
      if (manifest.producers.some(row => row.sessionId === session.id)) {
        throw Error('Temporary-HOME producer anchor already exists');
      }
      const producer = { domainId, sessionId: session.id, instanceId, seatId: session.seatId,
        source: 'CASE_CREATED_SESSION_BEFORE_NATIVE_CREATE_DISPATCH' };
      manifest.producers.push(producer);
      save();
      return producer;
    },
    beforeDispatch(record) {
      const request = record.request;
      if (createHomeFor(request)) {
        const producer = producerFor(request);
        if (!producer || request.domainId !== domainId || request.targetId !== producer.sessionId ||
            !nonempty(request.requestId) || typeof record.rawFrame !== 'string' ||
            record.rawFrame !== JSON.stringify(request) ||
            !nonempty(request.payload?.generation)) {
          throw Error('Unanchored temporary-HOME producer; native creation blocked before dispatch');
        }
        if (attemptFor(request.requestId)) throw Error('Duplicate original temporary-HOME create requestId');
        const attempt = {
          domainId, sessionId: producer.sessionId, instanceId: producer.instanceId,
          operation: request.operation, generation: String(request.payload.generation),
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
        record.temporaryHomeAttemptRequestId = request.requestId;
        save();
      } else if (request?.family === 'K-INSTANCE' && request.operation === 'temporary-home-close') {
        const sessionId = request.payload?.sessionId;
        const originalCreateRequestId = request.payload?.originalCreateRequestId;
        const attempt = attemptFor(originalCreateRequestId);
        if (!attempt || attempt.sessionId !== sessionId || attempt.instanceId !== request.targetId ||
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
    afterDispatchError(record, error) {
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
    afterReconciliation(originalRecord, observation) {
      const attempt = attemptFor(originalRecord.request?.requestId);
      if (!attempt) return;
      attempt.receipt = observation.receipt;
      attempt.dispatchState = 'ORIGINAL_REQUEST_RECONCILED';
      if (originalRecord.request.operation === 'resume' && nonempty(observation.receipt?.result?.newGeneration)) {
        attempt.generation = String(observation.receipt.result.newGeneration);
      }
      save();
    },
    async beforeCaptionClose() {
      const errors = [];
      for (const attempt of manifest.attempts) {
        if (attempt.teardownState.startsWith('CLOSED')) continue;
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
            row.request.targetId === attempt.sessionId && row.request.operation === 'resume' &&
            String(row.receipt?.result?.newGeneration ?? '') === String(attempt.generation));
          const currentGeneration = String(session.generation ?? '');
          const startProven = (opened.length === 1 &&
            ['APPLIED', 'REPLAYED'].includes(opened[0].receipt?.status)) ||
            (resumed.length === 1 && ['APPLIED', 'REPLAYED'].includes(resumed[0].receipt?.status));
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
          attempt.teardownState = 'CASE_TEARDOWN_INCOMPLETE_CLOSE_FAILED_OR_UNKNOWN';
          attempt.teardownError = String(error?.stack ?? error);
          errors.push(`${attempt.originalCreateRequestId}: close failed or unknown`);
        }
        save();
      }
      manifest.state = errors.length ? 'CASE_TEARDOWN_INCOMPLETE' :
        'CLOSE_RECEIPTS_COMPLETE_SHARED_CLOSED_READBACK_REQUIRED';
      manifest.errors = errors;
      save();
      journal.temporaryHomeLifecycle = { manifest: path.basename(manifestPath),
        state: manifest.state, attempts: manifest.attempts.length,
        closedReceipts: manifest.attempts.filter(row => row.teardownState ===
          'CLOSE_RECEIPT_CLOSED_SHARED_READBACK_REQUIRED').length,
        independentSharedReadback: 'NOT_VERIFIED' };
      product.save();
      return { complete: errors.length === 0, errors };
    },
    applySharedClosedReadback(facts) {
      if (manifest.attempts.length === 0) {
        manifest.sharedReadback = 'NOT_REQUIRED_NO_CREATE_DISPATCHED';
        manifest.state = 'NO_HOME_CREATE_DISPATCHED';
        save();
        journal.temporaryHomeLifecycle = { ...journal.temporaryHomeLifecycle,
          manifest: path.basename(manifestPath), state: manifest.state,
          independentSharedReadback: manifest.sharedReadback };
        product.save();
        return true;
      }
      const homes = facts?.temporaryHomes;
      const valid = Array.isArray(homes) && manifest.attempts.every(attempt => {
        const receipt = attempt.closeReceipt?.result;
        const rows = homes.filter(row => row.originalCreateRequestId === attempt.originalCreateRequestId &&
          row.domainId === attempt.domainId && row.sessionId === attempt.sessionId &&
          row.instanceId === attempt.instanceId && row.state === 'CLOSED' && String(row.revision) === '2' &&
          row.closeRequestId === attempt.closeRequestId && row.nativeReceiptId === receipt?.nativeReceiptId &&
          row.proofKind === receipt?.proofKind && row.sourceFingerprint === receipt?.sourceFingerprint);
        return rows.length === 1 && closeReceipt({ receipt: attempt.closeReceipt },
          attempt.closeRequestId, attempt.instanceId);
      }) && homes.length === manifest.attempts.length &&
        facts.homeInventoryUnchangedExceptOwnClosed === true;
      manifest.sharedReadback = valid ? 'EXACT_OWN_HOMES_CLOSED' : 'NOT_QUALIFIED';
      manifest.state = valid ? 'CASE_TEARDOWN_COMPLETE_REVIEW_REQUIRED' : 'CASE_TEARDOWN_INCOMPLETE';
      save();
      journal.temporaryHomeLifecycle = { ...journal.temporaryHomeLifecycle,
        state: manifest.state, independentSharedReadback: manifest.sharedReadback };
      product.save();
      return valid;
    },
    assertComplete() {
      if (manifest.state !== 'CASE_TEARDOWN_COMPLETE_REVIEW_REQUIRED') {
        throw Error('CASE_TEARDOWN_INCOMPLETE: own temporary HOME receipts or shared closed readback are missing');
      }
    },
  };
  product.temporaryHomeLifecycle = lifecycle;
  return lifecycle;
}

export const isTemporaryHomeCreateRequest = createHomeFor;
