import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { createHash } from 'node:crypto';
import { createStartupHomeLifecycle } from '../installed-startup-home-lifecycle.mjs';

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'gogoke-retained-ready-'));
test.after(() => fs.rmSync(root, { recursive: true, force: true }));
const sha = file => createHash('sha256').update(fs.readFileSync(file)).digest('hex');

function fixture(name, changeOriginal = () => {}, changeCurrent = () => {}) {
  const directory = path.join(root, name); fs.mkdirSync(directory);
  const endpoint = { pid: 101, port: 1234, setId: 'a'.repeat(64), generationId: 'generationA',
    readyPath: path.join(directory, 'original.ready'), stderrPath: path.join(directory, 'stderr.log'),
    url: `http://localhost/${'a'.repeat(64)}/index.html` };
  const config = { domainId: 'projectA', repositoryId: 'gogokeSeatTestbed', sourceCommit: 'b'.repeat(40), version: '0.1.72',
    stateRoot: path.join(directory, 'product-root'), installedSha256: { 'gogoke.exe': 'c'.repeat(64) } };
  const error = 'Error: Actual product readiness missing: original unchanged observation failed';
  const original = { state: 'FAIL_ORIGINAL_REQUESTS_RETAINED', caseId: 'caseA', acceptance: false, error,
    sourceCommit: config.sourceCommit, candidateVersion: config.version,
    candidateInstalledSha256: config.installedSha256, domainId: config.domainId, repositoryId: config.repositoryId,
    stateRoot: config.stateRoot, evidenceDirectory: directory,
    operations: [], launches: [], closes: [], failedProductHeld: true,
    currentEndpoint: { ...endpoint }, productSpawnAttemptUtc: '2026-10-18T06:28:17.978Z',
    productSpawnReturnedUtc: '2026-10-18T06:28:18.448Z',
    readinessTiming: { readyAcceptedBeforeDeadline: false, readyPresentAtDecision: false,
      spawnAttemptUtc: '2026-10-18T06:28:17.978Z', spawnReturnedUtc: '2026-10-18T06:28:18.448Z',
      deadlineAtUtc: '2026-10-18T06:31:18.450Z', decisionAtUtc: '2026-10-18T06:31:18.548Z',
      readyPath: endpoint.readyPath },
    startupReadiness: { targetUrl: null, diagnosticConnected: false,
      readyReceiptPresent: false, readyAcceptedBeforeDeadline: false } };
  original.stopWorktreePrepare = Object.fromEntries(['state', 'acceptance', 'domainId', 'repositoryId',
    'sourceCommit', 'stateRoot', 'evidenceDirectory', 'candidateVersion', 'candidateInstalledSha256', 'error']
    .map(key => [key, original[key]]));
  changeOriginal(original);
  const failurePath = path.join(directory, 'original-failure.json');
  fs.writeFileSync(failurePath, JSON.stringify(original));
  const reference = { path: failurePath, sha256: sha(failurePath) };
  const ui = { url: endpoint.url, home: true, tauri: true };
  const launch = { ...endpoint, sourceCommit: config.sourceCommit,
    bootstrap: { setId: endpoint.setId, generationId: endpoint.generationId }, ui,
    readinessQualification: 'LATE_AFTER_ORIGINAL_FAILURE_NOT_ACCEPTANCE',
    originalReadinessFailure: reference };
  const journal = { caseId: 'caseA', error, operations: [], startupHomeReads: [], launches: [launch],
    currentEndpoint: endpoint, originalReadinessFailure: reference,
    retainedAttachment: { observationMode: 'ATTACHED_RETAINED_PRODUCT', endpoint, ui } };
  const row = { caseId: 'caseA_no_startup_0', sourceKind: 'NATIVE_STARTUP_NO_CREATE',
    state: 'CLOSED_PREDECESSOR', seatId: 'secretary', seatIncarnation: 'incarnationA',
    instanceId: 'instanceA', sessionId: 'residentA', oldGeneration: '25', oldRevision: '12',
    expectedCreateRequestId: 'original-resume' };
  const manifest = { startupAttempts: [], noStartupHomes: [row], authorizedRepairs: [],
    cases: [{ caseId: row.caseId, sourceKind: row.sourceKind, state: 'RUNNING' }] };
  const nativeResume = { schema: 'gogoke.37.operations.v1', family: 'K-SESSION', operation: 'resume',
    requestId: row.expectedCreateRequestId, targetId: row.sessionId, status: 'CONFLICT',
    previousRevision: '12', revision: '12', result: {} };
  const current = { schema: 'gogoke.37.secretary-configuration.v1', state: 'DESIGNATED',
    seatState: 'BUSY', seatId: row.seatId, incarnation: row.seatIncarnation, instanceId: row.instanceId,
    conversation: { state: 'FOUND', sessionId: row.sessionId, generation: '25', revision: '12',
      claimState: 'STOPPED', runtimeAvailable: false, stoppedFact: true },
    startupReason: 'Original Secretary resume result: ' + JSON.stringify(nativeResume) };
  changeCurrent(current);
  let reads = 0;
  const product = { config, journal, endpoint, socket: { readyState: WebSocket.OPEN }, save() {},
    async evaluate() { reads++; return JSON.stringify(current); } };
  const lifecycle = createStartupHomeLifecycle(product, manifest, () => {}, null, null, {
    current: null, expectedNoCreate: structuredClone(row), launches: 1,
    originalManifestOrigin: path.join(directory, 'original-home-manifest.json'),
    originalManifestSha256: 'd'.repeat(64) });
  return { lifecycle, manifest, journal, failurePath, original, get reads() { return reads; } };
}

test('late retained readiness permits only a current no-startup fact, preserving failed bytes', async () => {
  const f = fixture('valid'); const before = fs.readFileSync(f.failurePath);
  await f.lifecycle.observeRetainedNoStartup();
  assert.equal(f.reads, 1);
  assert.equal(f.manifest.noStartupHomes[0].retainedMetadataObservation.state, 'QUALIFIED_CURRENT_READ');
  assert.equal(f.manifest.noStartupHomes[0].bootstrap.reply.conversation.generation, '25');
  assert.deepEqual(fs.readFileSync(f.failurePath), before);
  assert.equal(JSON.parse(before).state, 'FAIL_ORIGINAL_REQUESTS_RETAINED');
  assert.equal(f.journal.operations.length, 0);
});

for (const [name, mutate] of [
  ['already-accepted', value => { value.readinessTiming.readyAcceptedBeforeDeadline = true; }],
  ['different-case', value => { value.caseId = 'another-case'; }],
  ['missing-case', value => { delete value.caseId; }],
  ['acceptance-claim', value => { value.acceptance = true; }],
  ['nested-prepare-mismatch', value => { value.stopWorktreePrepare = { state: 'RUNNING' }; }],
  ['nested-domain-mismatch', value => { value.stopWorktreePrepare.domainId = 'otherDomain'; }],
  ['nested-repository-mismatch', value => { value.stopWorktreePrepare.repositoryId = 'otherRepository'; }],
  ['nested-acceptance-mismatch', value => { value.stopWorktreePrepare.acceptance = true; }],
  ['different-root', value => { value.stateRoot += '-other'; }],
  ['different-spawn', value => { value.currentEndpoint.pid++; }],
  ['business-dispatched', value => { value.operations.push({ request: {} }); }],
  ['already-exited', value => { value.productExits = [{ code: 0 }]; }],
  ['not-held', value => { value.failedProductHeld = false; }],
  ['early-decision', value => { value.readinessTiming.decisionAtUtc = '2026-10-18T06:28:18.500Z'; }],
  ['not-ready-failure', value => { value.error = value.stopWorktreePrepare.error = 'unrelated error'; }],
]) test(`retained readiness refuses ${name} before any current read`, async () => {
  const f = fixture(name, mutate);
  await assert.rejects(() => f.lifecycle.observeRetainedNoStartup(), /same untouched failed original spawn/);
  assert.equal(f.reads, 0);
});

test('changed original failure bytes cannot qualify a retained read', async () => {
  const f = fixture('changed-bytes'); fs.appendFileSync(f.failurePath, '\n');
  await assert.rejects(() => f.lifecycle.observeRetainedNoStartup(), /journal bytes differ/);
  assert.equal(f.reads, 0);
});

test('readiness source validation and parsing use the same captured bytes', async () => {
  const f = fixture('one-byte-snapshot');
  const originalRead = fs.readFileSync;
  let reads = 0;
  fs.readFileSync = function (file, ...args) {
    const bytes = originalRead.call(fs, file, ...args);
    if (file === f.failurePath) {
      reads++;
      const changed = JSON.parse(bytes.toString('utf8')); changed.caseId = 'replaced-after-capture';
      fs.writeFileSync(file, JSON.stringify(changed));
    }
    return bytes;
  };
  try { await f.lifecycle.observeRetainedNoStartup(); }
  finally { fs.readFileSync = originalRead; }
  assert.equal(reads, 1);
  assert.equal(f.reads, 1);
});

test('a later startup generation is refused instead of being called no-create', async () => {
  const f = fixture('new-generation', () => {}, value => {
    value.conversation.generation = '26'; value.conversation.claimState = 'COMMITTED';
    value.conversation.runtimeAvailable = true; value.conversation.stoppedFact = null;
  });
  await assert.rejects(() => f.lifecycle.observeRetainedNoStartup(), /startup identity\/refusal differs/);
  assert.equal(f.reads, 1);
  assert.equal(f.manifest.noStartupHomes[0].retainedMetadataObservation.state, 'UNQUALIFIED_CURRENT_READ');
  assert.equal(Object.hasOwn(f.manifest.noStartupHomes[0], 'bootstrap'), false);
});

test('a label without a pinned readiness failure does not replace the legacy source guard', async () => {
  const f = fixture('label-only'); delete f.journal.originalReadinessFailure;
  await assert.rejects(() => f.lifecycle.observeRetainedNoStartup(), /exact failed original launch/);
  assert.equal(f.reads, 0);
});
