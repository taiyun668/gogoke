import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { createTemporaryHomeLifecycle } from '../installed-temporary-home-lifecycle.mjs';
import { closeProjectOnlyAfterOwnedHome } from '../m2-history-boundaries.mjs';
import { ActualProduct } from '../product-cdp.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const digest = file => createHash('sha256').update(fs.readFileSync(file)).digest('hex');

test('the actual shared HOME baseline accepts the begun A/B case before any native attempt',
  { skip: process.platform !== 'win32' }, async () => {
    const base = fs.realpathSync(os.tmpdir());
    const dir = fs.mkdtempSync(path.join(base, 'gogoke-m2-history-baseline-'));
    try {
      const root = path.join(dir, 'root'), evidence = path.join(dir, 'evidence');
      fs.mkdirSync(root); fs.mkdirSync(evidence);
      const python = process.env.GOGOKE_TEST_PYTHON ?? 'python';
      const schema = `import sqlite3,sys\n` +
        `db=sqlite3.connect(sys.argv[1]);db.executescript('''` +
        `CREATE TABLE gogoke_v37_instance_homes(home_id,instance_id,domain_id,kind,owner_id,generation,directory_ref,directory_identity,state,revision);` +
        `CREATE TABLE gogoke_v37_instance_operations(request_id,request_hex,target_id,phase,receipt_json,native_receipt_id);` +
        `CREATE TABLE gogoke_v37_seat_secretary(singleton,domain_id,seat_id,incarnation);` +
        `CREATE TABLE gogoke_v37_seats(domain_id,seat_id,incarnation,instance_id,state,layer,kind,generation);` +
        `CREATE TABLE gogoke_v37_h_operation(domain_id,session_id,request_id,operation,raw_hex,status,previous_revision,revision);` +
        `CREATE TABLE gogoke_v37_h_process_episode(domain_id,session_id,request_id,raw_hex,generation,old_generation,phase,process_operation_id,binding_id,seat_id,seat_incarnation,home_id,instance_id,stop_fact_id,stop_request_id,result_revision);` +
        `CREATE TABLE gogoke_coordination_process_custody(operation_id,domain_id,profile_id,generation,state,stop_proof_hash);` +
        `''');db.commit();db.close()\n`;
      const setup = spawnSync(python, ['-c', schema, path.join(root, 'state.sqlite')],
        { encoding: 'utf8', windowsHide: true });
      assert.equal(setup.status, 0, setup.stderr || setup.error?.message);
      const reader = path.resolve(here, '..', 'installed-temporary-home-readback.py');
      const core = path.resolve(here, '..', 'installed_native_readback.py');
      const journal = { caseId: 'caseAB', sessions: [], operations: [], closes: [] };
      const product = { journal, config: { domainId: 'A', temporaryHomeDomains: ['A', 'B'],
        evidenceDirectory: evidence, stateRoot: root, python,
        temporaryHomeReadback: { path: reader, sha256: digest(reader),
          corePath: core, coreSha256: digest(core) } }, save() {} };
      const home = createTemporaryHomeLifecycle(product, path.join(evidence, 'temporary-home-attempts.json'));
      home.beginCase(journal.caseId);
      const baseline = await home.captureBaseline();
      assert.equal(baseline.state, 'BASELINE_MEASURED');
      assert.deepEqual(baseline.ownedDomains, ['A', 'B']);
      assert.equal(home.manifest.attempts.length, 0);
      assert.equal(home.manifest.cases.filter(row => row.caseId === journal.caseId).length, 1);
    } finally {
      assert.equal(path.dirname(dir), base);
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });

test('one owned HOME manifest distinguishes equal session IDs by original domain', () => {
  const base = fs.realpathSync(os.tmpdir());
  const dir = fs.mkdtempSync(path.join(base, 'gogoke-m2-history-owned-'));
  try {
    const journal = { caseId: 'caseA', sessions: [], operations: [], closes: [] };
    const product = { journal, config: { domainId: 'A', temporaryHomeDomains: ['A', 'B'],
      evidenceDirectory: dir, stateRoot: dir }, save() {} };
    const home = createTemporaryHomeLifecycle(product, path.join(dir, 'temporary-home-attempts.json'));
    home.beginCase(journal.caseId);
    const a = { id: 'sameSession', domainId: 'A', seatId: 'seatA' };
    const b = { id: 'sameSession', domainId: 'B', seatId: 'seatB' };
    journal.sessions.push(a, b);
    home.registerProducer(a, 'sameInstance');
    home.registerProducer(b, 'sameInstance');
    const admission = (domainId, seatId, requestId) => {
      const request = { schema: 'gogoke.37.operations.v1', family: 'K-SESSION',
        operation: 'admission-reserve', requestId, domainId, targetId: 'sameSession',
        expectedRevision: '0', payload: { generation: '1', seatId } };
      return { request, rawFrame: JSON.stringify(request), receipt: null };
    };
    home.beforeDispatch(admission('A', 'seatA', 'admissionA'));
    home.beforeDispatch(admission('B', 'seatB', 'admissionB'));
    assert.deepEqual(home.manifest.producers.map(row => [row.domainId, row.sessionId]),
      [['A', 'sameSession'], ['B', 'sameSession']]);
    assert.deepEqual(home.manifest.attempts.map(row => [row.domainId, row.originalCreateRequestId]),
      [['A', 'admissionA'], ['B', 'admissionB']]);
    assert.throws(() => home.beforeDispatch(admission('B', 'seatA', 'wrongSeat')),
      /Unanchored temporary-HOME producer/);
    assert.throws(() => home.beforeDispatch(admission('C', 'seatA', 'foreignDomain')),
      /Unanchored temporary-HOME producer/);
    assert.throws(() => home.registerProducer(a, 'sameInstance', 'otherCase'),
      /exact journaled session|configured instance/);
  } finally {
    assert.equal(path.dirname(dir), base);
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test('actual User operation preserves the exact raw receipt before parsing, including failure', async () => {
  const journal = { operations: [] };
  const product = { config: { domainId: 'A' }, journal, save() {},
    async evaluate(expression) {
      const request = JSON.parse(JSON.parse(expression.slice(expression.indexOf('{frame:') + 7, -2)));
      return ` { "schema":"${request.schema}", "family":"${request.family}",` +
        ` "operation":"${request.operation}", "requestId":"${request.requestId}",` +
        ` "targetId":"${request.targetId}", "status":"APPLIED",` +
        ` "previousRevision":"${request.expectedRevision}", "revision":"1", "result":{} } `;
    } };
  const receipt = await ActualProduct.prototype.operation.call(product,
    'K-SESSION', 'admission-release', 'sameSession', { generation: '1', seatId: 'seatA' },
    '0', ['APPLIED'], 'B');
  const record = journal.operations[0];
  assert.equal(record.request.domainId, 'B');
  assert.equal(record.rawReceipt.startsWith(' { '), true);
  assert.equal(record.rawReceipt.endsWith(' } '), true);
  assert.deepEqual(JSON.parse(record.rawReceipt), receipt);
  assert.equal(record.rawFrame, JSON.stringify(record.request));
  product.evaluate = async () => '{original invalid JSON';
  await assert.rejects(ActualProduct.prototype.operation.call(product,
    'K-SESSION', 'output-stream', 'sameSession', {}, '1', ['APPLIED'], 'B'), SyntaxError);
  assert.equal(journal.operations[1].rawReceipt, '{original invalid JSON');
  assert.equal(journal.operations[1].receipt, null);
});

test('actual User ingress settles B in its own domain through stop, release, and F close', async () => {
  const base = fs.realpathSync(os.tmpdir());
  const dir = fs.mkdtempSync(path.join(base, 'gogoke-m2-history-settle-'));
  try {
    const journal = { caseId: 'caseAB', sessions: [], operations: [], closes: [] };
    const sent = [];
    const product = { config: { domainId: 'A', temporaryHomeDomains: ['A', 'B'],
        evidenceDirectory: dir, stateRoot: dir }, journal, save() {},
      socket: { readyState: WebSocket.OPEN },
      operation: ActualProduct.prototype.operation,
      async evaluate(expression) {
        const request = JSON.parse(JSON.parse(expression.slice(expression.indexOf('{frame:') + 7, -2)));
        sent.push(request);
        const result = request.operation === 'stop'
          ? { stopFact: `sha256:${request.domainId === 'A' ? 'a' : 'b'}`.padEnd(71,
            request.domainId === 'A' ? 'a' : 'b') }
          : request.operation === 'temporary-home-close'
          ? { homeId: `home${request.domainId}`, state: 'CLOSED',
            nativeReceiptId: `receipt${request.domainId}`, proofKind: 'ORIGINAL_STOP',
            sourceFingerprint: 'c'.repeat(64) }
          : { generation: request.payload.generation };
        return JSON.stringify({ schema: request.schema, family: request.family,
          operation: request.operation, requestId: request.requestId, targetId: request.targetId,
          status: 'APPLIED', previousRevision: request.expectedRevision,
          revision: String(BigInt(request.expectedRevision) + 1n), result });
      } };
    const home = createTemporaryHomeLifecycle(product, path.join(dir, 'temporary-home-attempts.json'));
    home.beginCase(journal.caseId);
    for (const domainId of ['A', 'B']) {
      const session = { id: 'sameSession', domainId, seatId: `seat${domainId}`,
        instanceId: 'sameInstance', generation: '1', revision: '0' };
      journal.sessions.push(session);
      home.registerProducer(session, session.instanceId);
      for (const operation of ['admission-reserve', 'admission-commit', 'open']) {
        const receipt = await product.operation('K-SESSION', operation, session.id,
          { generation: session.generation, seatId: session.seatId }, session.revision,
          ['APPLIED'], domainId);
        session.revision = receipt.revision;
      }
    }
    const settled = await home.finishCase(journal.caseId);
    assert.equal(settled.complete, true, JSON.stringify(settled));
    assert.deepEqual(sent.filter(row => ['stop', 'admission-release', 'temporary-home-close']
      .includes(row.operation)).map(row => [row.domainId, row.operation]),
    [['A', 'stop'], ['A', 'admission-release'], ['A', 'temporary-home-close'],
      ['B', 'stop'], ['B', 'admission-release'], ['B', 'temporary-home-close']]);
    for (const attempt of home.manifest.attempts) {
      assert.equal(JSON.parse(attempt.hStopSource.rawFrame).domainId, attempt.domainId);
      assert.equal(JSON.parse(attempt.hReleaseSource.rawFrame).domainId, attempt.domainId);
      assert.equal(attempt.closeReceipt.result.homeId, `home${attempt.domainId}`);
      assert.equal(JSON.parse(journal.operations.find(row =>
        row.request.requestId === attempt.closeRequestId).rawReceipt).requestId,
      attempt.closeRequestId);
    }
  } finally {
    assert.equal(path.dirname(dir), base);
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test('the project-only close occurs once after every owned HOME is complete', async () => {
  const product = { journal: { caseId: 'caseA' } };
  const order = [];
  const home = { manifest: { ownedDomains: ['A', 'B'],
      attempts: [{ caseId: 'caseA', domainId: 'A' }, { caseId: 'caseA', domainId: 'B' }] },
    async finishCase(caseId) { assert.equal(caseId, 'caseA'); order.push('finish');
      return { complete: true }; },
    async beforeCaptionClose() { order.push('teardown'); return { complete: true }; },
    async verifyClosedReadback() { order.push('readback'); return true; } };
  await closeProjectOnlyAfterOwnedHome(home, product, async () => { order.push('close'); });
  assert.deepEqual(order, ['finish', 'teardown', 'close', 'readback']);
  order.length = 0;
  home.finishCase = async () => ({ complete: false });
  await assert.rejects(closeProjectOnlyAfterOwnedHome(home, product,
    async () => { order.push('close'); }), /Both original project HOME attempts/);
  assert.deepEqual(order, []);
  home.manifest.attempts.pop();
  await assert.rejects(closeProjectOnlyAfterOwnedHome(home, product,
    async () => { order.push('close'); }), /Both declared project domains/);
  assert.deepEqual(order, []);
});

test('the actual closed reader retains A/B own inventory and rejects C or another case', () => {
  const python = process.env.GOGOKE_TEST_PYTHON ?? 'python';
  const reader = path.resolve(here, '..', 'installed-temporary-home-readback.py');
  const source = `import copy,importlib.util,sys\n` +
    `spec=importlib.util.spec_from_file_location('home_reader',sys.argv[1]); r=importlib.util.module_from_spec(spec); spec.loader.exec_module(r)\n` +
    `m={'caseId':'one','domainId':'A','ownedDomains':['A','B'],'cases':[{'caseId':'one'}],` +
    `'producers':[{'caseId':'one','domainId':'A','sessionId':'same'},{'caseId':'one','domainId':'B','sessionId':'same'}],` +
    `'attempts':[{'caseId':'one','domainId':'A'},{'caseId':'one','domainId':'B'}]}\n` +
    `assert r.owned_home_domains(m)==['A','B']\n` +
    `startup=copy.deepcopy(m); startup['cases'].append({'caseId':'one_startup_0','sourceKind':'NATIVE_STARTUP'}); ` +
    `startup['startupAttempts']=[{'caseId':'one_startup_0','sourceKind':'NATIVE_STARTUP'}]\n` +
    `assert r.owned_home_domains(startup)==['A','B']\n` +
    `assert r.home_inventory_differences({}, {'a':{'domain_id':'A'},'b':{'domain_id':'B'}}, ['a','b'],set())==([],[],[])\n` +
    `assert r.home_inventory_differences({}, {'a':{'domain_id':'A'},'b':{'domain_id':'B'},'c':{'domain_id':'C'}}, ['a','b'],set())[2]==['c']\n` +
    `for key,value in [('domainId','C'),('caseId','other')]:\n` +
    ` bad=copy.deepcopy(m); bad['attempts'][1][key]=value\n` +
    ` try: r.owned_home_domains(bad)\n` +
    ` except RuntimeError: pass\n` +
    ` else: raise AssertionError('undeclared domain or wrong case passed')\n`;
  const run = spawnSync(python, ['-c', source, reader], { encoding: 'utf8', windowsHide: true });
  assert.equal(run.status, 0, run.stderr || run.error?.message);
});
