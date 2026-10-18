import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { createTemporaryHomeLifecycle } from '../installed-temporary-home-lifecycle.mjs';
import { closeProjectOnlyAfterOwnedHome, validateControlledInstructions, CONTROLLED_CLAUDE_PROMPT } from '../m2-history-boundaries.mjs';
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

test('the shared history reader preserves only the four observed unresolved Claude frame shapes', () => {
  const python = process.env.GOGOKE_TEST_PYTHON ?? 'python';
  const reader = path.resolve(here, '..', 'm2-history-boundaries-readback.py');
  const source = `import importlib.util,sys\n` +
    `spec=importlib.util.spec_from_file_location('history_reader',sys.argv[1]); r=importlib.util.module_from_spec(spec); spec.loader.exec_module(r)\n` +
    `sid='native-session'\n` +
    `frames=[\n` +
    ` {'type':'system','subtype':'init','session_id':sid},\n` +
    ` {'type':'rate_limit_event','session_id':sid,'rate_limit_info':{}},\n` +
    ` {'type':'system','subtype':'thinking_tokens','session_id':sid},\n` +
    ` {'type':'assistant','session_id':sid,'request_id':'req_retained_native','parent_tool_use_id':None,'message':{'role':'assistant','content':[{'type':'thinking','thinking':'...'}]}},\n` +
    `]\n` +
    `expected=['system/init','rate_limit_event','system/thinking_tokens','assistant/thinking-only']\n` +
    `assert [r.pending_unhandled_claude_frame(frame,sid)[0] for frame in frames]==expected\n` +
    `assert r.PENDING_CLASSIFICATION=='PRESERVED_UNRESOLVED_RAW_SOURCE_NO_SUCCESS_CREDIT'\n` +
    `def reject(frame,session=sid):\n` +
    ` try: r.pending_unhandled_claude_frame(frame,session)\n` +
    ` except RuntimeError: return\n` +
    ` raise AssertionError('unsupported Claude PENDING frame was accepted')\n` +
    `for frame in [\n` +
    ` {'type':'system','subtype':'init','session_id':'other'},\n` +
    ` {'type':'system','subtype':'init','session_id':sid,'method':'control'},\n` +
    ` {'type':'result','subtype':'success','session_id':sid},\n` +
    ` {'type':'user','session_id':sid},\n` +
    ` {'type':'assistant','session_id':sid,'message':{'role':'assistant','content':[{'type':'text','text':'marker'}]}},\n` +
    ` {'type':'assistant','session_id':sid,'message':{'role':'assistant','content':[{'type':'thinking','thinking':'...'}, {'type':'tool_use','id':'tool'}]}},\n` +
    ` {'type':'assistant','session_id':sid,'message':{'role':'assistant','content':[{'type':'thinking','thinking':'...','tool_use':{'id':'tool'}}]}},\n` +
    ` {'type':'rate_limit_event','rate_limit_info':{},'request_id':'rpc'},\n` +
    ` {'type':'assistant','session_id':sid,'request_id':None,'message':{'role':'assistant','content':[{'type':'thinking','thinking':'...'}]}},\n` +
    `]: reject(frame)\n` +
    `assert r.pending_unhandled_codex_frame({'method':'warning','params':{}},'thread-1')==( 'warning',None)\n`;
  const run = spawnSync(python, ['-c', source, reader], { encoding: 'utf8', windowsHide: true });
  assert.equal(run.status, 0, run.stderr || run.error?.message);
});

test('controlled Claude project instructions bind two distinct tags to their original F fixtures', () => {
  const project = (domainId, seatId, worktreeId, worktreePath, worktreePhysicalIdentity) => ({
    domainId, repositoryId: 'gogokeSeatTestbed', seatId, worktreeId });
  const projectA = project('A', 'seatA', 'treeA', 'D:\\F-A', 'volume:1/file:a');
  const projectB = project('B', 'seatB', 'treeB', 'D:\\F-B', 'volume:1/file:b');
  const fixture = (value, tag) => ({ relativePath: 'CLAUDE.md', path: `${value.path}\\CLAUDE.md`,
    worktreePath: value.path, worktreePhysicalIdentity: value.identity,
    filePhysicalIdentity: `volume:1/file:${tag.toLowerCase()}`, fileSha256: 'a'.repeat(64), expectedTag: tag,
    originalFSource: { domainId: value.domain, repositoryId: 'gogokeSeatTestbed',
      seatId: value.seat, worktreeId: value.tree, worktreePath: value.path,
      worktreePhysicalIdentity: value.identity } });
  const sources = {
    A: fixture({ domain: 'A', seat: 'seatA', tree: 'treeA', path: 'D:\\F-A', identity: 'volume:1/file:a' }, 'TAG_AAAAAAAAAAAAAAAAAAAA'),
    B: fixture({ domain: 'B', seat: 'seatB', tree: 'treeB', path: 'D:\\F-B', identity: 'volume:1/file:b' }, 'TAG_BBBBBBBBBBBBBBBBBBBB') };
  const row = { driverId: 'claude', version: '2.1.196', projectA, projectB,
    controlledInstructions: { kind: 'CLAUDE_MD_AB', prompt: CONTROLLED_CLAUDE_PROMPT, fixtures: sources } };
  assert.equal(validateControlledInstructions(row, true), row.controlledInstructions);
  assert.throws(() => validateControlledInstructions({ ...row, controlledInstructions: {
    ...row.controlledInstructions, fixtures: { ...sources, B: { ...sources.B, expectedTag: sources.A.expectedTag } } } }, true),
  /independent/);
  assert.throws(() => validateControlledInstructions({ ...row, controlledInstructions: {
    ...row.controlledInstructions, prompt: `${CONTROLLED_CLAUDE_PROMPT} ${sources.A.expectedTag}` } }, true), /shared tag-free/);
  assert.throws(() => validateControlledInstructions({ ...row, controlledInstructions: {
    ...row.controlledInstructions, fixtures: { ...sources, A: { ...sources.A,
      originalFSource: { ...sources.A.originalFSource, worktreeId: 'treeB' } } } } }, true), /exact raw F/);
  assert.throws(() => validateControlledInstructions(row, false), /project-only/);
});

test('controlled Claude readback binds assistant and exact H-receipted result while rejecting substitutions', () => {
  const python = process.env.GOGOKE_TEST_PYTHON ?? 'python';
  const reader = path.resolve(here, '..', 'm2-history-boundaries-readback.py');
  const source = `import copy,importlib.util,json,sys\n` +
    `spec=importlib.util.spec_from_file_location('history_reader',sys.argv[1]); r=importlib.util.module_from_spec(spec); spec.loader.exec_module(r)\n` +
    `tag='TAG_AAAAAAAAAAAAAAAAAAAA'; peer='TAG_BBBBBBBBBBBBBBBBBBBB'; prompt="What is the project instruction tag already supplied by this project's startup instructions?\\nReply with only that tag. If no tag was supplied, reply MISSING.\\nDo not use tools or agents, and do not read or write files to discover it."\n` +
    `tree={'worktree_path':r'D:\\\\F-A'}\n` +
    `fixture={'expectedTag':tag}\n` +
    `def frame(value):\n return {'raw_bytes':json.dumps(value).encode()},value\n` +
    `def valid(answer=tag,cwd=tree['worktree_path'],include_init=True):\n` +
    ` frames=[]\n if include_init: frames.append(frame({'type':'system','subtype':'init','session_id':'native','claude_code_version':'2.1.196','cwd':cwd}))\n` +
    ` frames.append(frame({'type':'assistant','session_id':'native','message':{'content':[{'type':'text','text':answer}]}}))\n` +
    ` result={'type':'result','subtype':'success','is_error':False,'session_id':'native','result':answer}\n` +
    ` terminal_row={'source_epoch':'epoch1','source_cursor':'6','raw_bytes':json.dumps(result).encode()}\n` +
    ` frames.append((terminal_row,result)); sent={'result':{'sourceEpoch':'epoch1','sourceCursor':'6','rawResultSha256':__import__('hashlib').sha256(terminal_row['raw_bytes']).hexdigest(),'resultSubtype':'success','vendorSessionId':'native'}}\n` +
    ` commands=[({'command_hex':json.dumps({'input':prompt}).encode().hex()}, {'input':prompt})]\n` +
    ` return frames,commands,(terminal_row,result),sent\n` +
    `def accepts(frames,commands,terminal,sent,body=prompt): return r.verify_controlled_instruction_frames(frames,commands,body,'native',tree,fixture,peer,prompt,terminal,sent)\n` +
    `assert accepts(*valid())['state']=='CONTROLLED_PROJECT_INSTRUCTION_CONTENT_OBSERVED'\n` +
    `def reject(frames,commands,terminal,sent,body=prompt):\n` +
    ` try: accepts(frames,commands,terminal,sent,body)\n except RuntimeError: return\n raise AssertionError('invalid controlled input evidence passed')\n` +
    `frames,commands,terminal,sent=valid(); commands[0]=({'command_hex':(prompt+tag).encode().hex()},{}); reject(frames,commands,terminal,sent)\n` +
    `reject(*valid(answer=peer))\n` +
    `reject(*valid(include_init=False))\n` +
    `reject(*valid(cwd=r'D:\\\\F-B'))\n` +
    `reject(*valid(),body=prompt+tag)\n` +
    `frames,commands,terminal,sent=valid(); terminal[0]['source_cursor']='7'; reject(frames,commands,terminal,sent)\n` +
    `frames,commands,terminal,sent=valid(); sent['result']['rawResultSha256']='0'*64; reject(frames,commands,terminal,sent)\n` +
    `frames,commands,terminal,sent=valid(); terminal[1]['session_id']='other'; reject(frames,commands,terminal,sent)\n` +
    `frames,commands,terminal,sent=valid(); terminal[1]['result']=peer; terminal[0]['raw_bytes']=json.dumps(terminal[1]).encode(); sent['result']['rawResultSha256']=__import__('hashlib').sha256(terminal[0]['raw_bytes']).hexdigest(); reject(frames,commands,terminal,sent)\n` +
    `case={'driverId':'claude','version':'2.1.196'}; controlled={'kind':'CLAUDE_MD_AB','prompt':prompt,'fixtures':{'A':{'expectedTag':tag},'B':{'expectedTag':peer}}}\n` +
    `assert r.verify_controlled_operation_tag_absence(controlled,case,[{'rawFrame':'{"operation":"stop"}'}])==controlled['fixtures']\n` +
    `for bad in [dict(controlled,prompt=prompt+' extra'),dict(controlled,fixtures={'A':{'expectedTag':''},'B':{'expectedTag':peer}})]:\n` +
    ` try: r.validate_controlled_instructions(bad,case)\n except RuntimeError: pass\n else: raise AssertionError('invalid controlled manifest passed')\n` +
    `try: r.verify_controlled_operation_tag_absence(controlled,case,[{'rawFrame':'{"body":"'+tag+'"}'}])\n` +
    `except RuntimeError: pass\nelse: raise AssertionError('tag in User operation rawFrame passed')\n` +
    `tree_source={'domain_id':'A','repository_id':'repo','seat_id':'seatA','worktree_id':'treeA',` +
    `'worktree_path':r'D:\\\\F-A','worktree_identity':'tree-physical'}\n` +
    `wrong_fixture={'originalFSource':{'domainId':'A','repositoryId':'repo','seatId':'seatA','worktreeId':'treeB',` +
    `'worktreePath':r'D:\\\\F-A','worktreePhysicalIdentity':'tree-physical'},'worktreePath':r'D:\\\\F-A',` +
    `'worktreePhysicalIdentity':'tree-physical','relativePath':'CLAUDE.md','path':r'D:\\\\F-A\\CLAUDE.md'}\n` +
    `try: r.verify_controlled_instruction_fixture(tree_source,wrong_fixture)\n` +
    `except RuntimeError: pass\nelse: raise AssertionError('wrong raw F fixture passed')\n`;
  const run = spawnSync(python, ['-c', source, reader], { encoding: 'utf8', windowsHide: true });
  assert.equal(run.status, 0, run.stderr || run.error?.message);
});
