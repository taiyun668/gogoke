// Offline synthetic transport controls. These do not execute the installed product.
import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { createHash } from 'node:crypto';

const here = path.dirname(fileURLToPath(import.meta.url));
const { createStartupHomeLifecycle, startupLifecycleSource } = await import(
  pathToFileURL(path.join(here, 'installed-startup-home-lifecycle.mjs')));
const { ActualProduct } = await import(pathToFileURL(path.join(here, 'product-cdp.mjs')));
const sha = file => createHash('sha256').update(fs.readFileSync(file)).digest('hex');
console.log(JSON.stringify({ syntheticOnly: true, acceptance: false,
  startupSourceSha256: sha(path.join(here, 'installed-startup-home-lifecycle.mjs')),
  ingressSha256: sha(path.join(here, 'product-cdp.mjs')) }));

const fixtureRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'gogoke-startup-home-test-'));
test.after(() => {
  const relative = path.relative(path.resolve(os.tmpdir()), path.resolve(fixtureRoot));
  assert.ok(relative && !relative.startsWith('..') && !path.isAbsolute(relative));
  fs.rmSync(fixtureRoot, { recursive: true, force: true });
});

const plan = { state: 'RESUME_EXPECTED', domainId: 'global', seatId: 'secretarySeat',
  seatIncarnation: 'secretaryIncarnation', instanceId: 'codexInstance', sessionId: 'residentSession',
  oldGeneration: '24', oldRevision: '10', expectedGeneration: '25',
  expectedCreateRequestId: 'resident-resume-source' };
const repairIntent = { homeId: 'home24', instanceId: 'codexInstance', domainId: 'global',
  sessionId: 'residentSession', generation: '24', expectedRevision: '1',
  provenance: { path: 'source.json', sha256: 'a'.repeat(64) },
  beforeHome: { home_id: 'home24', state: 'ACTIVE', revision: '1' } };
const stopFact = `sha256:${'b'.repeat(64)}`;

function decodeInvocation(expression) {
  const at = expression.indexOf('{frame:');
  assert.ok(at >= 0 && expression.endsWith('})'));
  return JSON.parse(JSON.parse(expression.slice(at + 7, -2)));
}

function fixture(name, { startup = plan, repair = false, stopStatus = 'APPLIED',
    startupCloseStatus = 'APPLIED', repairCloseStatus = 'APPLIED' } = {}) {
  const dir = path.join(fixtureRoot, name); fs.mkdirSync(dir);
  const journal = { caseId: name, operations: [] };
  const manifest = { schema: 'gogoke.37.temporary-home-attempts.v1', caseId: name,
    domainId: 'global', cases: [], attempts: [], producers: [] };
  const calls = [], saved = [];
  const product = { config: { domainId: 'global', evidenceDirectory: dir }, journal,
    save() { saved.push(structuredClone(journal)); },
    async evaluate(expression) {
      const frame = decodeInvocation(expression);
      if (frame.command === 'secretary-configuration-read') {
        calls.push({ kind: 'configuration', frame });
        return JSON.stringify(startup.state === 'NONE'
          ? { schema: 'gogoke.37.secretary-configuration.v1', state: 'UNSET',
              conversation: { state: 'NONE' } }
          : { schema: 'gogoke.37.secretary-configuration.v1', state: 'DESIGNATED',
              seatId: plan.seatId, incarnation: plan.seatIncarnation, instanceId: plan.instanceId,
              generation: '24', revision: '8',
              conversation: { state: 'FOUND', sessionId: plan.sessionId,
                generation: '25', revision: '9', claimState: 'COMMITTED',
                runtimeAvailable: true, stoppedFact: null } });
      }
      const pending = journal.operations.at(-1);
      assert.deepEqual(pending.request, frame);
      assert.equal(pending.receipt, null);
      calls.push({ kind: 'operation', frame });
      const status = frame.operation === 'stop' ? stopStatus :
        frame.payload.historicalHomeId ? (typeof repairCloseStatus === 'function'
          ? repairCloseStatus(frame.payload.historicalHomeId) : repairCloseStatus) : startupCloseStatus;
      const denied = ['DENIED', 'CONFLICT', 'STALE'].includes(status);
      return JSON.stringify({ schema: frame.schema, family: frame.family,
        operation: frame.operation, requestId: frame.requestId, targetId: frame.targetId,
        status, previousRevision: frame.expectedRevision,
        revision: denied ? '1' : frame.operation === 'stop' ? '10' : '2',
        result: frame.operation === 'stop' ? { stopFact } :
          denied ? { reason: 'native temporary HOME close: original proof refused' } :
          status === 'UNKNOWN' ? {} : { homeId: frame.payload.historicalHomeId ?? 'home25',
            state: 'CLOSED', nativeReceiptId: 'native-' + frame.requestId,
            sourceFingerprint: 'c'.repeat(64) } });
    } };
  let lifecycle;
  const save = () => saved.push(structuredClone(manifest));
  lifecycle = createStartupHomeLifecycle(product, manifest, save,
    async (mode, _baseline, phase) => {
      assert.equal(mode, 'startup'); assert.equal(phase, 'startup-0');
      return { report: { startup }, reference: { file: 'prelaunch.json', sha256: 'd'.repeat(64) } };
    }, () => ({ file: 'baseline.json' }));
  product.temporaryHomeLifecycle = {
    beforeDispatch: record => lifecycle.beforeDispatch(record),
    afterReceipt: record => lifecycle.afterReceipt(record),
    afterDispatchError: (record, error) => lifecycle.afterError(record, error),
    afterRejectedReceipt: record => lifecycle.afterError(record, Error('rejected')),
    beforeCaptionClose: () => lifecycle.settle(),
  };
  product.operation = (...args) => ActualProduct.prototype.operation.call(product, ...args);
  if (repair) lifecycle.initializeRepairs({ authorizedRepairs: [repairIntent] });
  return { product, lifecycle, manifest, journal, calls, saved };
}

test('actual shared USER ingress closes repair24, then stops and closes startup25', async () => {
  const f = fixture('healthy', { repair: true });
  await f.lifecycle.beforeLaunch();
  await f.lifecycle.afterLaunch();
  assert.equal(f.manifest.startupAttempts[0].bootstrap.reply.generation, '24');
  assert.equal(f.manifest.startupAttempts[0].bootstrap.reply.conversation.generation, '25');
  const settled = await f.lifecycle.settle();
  assert.equal(settled.complete, true, settled.errors.join('; '));
  const sent = f.calls.filter(row => row.kind === 'operation').map(row => row.frame);
  assert.deepEqual(sent.map(row => row.operation),
    ['temporary-home-close', 'stop', 'temporary-home-close']);
  assert.deepEqual(sent[0].payload,
    { sessionId: plan.sessionId, historicalHomeId: 'home24' });
  assert.equal(sent[0].expectedRevision, '1');
  assert.deepEqual(sent[1].payload, { generation: '25', seatId: plan.seatId });
  assert.equal(sent[1].expectedRevision, '9');
  assert.deepEqual(sent[2].payload,
    { sessionId: plan.sessionId, originalCreateRequestId: plan.expectedCreateRequestId });
  assert.equal(sent[2].expectedRevision, '1');
  assert.equal(f.manifest.startupAttempts[0].hStopSource.stopFact, stopFact);
  assert.equal(f.manifest.authorizedRepairs[0].closeReceipt.result.homeId, 'home24');
  assert.equal(f.manifest.startupAttempts[0].closeReceipt.result.homeId, 'home25');
});

test('fifteen explicit repairs retain known refusals and continue independent selectors', async () => {
  const f = fixture('mixedFifteen', { repair: true, repairCloseStatus: home =>
    ({home24:'DENIED',home0:'CONFLICT',home1:'STALE'})[home] ?? 'APPLIED' });
  f.lifecycle.initializeRepairs({ authorizedRepairs: Array.from({length:14}, (_,i) =>
    ({...repairIntent, homeId:'home'+i, beforeHome:{...repairIntent.beforeHome,home_id:'home'+i}})) });
  await f.lifecycle.beforeLaunch(); await f.lifecycle.afterLaunch();
  const settled = await f.lifecycle.settle();
  assert.equal(settled.complete, true, settled.errors.join('; '));
  const frames = f.calls.filter(row => row.kind === 'operation').map(row => row.frame);
  assert.equal(frames.length,17);
  assert.deepEqual(frames.slice(-2).map(frame => frame.operation),['stop','temporary-home-close']);
  assert.equal(new Set(frames.slice(0,15).map(frame => frame.payload.historicalHomeId)).size,15);
  assert.equal(f.manifest.cases.filter(row => row.state==='CASE_HOME_CLOSE_REFUSED').length,3);
  await f.lifecycle.settle();
  assert.equal(f.calls.filter(row => row.kind === 'operation').length,17);
  const closedFact = row => ({caseId:row.caseId,domainId:row.domainId,sessionId:row.sessionId,
    instanceId:row.instanceId,homeId:row.closeReceipt.result.homeId,state:'CLOSED',revision:'2',
    originalCreateRequestId:row.originalCreateRequestId,closeRequestId:row.closeRequestId,
    nativeReceiptId:row.closeReceipt.result.nativeReceiptId,sourceFingerprint:row.closeReceipt.result.sourceFingerprint});
  const facts = {startupHomes:f.manifest.startupAttempts.map(closedFact),
    authorizedRepairs:f.manifest.authorizedRepairs.filter(row=>row.closeReceipt.status==='APPLIED').map(closedFact),
    authorizedRepairRefusals:f.manifest.authorizedRepairs.filter(row=>row.closeReceipt.status!=='APPLIED').map(row=>
      ({caseId:row.caseId,domainId:row.domainId,sessionId:row.sessionId,instanceId:row.instanceId,
        homeId:row.homeId,state:'ACTIVE',revision:'1',closeRequestId:row.closeRequestId,
        status:row.closeReceipt.status,reason:row.closeReceipt.result.reason}))};
  assert.equal(f.lifecycle.applyFacts(facts),true);
  facts.authorizedRepairRefusals[0].state='CLOSED';
  assert.equal(f.lifecycle.applyFacts(facts),false);
});

test('an UNKNOWN historical close stops later selectors without retry and still settles its own startup', async () => {
  const f=fixture('unknownRepair',{repair:true,repairCloseStatus:'UNKNOWN'});
  f.lifecycle.initializeRepairs({authorizedRepairs:[{...repairIntent,homeId:'laterHome',
    beforeHome:{...repairIntent.beforeHome,home_id:'laterHome'}}]});
  await f.lifecycle.beforeLaunch(); await f.lifecycle.afterLaunch();
  assert.equal((await f.lifecycle.settle()).complete,false);
  const frames=f.calls.filter(row=>row.kind==='operation').map(row=>row.frame);
  assert.deepEqual(frames.map(row=>row.operation),['temporary-home-close','stop','temporary-home-close']);
  assert.equal(frames[0].payload.historicalHomeId,'home24');
  assert.equal(f.manifest.authorizedRepairs[1].closeDispatchState,'NOT_DISPATCHED');
  assert.equal(f.manifest.startupAttempts[0].closeReceipt.result.state,'CLOSED');
  await assert.rejects(()=>ActualProduct.prototype.closeNormally.call(f.product),/teardown incomplete/);
  assert.equal(f.calls.filter(row=>row.kind==='operation').length,3);
});

test('UNKNOWN startup stop is retained and caption close remains blocked without replay', async () => {
  const f = fixture('unknownStop', { repair: true, stopStatus: 'UNKNOWN' });
  await f.lifecycle.beforeLaunch(); await f.lifecycle.afterLaunch();
  assert.equal((await f.lifecycle.settle()).complete, false);
  const first = f.calls.filter(row => row.kind === 'operation').map(row => row.frame.operation);
  assert.deepEqual(first, ['temporary-home-close', 'stop']);
  assert.equal(f.manifest.startupAttempts[0].hStopSource, undefined);
  assert.equal((await f.lifecycle.settle()).complete, false);
  await assert.rejects(() => ActualProduct.prototype.closeNormally.call(f.product),
    /Temporary-HOME teardown incomplete/);
  assert.deepEqual(f.calls.filter(row => row.kind === 'operation').map(row => row.frame.operation), first);
  assert.equal(f.journal.operations.filter(row => row.request.operation === 'stop').length, 1);
});

test('UNKNOWN startup close cannot be resent or followed by caption close', async () => {
  const f = fixture('unknownClose', { startupCloseStatus: 'UNKNOWN' });
  await f.lifecycle.beforeLaunch(); await f.lifecycle.afterLaunch();
  assert.equal((await f.lifecycle.settle()).complete, false);
  const first = f.calls.filter(row => row.kind === 'operation').map(row => row.frame.operation);
  assert.deepEqual(first, ['stop', 'temporary-home-close']);
  assert.equal((await f.lifecycle.settle()).complete, false);
  await assert.rejects(() => ActualProduct.prototype.closeNormally.call(f.product),
    /Temporary-HOME teardown incomplete/);
  assert.deepEqual(f.calls.filter(row => row.kind === 'operation').map(row => row.frame.operation), first);
  assert.equal(f.manifest.startupAttempts[0].closeDispatchState, 'RECEIPT_RECORDED');
});

test('BLOCKED source prevents launch dispatch; repair-only creates no new HOME', async () => {
  const blocked = fixture('blocked', { startup: { state: 'BLOCKED', reason: 'pending original resume' } });
  await assert.rejects(() => blocked.lifecycle.beforeLaunch(), /launch blocked/);
  assert.equal(blocked.calls.length, 0);
  assert.equal(blocked.manifest.startupAttempts.length, 0);

  const repairOnly = fixture('repairOnly', { startup: { state: 'NONE', domainId: 'global' }, repair: true });
  await repairOnly.lifecycle.beforeLaunch(); await repairOnly.lifecycle.afterLaunch();
  const settled = await repairOnly.lifecycle.settle();
  assert.equal(settled.complete, true, settled.errors.join('; '));
  assert.equal(repairOnly.lifecycle.hasCreatedHomes(), false);
  assert.equal(repairOnly.lifecycle.hasTrackedHomes(), true);
  assert.deepEqual(repairOnly.calls.filter(row => row.kind === 'operation').map(row => row.frame.operation),
    ['temporary-home-close']);
});

test('loaded lifecycle source guard rejects a changed copy', async () => {
  assert.equal(startupLifecycleSource().path,
    path.join(here, 'installed-startup-home-lifecycle.mjs'));
  const copy = path.join(fixtureRoot, 'source-guard-copy.mjs');
  fs.copyFileSync(path.join(here, 'installed-startup-home-lifecycle.mjs'), copy);
  const copied = await import(pathToFileURL(copy));
  assert.equal(copied.startupLifecycleSource().sha256, sha(copy));
  fs.appendFileSync(copy, '\n// changed after module load\n');
  assert.throws(() => copied.startupLifecycleSource(), /source changed/);
});
