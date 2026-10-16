// Synthetic transport controls over the actual shared helper. No product runs.
import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { createHash } from 'node:crypto';
const sourceRoot = fileURLToPath(new URL('../..', import.meta.url));
const { createTemporaryHomeLifecycle } = await import(pathToFileURL(path.join(sourceRoot,
  'tools/e2e/installed-temporary-home-lifecycle.mjs')));
const { ActualProduct } = await import(pathToFileURL(path.join(sourceRoot, 'tools/e2e/product-cdp.mjs')));
const sha = file => createHash('sha256').update(fs.readFileSync(file)).digest('hex');
console.log(JSON.stringify({ syntheticOnly: true, acceptance: false,
  lifecycleSha256: sha(path.join(sourceRoot, 'tools/e2e/installed-temporary-home-lifecycle.mjs')),
  ingressSha256: sha(path.join(sourceRoot, 'tools/e2e/product-cdp.mjs')) }));
const fixtureRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'gogoke-home-lifecycle-'));

test('failed shared readback retains the exact manifest bytes used by that read', async () => {
  const dir = path.join(fixtureRoot, 'readbackFailure'); fs.mkdirSync(dir);
  const reader = path.join(dir, 'reader.cjs');
  fs.writeFileSync(reader, `const fs = require('node:fs');
    const input = JSON.parse(fs.readFileSync(process.argv[3], 'utf8'));
    const report = {schema:'gogoke.37.temporary-home-readback.v1', mode:input.mode,
      acceptance:false, state:input.mode==='baseline'?'BASELINE_MEASURED':'READBACK_FAILED'};
    fs.writeFileSync(process.argv[5], JSON.stringify(report));
    if(input.mode==='verify'){console.error('original synthetic inventory mismatch');process.exitCode=1;}`);
  const journal = { caseId: 'readbackFailure', operations: [], closes: [] };
  const product = { config: { domainId:'projectA', python:process.execPath, stateRoot:dir,
    evidenceDirectory:dir, temporaryHomeReadback:{path:reader,sha256:sha(reader),corePath:reader,coreSha256:sha(reader)} },
    journal, save() {}, endpoint:{pid:123}, child:{exitCode:0} };
  const manifestPath = path.join(dir, 'live-manifest.json');
  const lifecycle = createTemporaryHomeLifecycle(product, manifestPath);
  await lifecycle.captureBaseline();
  journal.closes.push({pid:123, forceKill:false, exitCode:0});
  await assert.rejects(() => lifecycle.verifyClosedReadback(), /original synthetic inventory mismatch/);
  const input = JSON.parse(fs.readFileSync(path.join(dir,'temporary-home-readback-verify-input.json')));
  assert.notEqual(input.manifest.path, manifestPath);
  assert.equal(sha(input.manifest.path), input.manifest.sha256);
  assert.notEqual(sha(manifestPath), input.manifest.sha256);
  assert.equal(lifecycle.manifest.state, 'CASE_TEARDOWN_INCOMPLETE');
});

test('an incomplete teardown stops before caption observation', async () => {
  const product = { journal:{}, save(){}, temporaryHomeLifecycle:{manifestPath:'synthetic',
    async beforeCaptionClose(){return {complete:false, errors:['original unresolved stop']};}} };
  await assert.rejects(() => ActualProduct.prototype.closeNormally.call(product), /original unresolved stop/);
  assert.equal(product.journal.nativeCloseObservationAttempts, undefined);
  assert.equal(product.journal.temporaryHomeLifecycle.state, 'CASE_TEARDOWN_INCOMPLETE');
});

function fixture(name) {
  const dir = path.join(fixtureRoot, name); fs.mkdirSync(dir);
  const session = { id: 'sessionA', seatId: 'seatA', instanceId: 'instanceA', generation: '1', revision: '4' };
  const journal = { caseId: name, sessions: [session], operations: [] };
  const calls = [];
  let lifecycle;
  const product = { config: { domainId: 'projectA' }, journal, socket: { readyState: WebSocket.OPEN },
    save() { fs.writeFileSync(path.join(dir, 'journal.json'), JSON.stringify(journal)); },
    async operation(family, operation, targetId, payload, expectedRevision) {
      calls.push({ family, operation, targetId, payload, expectedRevision });
      const request = { schema: 'gogoke.37.operations.v1', family, operation, requestId: `close${calls.length}`,
        domainId: 'projectA', targetId, expectedRevision, payload };
      const record = { request, rawFrame: JSON.stringify(request), receipt: null };
      lifecycle.beforeDispatch(record); journal.operations.push(record);
      record.receipt = operation === 'admission-release'
        ? { schema: request.schema, family, operation, requestId: request.requestId, targetId,
          status: 'APPLIED', previousRevision: String(expectedRevision), revision: String(Number(expectedRevision) + 1),
          result: { generation: payload.generation } }
        : { schema: request.schema, family, operation, requestId: request.requestId, targetId,
          status: 'APPLIED', previousRevision: '1', revision: '2', result: { homeId: 'homeA', state: 'CLOSED',
            directoryRef: 'temp-home-control', nativeReceiptId: 'f-home-close-v2-control',
            proofKind: 'ORIGINAL_STOP', sourceFingerprint: 'f'.repeat(64) } };
      lifecycle.afterReceipt(record); return record.receipt;
    } };
  lifecycle = createTemporaryHomeLifecycle(product, path.join(dir, 'own-manifest.json'));
  lifecycle.beginCase(name); lifecycle.registerProducer(session, 'instanceA', name);
  const record = (operation, id, generation, status = 'APPLIED', result = {}) => {
    const request = { schema: 'gogoke.37.operations.v1', family: 'K-SESSION', operation, requestId: id,
      domainId: 'projectA', targetId: session.id, expectedRevision: '3',
      payload: { seatId: 'seatA', generation } };
    return { request, rawFrame: JSON.stringify(request), receipt: { schema: request.schema,
      family: request.family, operation, requestId: id, targetId: session.id,
      previousRevision: '3', revision: '4', status, result } };
  };
  const create = row => { lifecycle.beforeDispatch(row); journal.operations.push(row); lifecycle.afterReceipt(row); };
  return { product, session, journal, lifecycle, calls, record, create };
}

test('UNKNOWN resume never borrows the old-generation stop', async () => {
  const f = fixture('unknownResume');
  f.journal.operations.push(f.record('stop', 'oldStop', '1', 'APPLIED', { stopFact: `sha256:${'a'.repeat(64)}` }));
  f.create(f.record('resume', 'newResume', '1', 'UNKNOWN'));
  const result = await f.lifecycle.finishCase('unknownResume');
  assert.equal(result.complete, false); assert.equal(f.calls.length, 0);
  assert.equal(f.lifecycle.manifest.attempts[0].generation, null);
  assert.equal(f.lifecycle.manifest.attempts[0].hStopSource, undefined);
});

test('UNKNOWN stop is retained with zero stop or close resends', async () => {
  const f = fixture('unknownStop');
  f.create(f.record('admission-reserve', 'createA', '1'));
  f.journal.operations.push(f.record('stop', 'stopUnknown', '1', 'UNKNOWN'));
  assert.equal((await f.lifecycle.finishCase('unknownStop')).complete, false);
  assert.deepEqual(f.calls, []);
});

test('own proven close is dispatched once and another case cannot adopt it', async () => {
  const f = fixture('oneClose');
  f.create(f.record('admission-reserve', 'createA', '1'));
  f.journal.operations.push(f.record('stop', 'ownStop', '1', 'APPLIED', { stopFact: `sha256:${'a'.repeat(64)}` }));
  assert.equal((await f.lifecycle.finishCase('oneClose')).complete, true);
  assert.equal((await f.lifecycle.beforeCaptionClose()).complete, true);
  assert.deepEqual(f.calls.map(row => row.operation), ['admission-release', 'temporary-home-close']);
  assert.deepEqual(f.calls[1].payload, { sessionId: 'sessionA', originalCreateRequestId: 'createA' });
  f.lifecycle.beginCase('differentCase');
  const request = { schema: 'gogoke.37.operations.v1', family: 'K-INSTANCE', operation: 'temporary-home-close',
    requestId: 'otherClose', targetId: 'instanceA', domainId: 'projectA', expectedRevision: '1',
    payload: { sessionId: 'sessionA', originalCreateRequestId: 'createA' } };
  assert.throws(() => f.lifecycle.beforeDispatch({ request, rawFrame: JSON.stringify(request) }), /this case/);
});

test('a recorded original CLOSED receipt remains settled after the host closes', async () => {
  const f = fixture('closedAfterHost');
  f.create(f.record('admission-reserve', 'createA', '1'));
  f.journal.operations.push(f.record('stop', 'ownStop', '1', 'APPLIED', { stopFact: `sha256:${'a'.repeat(64)}` }));
  assert.equal((await f.lifecycle.finishCase('closedAfterHost')).complete, true);
  f.product.socket = null;
  f.lifecycle.manifest.attempts[0].teardownState = 'CASE_TEARDOWN_INCOMPLETE_HOST_OR_PRODUCER_UNAVAILABLE';
  assert.equal((await f.lifecycle.finishCase('closedAfterHost')).complete, true);
  assert.equal(f.journal.temporaryHomeLifecycle.closedReceipts, 1);
  assert.deepEqual(f.calls.map(row => row.operation), ['admission-release', 'temporary-home-close']);
  f.lifecycle.manifest.attempts[0].closeReceipt = { ...f.lifecycle.manifest.attempts[0].closeReceipt, requestId: 'foreign' };
  assert.equal((await f.lifecycle.finishCase('closedAfterHost')).complete, false);
  assert.equal(f.calls.length, 2);
});

test('an intermediate checkpoint keeps original stopped owners until case finish', async () => {
  const f = fixture('stopOnlyCheckpoint');
  f.create(f.record('admission-reserve', 'createA', '1'));
  f.journal.operations.push(f.record('stop', 'ownStop', '1', 'APPLIED', { stopFact: `sha256:${'a'.repeat(64)}` }));
  f.session.revision = '4';
  const before = JSON.stringify(f.lifecycle.manifest);
  assert.equal(f.lifecycle.beforeCheckpointClose('stopOnlyCheckpoint').state, 'STOP_ONLY_CHECKPOINT_READBACK_REQUIRED');
  assert.equal(JSON.stringify(f.lifecycle.manifest), before);
  assert.deepEqual(f.calls, []);
  assert.throws(() => f.lifecycle.beforeCheckpointClose('anotherCase'), /active exclusive/);
  f.journal.operations.at(-1).receipt.status = 'UNKNOWN';
  assert.throws(() => f.lifecycle.beforeCheckpointClose('stopOnlyCheckpoint'), /stop-only/);
  f.journal.operations.at(-1).receipt.status = 'APPLIED';
  assert.equal((await f.lifecycle.finishCase('stopOnlyCheckpoint')).complete, true);
  assert.throws(() => f.lifecycle.beforeCheckpointClose('stopOnlyCheckpoint'), /active exclusive|stop-only/);
  assert.deepEqual(f.calls.map(row => row.operation), ['admission-release', 'temporary-home-close']);
});

test('an existing close with mismatched canonical provenance is incomplete without replay', async () => {
  for (const connected of [false, true]) {
    for (const field of ['family', 'targetId', 'rawFrame', 'expectedRevision', 'payload', 'duplicate', 'digest']) {
      const name = `badClosed-${connected}-${field}`;
      const f = fixture(name);
      f.create(f.record('admission-reserve', 'createA', '1'));
      f.journal.operations.push(f.record('stop', 'ownStop', '1', 'APPLIED', { stopFact: `sha256:${'a'.repeat(64)}` }));
      assert.equal((await f.lifecycle.finishCase(name)).complete, true);
      const close = f.journal.operations.at(-1);
      if (field === 'family') close.request.family = 'K-SESSION';
      if (field === 'targetId') close.request.targetId = 'foreign-instance';
      if (field === 'rawFrame') close.rawFrame = '{}';
      if (field === 'expectedRevision') close.request.expectedRevision = '2';
      if (field === 'payload') close.request.payload.foreign = true;
      if (field === 'duplicate') f.journal.operations.push(structuredClone(close));
      if (field === 'digest') f.lifecycle.manifest.attempts[0].closeRawSha256 = '0'.repeat(64);
      if (!connected) f.product.socket = null;
      assert.equal((await f.lifecycle.finishCase(name)).complete, false, name);
      assert.equal(f.journal.temporaryHomeLifecycle.closedReceipts, 0, name);
      assert.equal(f.calls.length, 2, name);
    }
  }
});

test('no-create does not pass a missing or changed inventory', () => {
  const f = fixture('noCreate');
  assert.equal(f.lifecycle.applySharedClosedReadback(null), false);
  assert.throws(() => f.lifecycle.assertComplete(), /INCOMPLETE/);
  assert.equal(f.lifecycle.applySharedClosedReadback({ temporaryHomes: [], homeInventoryUnchangedExceptOwnClosed: true }), true);
  assert.equal(f.lifecycle.manifest.state, 'NO_HOME_CREATE_DISPATCHED');
});

test('actual USER ingress rejects foreign receipts before promoting the manifest', async () => {
  const f = fixture('foreignReply');
  f.product.evaluate = async () => JSON.stringify({ schema: 'gogoke.37.operations.v1', family: 'K-SESSION',
    operation: 'admission-reserve', requestId: 'foreignRequest', targetId: 'sessionA', status: 'APPLIED' });
  await assert.rejects(() => ActualProduct.prototype.operation.call(f.product, 'K-SESSION',
    'admission-reserve', 'sessionA', { generation: '1', seatId: 'seatA' }, '0'), /inspect durable request/);
  assert.equal(f.lifecycle.manifest.attempts[0].receipt, null);
  assert.equal(f.lifecycle.manifest.attempts[0].dispatchState, 'DISPATCH_RESULT_UNKNOWN');
});

test('unmigrated compact and renew are refused before either USER entry point dispatches', async () => {
  for (const operation of ['compact', 'renew-session']) {
    for (const anchored of [false, true]) {
      const f = fixture(`${operation}-${anchored}`);
      if (!anchored) delete f.product.temporaryHomeLifecycle;
      let nativeCalls = 0;
      f.product.evaluate = async () => { nativeCalls++; throw Error('Native must not be reached'); };
      await assert.rejects(() => ActualProduct.prototype.operation.call(f.product, 'K-SESSION',
        operation, 'sessionA', { generation: '1' }, '3'), /not instrumented/);
      const original = f.record(operation, 'untrackedChange', '1', 'UNKNOWN');
      await assert.rejects(() => ActualProduct.prototype.reconcile.call(f.product, original), /not instrumented/);
      assert.equal(nativeCalls, 0);
      assert.equal(f.journal.operations.length, 0);
      assert.equal(f.lifecycle.manifest.attempts.length, 0);
      assert.throws(() => f.lifecycle.beforeDispatch(original), /not instrumented/);
    }
  }
});

test('resume reconciliation rejects missing, copied, and another case original attempts before dispatch', async () => {
  const f = fixture('ownedReconciliation');
  let nativeCalls = 0;
  f.product.evaluate = async () => { nativeCalls++; throw Error('Native must not be reached'); };
  const original = f.record('resume', 'ownResume', '1', 'UNKNOWN');
  await assert.rejects(() => ActualProduct.prototype.reconcile.call(f.product, original), /exact original UNKNOWN/);
  f.create(original);
  await assert.rejects(() => ActualProduct.prototype.reconcile.call(f.product, structuredClone(original)), /exact original UNKNOWN/);
  await f.lifecycle.finishCase('ownedReconciliation');
  f.lifecycle.beginCase('anotherCase');
  await assert.rejects(() => ActualProduct.prototype.reconcile.call(f.product, original), /exact original UNKNOWN/);
  assert.equal(nativeCalls, 0);
  assert.equal(f.lifecycle.manifest.attempts[0].generation, null);
});

test('owned UNKNOWN resume reconciliation sends only its original raw and records only native newGeneration', async () => {
  const f = fixture('originalReconciliation');
  const original = f.record('resume', 'ownResume', '1', 'UNKNOWN');
  f.create(original);
  const evaluated = [];
  f.product.evaluate = async expression => {
    evaluated.push(expression);
    return JSON.stringify({ ...original.receipt, status: 'APPLIED', result: { newGeneration: '2' } });
  };
  const reply = await ActualProduct.prototype.reconcile.call(f.product, original);
  assert.equal(reply.receipt.result.newGeneration, '2');
  assert.equal(evaluated.length, 1);
  assert.ok(evaluated[0].includes(JSON.stringify(original.rawFrame)));
  assert.equal(f.journal.operations.length, 2);
  assert.equal(f.journal.operations[1].request, original.request);
  assert.equal(original.receipt.status, 'UNKNOWN');
  const attempt = f.lifecycle.manifest.attempts[0];
  assert.equal(attempt.producerGeneration, '1');
  assert.equal(attempt.generation, '2');
  assert.equal(attempt.dispatchState, 'ORIGINAL_REQUEST_RECONCILED');
  await assert.rejects(() => ActualProduct.prototype.reconcile.call(f.product, original), /exact original UNKNOWN/);
  assert.equal(evaluated.length, 1);
});

test('the nonempty shared readback consumer requires the exact per-case fact row', async () => {
  const f = fixture('closedFactCase');
  f.create(f.record('admission-reserve', 'createA', '1'));
  f.journal.operations.push(f.record('stop', 'ownStop', '1', 'APPLIED', { stopFact: `sha256:${'a'.repeat(64)}` }));
  await f.lifecycle.finishCase('closedFactCase');
  const attempt = f.lifecycle.manifest.attempts[0];
  const row = { ...attempt.closeReceipt.result, originalCreateRequestId: 'createA',
    caseId: 'closedFactCase', domainId: 'projectA', sessionId: 'sessionA', instanceId: 'instanceA',
    revision: '2', closeRequestId: attempt.closeRequestId };
  const facts = { temporaryHomes: [row], homeInventoryUnchangedExceptOwnClosed: true };
  assert.equal(f.lifecycle.applySharedClosedReadback(facts), true);
  f.lifecycle.assertComplete();
  const missing = structuredClone(facts); delete missing.temporaryHomes[0].caseId;
  assert.equal(f.lifecycle.applySharedClosedReadback(missing), false);
  const foreign = structuredClone(facts); foreign.temporaryHomes[0].caseId = 'anotherCase';
  assert.equal(f.lifecycle.applySharedClosedReadback(foreign), false);
});
