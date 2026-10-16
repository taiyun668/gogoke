import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { createHash } from 'node:crypto';
import { ActualProduct } from '../product-cdp.mjs';
import { createTemporaryHomeLifecycle } from '../installed-temporary-home-lifecycle.mjs';
import { qualifyAttachedCausalExit, requireRecordedNormalClose } from '../causal-close/causal-close.mjs';
import { closeFixture } from './causal-close-fixture.mjs';

const sha = file => createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'gogoke-owned-resume-'));
test.after(() => fs.rmSync(tempRoot, { recursive: true, force: true }));

function restoreFixture(name) {
  const dir = path.join(tempRoot, name); fs.mkdirSync(dir);
  const origin = path.join(dir, 'temporary-home-attempts.json');
  const tracking = path.join(dir, 'temporary-home-attempts-continuation.json');
  const baseline = path.join(dir, 'baseline.json');
  const readerPin = path.join(dir, 'reader.py');
  const corePin = path.join(dir, 'core.py');
  fs.writeFileSync(readerPin, '# reader pin\n');
  fs.writeFileSync(corePin, '# core pin\n');
  const session = { id: 'sessionA', seatId: 'seatA', instanceId: 'instanceA', generation: '1', revision: '0' };
  const request = { schema: 'gogoke.37.operations.v1', family: 'K-SESSION', operation: 'admission-reserve',
    requestId: 'reserveA', domainId: 'projectA', targetId: session.id, expectedRevision: '0',
    payload: { generation: '1', seatId: session.seatId } };
  const original = { request, rawFrame: JSON.stringify(request), receipt: {
    schema: request.schema, family: request.family, operation: request.operation,
    requestId: request.requestId, targetId: request.targetId, previousRevision: '0', revision: '0',
    status: 'DENIED', result: { reason: 'CapacityExceeded' },
  } };
  const manifest = {
    schema: 'gogoke.37.temporary-home-attempts.v1', caseId: name, domainId: 'projectA',
    state: 'CASE_TEARDOWN_INCOMPLETE', cases: [{ caseId: name, state: 'CASE_TEARDOWN_INCOMPLETE', attempts: ['reserveA'] }],
    producers: [{ caseId: name, domainId: 'projectA', sessionId: session.id, instanceId: 'instanceA', seatId: session.seatId }],
    attempts: [{ caseId: name, domainId: 'projectA', sessionId: session.id, instanceId: 'instanceA', operation: 'admission-reserve',
      producerGeneration: '1', generation: '1', originalCreateRequestId: 'reserveA', originalCreateRawFrame: original.rawFrame,
      originalCreateRawSha256: createHash('sha256').update(original.rawFrame).digest('hex'), dispatchState: 'RECEIPT_RECORDED',
      receipt: original.receipt, closeDispatchState: 'NOT_DISPATCHED', closeRequestId: null, closeRawFrame: null,
      closeReceipt: null, teardownState: 'CASE_TEARDOWN_INCOMPLETE_NO_PROVEN_ORIGINAL_H_STOP' }],
    startupAttempts: [], noStartupHomes: [], authorizedRepairs: [], notAcceptance: true,
  };
  const journal = { caseId: name, domainId: 'projectA', sessions: [session], operations: [original], closes: [] };
  fs.writeFileSync(origin, JSON.stringify(manifest));
  const readbackSourcePins = { path: readerPin, sha256: sha(readerPin), corePath: corePin, coreSha256: sha(corePin) };
  const baselineInput = path.join(dir, 'temporary-home-readback-baseline-input.json');
  const baselineManifest = path.join(dir, 'temporary-home-readback-baseline-manifest.json');
  fs.writeFileSync(baselineInput, JSON.stringify({ stateRoot: dir }));
  fs.writeFileSync(baselineManifest, JSON.stringify({ ...manifest, attempts: [] }));
  fs.writeFileSync(baseline, JSON.stringify({ schema: 'gogoke.37.temporary-home-readback.v1',
    mode: 'baseline', state: 'BASELINE_MEASURED', caseId: name, domainId: 'projectA',
    source: { manifestOrigin: origin, reader: { path: readerPin, sha256: sha(readerPin) },
      core: { path: corePin, sha256: sha(corePin) }, inputSha256: sha(baselineInput),
      manifest: { path: baselineManifest, sha256: sha(baselineManifest) } } }));
  const product = { config: { domainId: 'projectA', evidenceDirectory: dir, stateRoot: dir,
      temporaryHomeReadback: { ...readbackSourcePins } }, journal,
    save() { fs.writeFileSync(path.join(dir, 'journal.json'), JSON.stringify(journal)); },
    socket: { readyState: WebSocket.OPEN } };
  const lifecycle = createTemporaryHomeLifecycle(product, tracking, {
    originalManifestOrigin: origin,
    originalManifestSha256: sha(origin),
    readbackSourcePins,
    readbackDirectory: dir,
    manifest,
    baselineReference: { path: baseline, sha256: sha(baseline) },
    startupState: { current: null, expectedNoCreate: null, launches: 0 },
  });
  return { dir, origin, tracking, session, original, journal, product, lifecycle };
}

function releaseFixture(name, releaseStatus = null, existingRelease = false) {
  const f = restoreFixture(name);
  f.lifecycle.restoreCase(name);
  f.lifecycle.restoreProducer(f.session, 'instanceA', name);
  const attempt = f.lifecycle.manifest.attempts[0];
  attempt.generation = '1';
  f.product.socket = { readyState: WebSocket.OPEN };
  const stopRequest = { schema: 'gogoke.37.operations.v1', family: 'K-SESSION', operation: 'stop',
    requestId: `stop-${name}`, domainId: 'projectA', targetId: f.session.id, expectedRevision: '0',
    payload: { generation: '1', seatId: f.session.seatId } };
  const stopReceipt = { schema: stopRequest.schema, family: stopRequest.family, operation: stopRequest.operation,
    requestId: stopRequest.requestId, targetId: stopRequest.targetId, previousRevision: '0', revision: '1',
    status: 'APPLIED', result: { stopFact: `sha256:${'1'.repeat(64)}` } };
  f.journal.operations.push({ request: stopRequest, rawFrame: JSON.stringify(stopRequest), receipt: stopReceipt });
  f.session.revision = '1';
  const calls = [];
  const makeRecord = (request, receipt) => ({ request, rawFrame: JSON.stringify(request), receipt });
  const addRelease = status => {
    const request = { schema: 'gogoke.37.operations.v1', family: 'K-SESSION', operation: 'admission-release',
      requestId: `release-${name}`, domainId: 'projectA', targetId: f.session.id, expectedRevision: '1',
      payload: { generation: '1', seatId: f.session.seatId } };
    const receipt = { schema: request.schema, family: request.family, operation: request.operation,
      requestId: request.requestId, targetId: request.targetId, previousRevision: '1', revision: '2',
      status, result: status === 'APPLIED' ? { generation: '1' } : { reason: status } };
    const record = makeRecord(request, receipt);
    f.journal.operations.push(record);
    return { record, receipt };
  };
  if (existingRelease) addRelease('APPLIED');
  f.product.operation = async (family, operation, targetId, payload, expectedRevision, allowed) => {
    calls.push({ family, operation, targetId, payload, expectedRevision, allowed });
    if (operation === 'admission-release') {
      const { record, receipt } = addRelease(releaseStatus ?? 'APPLIED');
      if (!allowed.includes(receipt.status)) throw Error(`Original ${operation} result=${receipt.status}; no replay`);
      return record.receipt;
    }
    assert.equal(operation, 'temporary-home-close');
    const request = { schema: 'gogoke.37.operations.v1', family, operation, requestId: `close-${name}`,
      domainId: 'projectA', targetId, expectedRevision: String(expectedRevision), payload };
    const receipt = { schema: request.schema, family, operation, requestId: request.requestId, targetId,
      previousRevision: '1', revision: '2', status: 'APPLIED', result: {
        state: 'CLOSED', homeId: `home-${name}`, nativeReceiptId: `native-${name}`,
        proofKind: 'NATIVE_EXACT_CLOSE', sourceFingerprint: `source-${name}`,
      } };
    const record = makeRecord(request, receipt);
    f.lifecycle.beforeDispatch(record);
    f.journal.operations.push(record);
    f.lifecycle.afterReceipt(record);
    return receipt;
  };
  return { ...f, attempt, calls };
}

test('retained attach binding rejects a second child and records only Home/Tauri identity', async () => {
  const product = Object.create(ActualProduct.prototype);
  const dir = path.join(tempRoot, 'attach'); fs.mkdirSync(dir);
  const pinFile = path.join(dir, 'pin.txt'); fs.writeFileSync(pinFile, 'pin');
  const indexPath = path.join(dir, 'resource-index.json');
  fs.writeFileSync(indexPath, JSON.stringify({ generationId: 'gen', sourceCommit: 'source', version: '0.1' }));
  const setId = sha(indexPath);
  const endpoint = { pid: 12, port: 3456, setId, generationId: 'gen',
    readyPath: path.join(dir, 'ready'), stderrPath: path.join(dir, 'stderr'), url: `http://gogoke-resource.localhost/${setId}/index.html` };
  fs.writeFileSync(endpoint.readyPath, JSON.stringify({ version: '0.1', setId, generationId: 'gen' }));
  fs.writeFileSync(endpoint.stderrPath, '');
  product.child = {}; product.config = { installedSha256: {}, installed: dir, version: '0.1', sourceCommit: 'source', registryKey: 'x' };
  await assert.rejects(() => product.attachRetained({ endpoint }), /unlaunched/);
  delete product.child;
  product.journal = {};
  product.save = () => {};
  product.verifyBytes = () => {};
  product.custody = async () => {};
  product.connectDiagnostic = async target => { product.socket = { readyState: WebSocket.OPEN, target, send() {} }; };
  product.evaluate = async () => ({ url: endpoint.url, home: true, tauri: true });
  const oldFetch = globalThis.fetch;
  globalThis.fetch = async () => ({ ok: true, async json() { return [{ type: 'page', url: endpoint.url,
    webSocketDebuggerUrl: 'ws://127.0.0.1:3456/devtools/page/1' }]; } });
  try {
    await product.attachRetained({ endpoint, sourcePins: { pin: { path: pinFile, sha256: sha(pinFile) } } });
    assert.deepEqual(product.journal.currentEndpoint, endpoint);
    assert.equal(product.journal.retainedAttachment.observationMode, 'ATTACHED_RETAINED_PRODUCT');
    assert.equal(product.journal.retainedAttachment.parentChildObservation, 'NOT_AVAILABLE_TO_CURRENT_OBSERVER');
  } finally { globalThis.fetch = oldFetch; }
});

test('restore lifecycle preserves original origin, refuses baseline recapture, and permits one exact resubmission', async () => {
  const f = restoreFixture('restore');
  const originalOriginBytes = fs.readFileSync(f.origin);
  f.lifecycle.restoreCase('restore');
  f.lifecycle.restoreProducer(f.session, 'instanceA', 'restore');
  await assert.rejects(() => f.lifecycle.captureBaseline(), /cannot recapture/);
  assert.throws(() => f.lifecycle.beginCase('restore'), /requires restoreCase/);
  const fake = Object.create(ActualProduct.prototype);
  fake.journal = f.journal; fake.temporaryHomeLifecycle = f.lifecycle; fake.save = f.product.save.bind(f.product);
  fake.evaluate = async expression => {
    assert.match(expression, /reserveA/);
    return JSON.stringify({ ...f.original.receipt, status: 'APPLIED', previousRevision: '0', revision: '1', result: { generation: '1' } });
  };
  const rawSha256 = createHash('sha256').update(f.original.rawFrame, 'utf8').digest('hex');
  const receipt = await fake.resubmitDeniedAdmission(f.original, { rawSha256 });
  assert.equal(receipt.status, 'APPLIED');
  assert.equal(f.lifecycle.manifest.attempts[0].originalReceipt.status, 'DENIED');
  assert.equal(f.lifecycle.manifest.attempts[0].resubmissionReceipt.status, 'APPLIED');
  await assert.rejects(() => fake.resubmitDeniedAdmission(f.original), /already has a resubmission/);
  assert.deepEqual(fs.readFileSync(f.origin), originalOriginBytes);
  assert.notEqual(path.resolve(f.tracking), path.resolve(f.origin));
});

test('restore rejects a same-byte copied reader when its original 8f path binding drifts', () => {
  const f = restoreFixture('source-path-drift');
  const copiedReader = path.join(f.dir, 'reader-copy.py');
  fs.copyFileSync(f.product.config.temporaryHomeReadback.path, copiedReader);
  const product = { ...f.product, config: { ...f.product.config,
    result: path.join(f.dir, 'source-path-drift-journal.json'),
    temporaryHomeReadback: { ...f.product.config.temporaryHomeReadback, path: copiedReader },
  } };
  assert.throws(() => createTemporaryHomeLifecycle(product, path.join(f.dir, 'source-path-drift-continuation.json'), {
    originalManifestOrigin: f.origin, originalManifestSha256: sha(f.origin),
    readbackSourcePins: f.product.config.temporaryHomeReadback, readbackDirectory: f.dir,
    manifest: structuredClone(f.lifecycle.manifest),
    baselineReference: { path: f.dir + '\\baseline.json', sha256: sha(f.dir + '\\baseline.json') },
  }), /binding is incomplete/);
});

test('resubmission preserves UNKNOWN and rejects changed raw without a third dispatch', async () => {
  const f = restoreFixture('unknown');
  f.lifecycle.restoreCase('unknown'); f.lifecycle.restoreProducer(f.session, 'instanceA', 'unknown');
  const fake = Object.create(ActualProduct.prototype); fake.journal = f.journal; fake.temporaryHomeLifecycle = f.lifecycle;
  let calls = 0; fake.save = f.product.save.bind(f.product); fake.evaluate = async () => { calls++; throw Error('transport unknown'); };
  await assert.rejects(() => fake.resubmitDeniedAdmission(f.original, { rawSha256: 'b'.repeat(64) }), /raw-frame/);
  await assert.rejects(() => fake.resubmitDeniedAdmission(f.original), /transport unknown/);
  assert.equal(calls, 1);
  await assert.rejects(() => fake.resubmitDeniedAdmission(f.original), /already has a resubmission/);
});

test('settlement releases the stopped original H once before F close and repeated finish is read-only', async () => {
  const f = releaseFixture('release-once');
  const first = await f.lifecycle.finishCase('release-once');
  assert.equal(first.complete, true, first.errors.join('; '));
  assert.deepEqual(f.calls.map(row => row.operation), ['admission-release', 'temporary-home-close']);
  assert.equal(f.attempt.releaseDispatchState, 'RECEIPT_RECORDED');
  assert.equal(f.attempt.releaseReceipt.status, 'APPLIED');
  const second = await f.lifecycle.finishCase('release-once');
  assert.equal(second.complete, true, second.errors.join('; '));
  assert.deepEqual(f.calls.map(row => row.operation), ['admission-release', 'temporary-home-close']);
});

test('settlement reuses an existing exact H release without dispatching another release', async () => {
  const f = releaseFixture('release-existing', null, true);
  const result = await f.lifecycle.finishCase('release-existing');
  assert.equal(result.complete, true, result.errors.join('; '));
  assert.deepEqual(f.calls.map(row => row.operation), ['temporary-home-close']);
  assert.equal(f.attempt.hReleaseSource.requestId, 'release-release-existing');
});

test('settlement retains DENIED and UNKNOWN H release outcomes and never continues to F close', async () => {
  for (const status of ['DENIED', 'UNKNOWN']) {
    const f = releaseFixture(`release-${status.toLowerCase()}`, status);
    const first = await f.lifecycle.finishCase(`release-${status.toLowerCase()}`);
    assert.equal(first.complete, false);
    assert.deepEqual(f.calls.map(row => row.operation), ['admission-release']);
    assert.equal(f.attempt.releaseReceipt.status, status);
    assert.equal(f.attempt.teardownState, `CASE_TEARDOWN_INCOMPLETE_RELEASE_${status}`);
    const second = await f.lifecycle.finishCase(`release-${status.toLowerCase()}`);
    assert.equal(second.complete, false);
    assert.deepEqual(f.calls.map(row => row.operation), ['admission-release']);
  }
});

test('new and existing release receipts require native schema and original generation before F close', async () => {
  for (const existing of [false, true]) for (const [index, mutate] of [
    r => { r.schema = 'foreign'; }, r => { r.result.generation = '999'; },
    r => { delete r.result.generation; },
  ].entries()) {
    const name = `release-contract-${existing}-${index}`;
    const f = releaseFixture(name, null, existing);
    if (existing) mutate(f.journal.operations.at(-1).receipt);
    else {
      const operation = f.product.operation;
      f.product.operation = async (...args) => {
        const reply = await operation(...args);
        if (args[1] === 'admission-release') mutate(reply);
        return reply;
      };
    }
    const result = await f.lifecycle.finishCase(name);
    assert.equal(result.complete, false);
    assert.equal(f.calls.some(row => row.operation === 'temporary-home-close'), false);
  }
});

test('failed recorded-close validation preserves its original diagnostic without calling the reader', async () => {
  const f = restoreFixture('close-original-cause');
  const close = closeFixture();
  Object.assign(f.journal, close);
  Object.assign(f.product.config, close.boundInstalledIdentity);
  f.product.endpoint = { pid: 10 };
  f.journal.causalCloseAttempts[0].source.module.sha256 = '0'.repeat(64);
  assert.equal(await f.lifecycle.verifyClosedReadback(), false);
  assert.match(f.lifecycle.manifest.sharedReadbackError, /Recorded normal close source/);
  assert.match(f.journal.temporaryHomeLifecycle.error, /Recorded normal close source/);
  assert.equal(fs.existsSync(path.join(f.dir, 'temporary-home-readback-verify-input.json')), false);
});

test('attached causal close requires two real OS exits and never accepts a fake NodeChild exit', () => {
  const ready = { shell: { pid: 10, creationUtc: 's' }, native: { pid: 20, creationUtc: 'n' } };
  const observed = { ready, armed: { token: 'x', waitResult: 'WAIT_TIMEOUT', waitMilliseconds: 0,
    shellPid: 10, shellCreationUtc: 's', nativePid: 20, nativeCreationUtc: 'n' },
    events: [{ role: 'shell', pid: 10, creationUtc: 's', code: 0, codeBasis: 'GET_EXIT_CODE_PROCESS' },
      { role: 'native', pid: 20, creationUtc: 'n', code: 0, codeBasis: 'GET_EXIT_CODE_PROCESS' }], helperExitCode: 0, complete: true };
  assert.deepEqual(Object.keys(qualifyAttachedCausalExit(ready, observed)), ['shell', 'native']);
  assert.throws(() => qualifyAttachedCausalExit(ready, { ...observed,
    events: [{ ...observed.events[0], code: 1 }, observed.events[1]] }), /exit/);
  const journal = closeFixture();
  assert.equal(requireRecordedNormalClose(journal, { pid: 10 }, undefined, journal.boundInstalledIdentity).mode, 'ATTACHED_RETAINED_OS_HANDLES');
  assert.throws(() => requireRecordedNormalClose({ ...journal,
    closes: [{ ...journal.closes[0], parentChildObservation: 'NOT_AVAILABLE_ORIGINAL_DRIVER_EXITED' }] },
  { pid: 10 }, undefined, journal.boundInstalledIdentity), /mode-specific/);
  assert.throws(() => requireRecordedNormalClose({ ...journal,
    closes: [{ ...journal.closes[0], exitCode: 0 }] }, { pid: 10 }, undefined, journal.boundInstalledIdentity), /mode-specific/);
});

test('recorded close rejects nonzero exits, substituted identities, missing caption and source drift', () => {
  for (const mutate of [
    j => { j.causalCloseAttempts[0].events[0].code = 1; },
    j => { j.causalCloseAttempts[0].events[0].pid = 999; },
    j => { j.causalCloseAttempts[0].events[0].codeBasis = 'SYNTHETIC'; },
    j => { j.causalCloseAttempts[0].pausedCall = null; },
    j => { j.causalCloseAttempts[0].source.module.sha256 = '0'.repeat(64); },
    j => { j.causalCloseAttempts[0].pausedCall.arm.token = 'other'; },
  ]) {
    const j = closeFixture(); mutate(j);
    assert.throws(() => requireRecordedNormalClose(j, { pid: 10 }, undefined, j.boundInstalledIdentity));
  }
});

test('second APPLIED receipt requires exact native schema, 0-to-1 revision and generation', async () => {
  const changes = [r => { r.schema = 'foreign'; }, r => { r.previousRevision = '999'; },
    r => { r.revision = 'not-a-revision'; }, r => { r.revision = '9'; },
    r => { r.result.generation = '999'; }];
  for (const [index, mutate] of changes.entries()) {
    const f = restoreFixture(`receipt-shape-${index}`);
    f.lifecycle.restoreCase(f.journal.caseId); f.lifecycle.restoreProducer(f.session, 'instanceA');
    const reply = { ...structuredClone(f.original.receipt), status: 'APPLIED', revision: '1', result: { generation: '1' } };
    mutate(reply);
    const fake = Object.create(ActualProduct.prototype);
    Object.assign(fake, { journal: f.journal, temporaryHomeLifecycle: f.lifecycle,
      save: f.product.save.bind(f.product), evaluate: async () => JSON.stringify(reply) });
    await assert.rejects(() => fake.resubmitDeniedAdmission(f.original), /identity differs/);
    assert.equal(f.lifecycle.manifest.attempts[0].receipt.status, 'DENIED');
    assert.equal(f.lifecycle.manifest.attempts[0].resubmissionState, 'UNKNOWN');
  }
});

test('native early reason-only refusal remains known DENIED and cannot dispatch a third reserve', async () => {
  const f = restoreFixture('reason-only-refusal');
  f.lifecycle.restoreCase(f.journal.caseId); f.lifecycle.restoreProducer(f.session, 'instanceA');
  const fake = Object.create(ActualProduct.prototype);
  Object.assign(fake, { journal: f.journal, temporaryHomeLifecycle: f.lifecycle,
    save: f.product.save.bind(f.product), evaluate: async () => JSON.stringify({
      ...f.original.receipt, result: { reason: 'original native early refusal' } }) });
  await assert.rejects(() => fake.resubmitDeniedAdmission(f.original), /result=DENIED; no replay/);
  assert.equal(f.lifecycle.manifest.attempts[0].resubmissionState, 'REJECTED_DENIED');
  assert.equal(f.lifecycle.manifest.attempts[0].receipt.status, 'DENIED');
  await assert.rejects(() => fake.resubmitDeniedAdmission(f.original), /already has a resubmission/);
});

test('durable READY requires original bound image hash, physical identity and ancestry', () => {
  for (const mutate of [
    j => { delete j.causalCloseAttempts[0].ready.shell.image; },
    j => { j.causalCloseAttempts[0].ready.shell.image.sha256 = 'c'.repeat(64); },
    j => { delete j.causalCloseAttempts[0].ready.native.image; },
    j => { j.causalCloseAttempts[0].ready.native.image.sha256 = 'c'.repeat(64); },
    j => { delete j.causalCloseAttempts[0].ready.native.image.fileId; },
    j => { delete j.causalCloseAttempts[0].ready.nativeAncestorChain; },
  ]) {
    const j = closeFixture(); mutate(j);
    const durable = JSON.parse(JSON.stringify(j));
    assert.throws(() => requireRecordedNormalClose(durable, { pid: 10 }, undefined,
      durable.boundInstalledIdentity), /READY image/);
  }
});

test('invalid second receipt never promotes the original DENIED attempt', async () => {
  const f = restoreFixture('wrong-second-receipt');
  f.lifecycle.restoreCase(f.journal.caseId); f.lifecycle.restoreProducer(f.session, 'instanceA');
  const fake = Object.create(ActualProduct.prototype);
  Object.assign(fake, { journal: f.journal, temporaryHomeLifecycle: f.lifecycle,
    save: f.product.save.bind(f.product), evaluate: async () => JSON.stringify({
      ...f.original.receipt, requestId: 'other-request', status: 'APPLIED' }) });
  await assert.rejects(() => fake.resubmitDeniedAdmission(f.original), /identity differs/);
  assert.equal(f.lifecycle.manifest.attempts[0].receipt.status, 'DENIED');
  assert.equal(f.lifecycle.manifest.attempts[0].resubmissionState, 'UNKNOWN');
  await assert.rejects(() => fake.resubmitDeniedAdmission(f.original), /already has a resubmission/);
});

test('restored original raw mismatch refuses before any second dispatch', async () => {
  const f = restoreFixture('raw-chain-drift');
  f.lifecycle.restoreCase(f.journal.caseId); f.lifecycle.restoreProducer(f.session, 'instanceA');
  f.original.request.payload.seatId = 'other-seat';
  f.original.rawFrame = JSON.stringify(f.original.request);
  const fake = Object.create(ActualProduct.prototype);
  Object.assign(fake, { journal: f.journal, temporaryHomeLifecycle: f.lifecycle, save() {},
    evaluate() { assert.fail('Native dispatch must not run'); } });
  await assert.rejects(() => fake.resubmitDeniedAdmission(f.original), /exact first owned attempt/);
});

test('restoration rejects a supplied manifest or baseline root differing from original pinned contents', () => {
  for (const axis of ['manifest', 'root']) {
    const f = restoreFixture(`restore-${axis}-mismatch`);
    const pins = f.product.config.temporaryHomeReadback;
    const manifest = JSON.parse(fs.readFileSync(f.origin, 'utf8'));
    if (axis === 'manifest') manifest.state = 'CHANGED';
    const product = { ...f.product, config: { ...f.product.config,
      stateRoot: axis === 'root' ? path.join(f.dir, 'other') : f.dir } };
    assert.throws(() => createTemporaryHomeLifecycle(product, path.join(f.dir, 'different-tracking.json'), {
      originalManifestOrigin: f.origin, originalManifestSha256: sha(f.origin), manifest,
      readbackSourcePins: pins, baselineReference: { path: path.join(f.dir, 'baseline.json'),
        sha256: sha(path.join(f.dir, 'baseline.json')) },
    }), axis === 'manifest' ? /pinned original content/ : /baseline original case/);
  }
});

test('restored scope refuses ordinary new create requests and additional producers before dispatch', async () => {
  const f = restoreFixture('restored-scope');
  f.lifecycle.restoreCase(f.journal.caseId); f.lifecycle.restoreProducer(f.session, 'instanceA');
  const fake = Object.create(ActualProduct.prototype);
  Object.assign(fake, { config: f.product.config, journal: f.journal, temporaryHomeLifecycle: f.lifecycle,
    save() {}, evaluate() { assert.fail('New create cannot dispatch in restored scope'); } });
  for (const operation of ['admission-reserve', 'resume'])
    await assert.rejects(() => fake.operation('K-SESSION', operation, f.session.id,
      { generation: '1', seatId: f.session.seatId }), /no new create/);
  assert.throws(() => f.lifecycle.registerProducer(f.session, 'instanceA'), /cannot register/);
  assert.throws(() => f.lifecycle.beforeReconciliation(f.original), /only the explicit original DENIED/);
  assert.equal(f.lifecycle.manifest.attempts.length, 1);
  assert.equal(f.lifecycle.manifest.producers.length, 1);
});
