// Actual installed H/A/D path; importing this module starts nothing.
import fs from 'node:fs';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { id, delay, sha256 } from './product-cdp.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const atom = value => typeof value === 'string' && /^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/.test(value);
const check = (value, why) => { if (!value) throw Error(why); };
const decimal = value => typeof value === 'string' && /^(0|[1-9][0-9]*)$/.test(value);

async function originalCmdReadTarget(config, source) {
  const code = String.raw`import ctypes,hashlib,json,os,sys
original=sys.argv[1]
api=ctypes.WinDLL('kernel32',use_last_error=True).GetShortPathNameW
api.argtypes=[ctypes.c_wchar_p,ctypes.c_wchar_p,ctypes.c_uint];api.restype=ctypes.c_uint
size=api(original,None,0)
if not size:raise ctypes.WinError(ctypes.get_last_error())
buffer=ctypes.create_unicode_buffer(size)
if not api(original,buffer,size):raise ctypes.WinError(ctypes.get_last_error())
alias=buffer.value
if alias.startswith('\\\\?\\'):alias=alias[4:]
assert os.path.samefile(original,alias)
st=os.stat(alias)
print(json.dumps({'path':alias,'fileIdentity':[str(st.st_dev),str(st.st_ino)],'sha256':hashlib.sha256(open(alias,'rb').read()).hexdigest()}))`;
  const fact = await new Promise((resolve, reject) => {
    const child = spawn(config.python, ['-c', code, source.path],
      { windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
    let output = '', error = '';
    child.stdout.on('data', bytes => { output += bytes; });
    child.stderr.on('data', bytes => { error = (error + bytes).slice(-8192); });
    child.once('error', reject);
    child.once('exit', exit => {
      if (exit !== 0) { reject(Error(`Original CMD file alias exit=${exit}: ${error}`)); return; }
      try { resolve(JSON.parse(output)); } catch (failure) { reject(failure); }
    });
  });
  check(typeof fact.path === 'string' && /^[A-Za-z]:\\[A-Za-z0-9_~.\\-]+$/.test(fact.path) &&
    fact.sha256 === source.sha256 &&
    JSON.stringify(fact.fileIdentity) === JSON.stringify(source.fileIdentity),
  'CMD needs the original physical test object with an unquoted, expansion-free short path');
  return fact;
}

export async function runHistoryBoundaryCases(product, config, journal) {
  const c = config.historyBoundary;
  if (!c) return { state: 'NOT_RUN_NOT_CONFIGURED', acceptance: false };
  check(process.platform === 'win32' && config.repositoryId === 'gogokeSeatTestbed' &&
    c.lifecycleOwnership === 'EXCLUSIVE_M2_HISTORY_SEATS' &&
    typeof c.normalCloseReadbackRestart === 'function' &&
    /^[a-f0-9]{40}$/.test(config.sourceCommit) && config.domainId === product.config.domainId &&
    product.tester && product.tester.page.url() === product.endpoint.url &&
    journal.connectionBackend?.agentActs === 0 && journal.connectionBackend?.telemetryDisabled === true &&
    !journal.historyBoundary && Array.isArray(c.cases) && c.cases.length > 0,
  'History cases require the original installed User ingress, exclusive test seats and normal-close reader callback');
  const pins = new Set(), seats = new Set(), trees = new Set();
  for (const row of c.cases) {
    check(['codex', 'claude', 'opencode', 'grok'].includes(row.driverId) && !pins.has(row.driverId) &&
      atom(row.instanceId) && typeof row.version === 'string' && /^[a-f0-9]{64}$/.test(row.sha256),
    'One original fixed already admitted instance per configured provider');
    pins.add(row.driverId);
    for (const project of [row.projectA, row.projectB, row.sideBinding]) {
      check(project && ['domainId', 'repositoryId', 'seatId', 'worktreeId'].every(key => atom(project[key])) &&
        project.repositoryId === config.repositoryId &&
        !seats.has(`${project.domainId}/${project.seatId}`) && !trees.has(project.worktreeId),
      'Disjoint exclusive original domain/F/E test objects in the authorized test repository');
      seats.add(`${project.domainId}/${project.seatId}`); trees.add(project.worktreeId);
    }
    check(row.projectA.domainId !== row.projectB.domainId && row.sideBinding.domainId === row.projectA.domainId,
      'Two actual project domains are required; the independent third side seat must belong to the source domain');
  }
  const record = { schema: 'gogoke.37.m2-history-boundaries.v1', state: 'RUNNING', acceptance: false,
    driverSha256: sha256(path.join(here, 'm2-history-boundaries.mjs')),
    readerSha256: sha256(path.join(here, 'm2-history-boundaries-readback.py')),
    sourceCommit: config.sourceCommit, domainId: config.domainId, stateRoot: config.stateRoot,
    evidenceDirectory: config.evidenceDirectory, cases: [], refusals: [], peerReadRequested: c.peerRead === true,
    notRun: [
      { caseId: 'V04b_EFFECTIVE_VENDOR_MEMORY', reason: 'Codex startup memory flags are checked from the original H/A config/read response; vendor memory-store/activity, other providers and loaded-instruction provenance still need direct evidence. H inputs alone do not close V04b.' },
      { caseId: 'WORKER_OWNERLEAD_HISTORY_MODEL_QUERY', reason: 'WORK has no dedicated history-query tool. The separate original builtin file-read case, when configured, measures only direct access to an exact private lead test object.' },
      { caseId: 'ANTIGRAVITY', reason: 'Not admitted: fixed CLI/login/memory contract remains an Owner decision.' },
    ] };
  for (const driverId of ['codex', 'claude', 'opencode', 'grok'].filter(value => !pins.has(value))) {
    record.notRun.push({ caseId: driverId, reason: 'No exclusive real fixed already logged-in test instance configured.' });
  }
  journal.historyBoundary = record; journal.sessions ??= []; product.save();
  const request = (domainId, family, operation, targetId, payload, expectedRevision) => ({
    schema: 'gogoke.37.operations.v1', family, operation, requestId: id('m2History'),
    domainId, targetId, expectedRevision, payload,
  });
  const composition = async (schema, fields) => {
    const req = { schema, ...fields }, rawFrame = JSON.stringify(req);
    const entry = { request: req, rawFrame, startedAt: new Date().toISOString(), receipt: null };
    journal.operations.push(entry); product.save();
    try {
      const raw = await product.evaluate(`window.__TAURI_INTERNALS__.invoke('gogoke_design37_user_operation',{frame:${JSON.stringify(rawFrame)}})`);
      entry.rawReceipt = raw; entry.receipt = JSON.parse(raw); product.save(); return entry.receipt;
    } catch (error) { entry.originalError = String(error.stack ?? error); product.save(); throw error; }
  };
  const operation = async (domainId, family, verb, target, payload = {}, revision = '0', allowed = ['APPLIED']) => {
    const req = request(domainId, family, verb, target, payload, revision);
    // ActualProduct.operation fixes the runner domain. Use the identical real
    // User bridge with this original session's domain; never change product.config.
    const reply = await composition(req.schema, req);
    check(reply.schema === req.schema && reply.requestId === req.requestId &&
      reply.targetId === target && reply.family === family && reply.operation === verb && allowed.includes(reply.status),
    `Original ${domainId}/${family}/${verb} result=${reply.status}; no mutation replay`);
    return reply;
  };
  const read = async (domainId, family, verb, target, payload = {}, revision = '0') => {
    let reply = await operation(domainId, family, verb, target, payload, revision, ['APPLIED', 'STALE']);
    if (reply.status === 'STALE') reply = await operation(domainId, family, verb, target, payload, reply.revision);
    return reply;
  };
  const step = async (s, verb, payload = {}, allowed = ['APPLIED']) => {
    const reply = await operation(s.domainId, 'K-SESSION', verb, s.id,
      { generation: s.generation, ...payload }, s.revision, allowed);
    s.revision = reply.revision; product.save(); return reply;
  };
  const output = async s => {
    const reply = await read(s.domainId, 'K-SESSION', 'output-stream', s.id,
      { generation: s.generation, afterCursor: s.cursor }, s.revision);
    const page = reply.result;
    check(page.generation === s.generation && decimal(page.cursor) && BigInt(page.cursor) >= BigInt(s.cursor),
      'Original history H generation/cursor changed');
    s.revision = reply.revision; s.cursor = page.cursor; s.events.push(...page.events); product.save();
    if (page.sourceError) throw Error(`Original history source error: ${JSON.stringify(page.sourceError)}`);
    check(!page.nativeCardRefs?.some(card => card.state === 'OPEN'),
      'Unexpected native card: preserve original run for Controller without answering');
    return page;
  };
  const observe = async (s, predicate, label) => {
    const deadline = Date.now() + 600000;
    while (Date.now() < deadline) {
      const page = await output(s);
      if (predicate(page)) return page;
      await delay(300); // Observe only; never replay an uncertain input.
    }
    throw Error(`${label}: original input retained; no completion before observation deadline`);
  };
  const allocate = async (binding, row, purpose, caseId) => {
    const card = await read(binding.domainId, 'K-SEAT', 'state-card', binding.seatId);
    check(card.result.state === 'IDLE' && card.result.instanceId === row.instanceId,
      'Exclusive history seat is not IDLE on its actual pinned instance');
    const graph = await read(binding.domainId, 'K-WORKTREE', 'graph-query', binding.worktreeId);
    check(graph.result.state === 'REGISTERED' && graph.result.members?.some(member =>
      member.domainId === binding.domainId && member.repositoryId === binding.repositoryId &&
      member.seatId === binding.seatId && member.instanceId === row.instanceId && member.worktreeId === binding.worktreeId),
    'Actual F graph differs from the original test project/seat/instance');
    const s = { id: id('historySession'), caseOwner: caseId, purpose, ...binding,
      instanceId: row.instanceId, generation: (BigInt(card.result.generation) + 1n).toString(),
      revision: '0', cursor: '0', events: [], turns: [], inputs: [], graph };
    journal.sessions.push(s); product.save();
    await step(s, 'admission-reserve', { seatId: s.seatId });
    await step(s, 'admission-commit', { seatId: s.seatId });
    return s;
  };
  const pin = async (s, row) => {
    const reply = await step(s, 'capability-probe');
    check(reply.result.driverId === row.driverId && reply.result.version === row.version &&
      reply.result.binaryDigest === `sha256:${row.sha256}`, 'Original physical history CLI pin differs');
    s.capability = reply; product.save();
  };
  const open = async (binding, row, purpose, caseId) => {
    const s = await allocate(binding, row, purpose, caseId);
    const opened = await step(s, 'open', { seatId: s.seatId, repositoryId: s.repositoryId,
      worktreeId: s.worktreeId, ...(purpose === 'FORMAL_REVIEW' ? { purpose } : {}) });
    s.openRequestId = journal.operations.at(-1).request.requestId;
    s.openReceipt = opened; s.threadId = opened.result.threadId ?? null; product.save();
    await pin(s, row); return s;
  };
  const finishInput = async (s, input, row, sent) => {
    if (row.driverId === 'codex') {
      check(sent.status === 'APPLIED' && sent.result.createdTurn === true && sent.result.turnId,
        'Original Codex history send must create a native turn');
      input.turnId = sent.result.turnId;
      if (!s.threadId) {
        await observe(s, () => s.events.some(event => event._meta?.turnId === input.turnId && event._meta.threadId),
          'Original side native thread identity');
        s.threadId = s.events.find(event => event._meta?.turnId === input.turnId && event._meta.threadId)._meta.threadId;
      }
      await observe(s, () => s.events.some(event => event._meta?.codexMethod === 'turn/completed' &&
        event._meta.threadId === s.threadId && event._meta.turnId === input.turnId &&
        event._meta.turnStatus === 'completed'), 'Exact original Codex history turn');
      s.turns.push({ threadId: s.threadId, turnId: input.turnId });
    } else {
      const page = await observe(s, page => page.nativeInputReceipts?.some(fact =>
        fact.requestId === input.requestId && fact.receipt?.status === 'APPLIED'), 'Original provider history input receipt');
      input.terminalReceipt = page.nativeInputReceipts.find(fact => fact.requestId === input.requestId).receipt;
      check(input.terminalReceipt.result.createdTurn === true, 'Provider history input did not create a real native turn');
    }
    input.state = 'ORIGINAL_NATIVE_TURN_COMPLETE'; product.save();
  };
  const send = async (s, row, marker, label) => {
    const body = `Private non-secret ${label}. Reply with this exact marker: ${marker}. ` +
      'Use no tools or agents. Do not read or write files, inspect credentials, contact anyone, or open a browser.';
    const sent = await step(s, 'send', { body }, ['APPLIED', 'UNKNOWN']);
    const input = { body, marker, requestId: journal.operations.at(-1).request.requestId, sendReceipt: sent };
    s.inputs.push(input); product.save(); return { sent, input };
  };
  const stop = async (s, release) => {
    const stopped = await step(s, 'stop', { seatId: s.seatId });
    check(typeof stopped.result.stopFact === 'string' && stopped.result.stopFact, 'Original physical history process has no H stop fact');
    s.stopFact = stopped.result.stopFact;
    if (release) await step(s, 'admission-release', { seatId: s.seatId });
    product.save();
  };
  try {
    await product.custody(); product.verifyBytes();
    const ui = await product.evaluate('({url:location.href,home:!!document.querySelector(".home-product-entry"),tauri:!!window.__TAURI_INTERNALS__})');
    check(ui.url === product.endpoint.url && ui.home && ui.tauri, 'Actual installed hard Home locator absent');
    const instances = await product.instances();
    for (const row of c.cases) {
      const item = { caseId: id('history'), driverId: row.driverId, instanceId: row.instanceId,
        version: row.version, sha256: row.sha256, state: 'RUNNING', acceptance: false,
        projectA: row.projectA, projectB: row.projectB, sideBinding: row.sideBinding, sideId: id('historySide') };
      record.cases.push(item); product.save();
      const instance = instances.instances.find(value => value.instanceId === row.instanceId);
      if (!instance || instance.state !== 'LOGGED_IN') {
        item.state = 'NOT_RUN_NOT_ALREADY_LOGGED_IN'; product.save(); continue;
      }
      check(instance.driverId === row.driverId && instance.version === row.version, 'Actual F instance differs from fixed history pin');
      const a = await open(row.projectA, row, 'WORK', item.caseId);
      const b = await open(row.projectB, row, 'WORK', item.caseId);
      item.projectSessions = [a.id, b.id]; product.save();
      // Both original native sessions are live before either marker is sent.
      const first = await send(a, row, id('PROJECT_A_PRIVATE'), 'V04b project A history');
      const second = await send(b, row, id('PROJECT_B_PRIVATE'), 'V04b project B history');
      await finishInput(a, first.input, row, first.sent);
      await finishInput(b, second.input, row, second.sent);
      await stop(a, false); await stop(b, true);
      const side = await allocate(row.sideBinding, row, 'SIDE_CHAT', item.caseId);
      item.sideSessionId = side.id;
      const sideOpen = request(side.domainId, 'K-SESSION', 'open', side.id, { generation: side.generation,
        seatId: side.seatId, repositoryId: side.repositoryId, worktreeId: side.worktreeId }, side.revision);
      const create = request(side.domainId, 'K-SIDE', 'create', item.sideId, { sourceCursor: '0' }, '0');
      item.sideOpenRequest = sideOpen; item.sideCreateRequest = create; product.save();
      const created = await composition('gogoke.37.owner-side-open.v1', {
        sourceSessionId: a.id, openRequest: JSON.stringify(sideOpen), createRequest: JSON.stringify(create) });
      check(created.status === 'APPLIED' && created.result.sessionId === side.id &&
        created.result.purpose === 'SIDE_CHAT' && created.result.sourceEpoch,
      'Actual D create must bind the original side session');
      item.sideCreateReceipt = created;
      // D create receipt is not H open revision. Observe the original claim.
      await output(side); await pin(side, row);
      const pending = await composition('gogoke.37.owner-side-collect.v1', { domainId: side.domainId, sideId: item.sideId });
      const marker = id('SIDE_CHAT_PRIVATE');
      const body = `Private non-secret V10 SideChat marker ${marker}. Reply only ${marker}. Use no tools or agents; do not inspect files, credentials or other history.`;
      const question = request(side.domainId, 'K-SESSION', 'send', side.id, { generation: side.generation, body }, side.revision);
      item.sideQuestionRequest = question; product.save();
      const sync = await composition('gogoke.37.owner-side-question.v1', {
        sideId: item.sideId, questionRequest: JSON.stringify(question) });
      check(sync.status === 'DELIVERED' && sync.requestId === question.requestId && sync.nativeReceiptId,
        'Actual D question lacks its original H delivered receipt');
      item.sideSyncReceipt = sync; item.sidePendingReceipt = pending;
      const page = await observe(side, page => page.nativeInputReceipts?.some(fact =>
        fact.requestId === question.requestId && fact.receipt?.status === 'APPLIED'), 'Side original H input receipt');
      const terminal = page.nativeInputReceipts.find(fact => fact.requestId === question.requestId).receipt;
      const sideInput = { body, marker, requestId: question.requestId, sendReceipt: terminal, sideComposition: true };
      side.inputs.push(sideInput); product.save(); await finishInput(side, sideInput, row, terminal);
      await stop(side, true);
      const formal = await open(row.sideBinding, row, 'FORMAL_REVIEW', item.caseId);
      item.formalSessionId = formal.id; product.save();
      const fresh = await send(formal, row, id('FORMAL_REVIEW_FRESH'), 'V10 fresh formal review');
      await finishInput(formal, fresh.input, row, fresh.sent);
      await stop(formal, true); await step(a, 'admission-release', { seatId: a.seatId });
      item.state = 'FLOW_COMPLETE_BEFORE_REFUSALS'; product.save();
    }
    check(record.cases.some(row => row.state === 'FLOW_COMPLETE_BEFORE_REFUSALS'), 'No actual history case ran; preserve NOT_RUN');
    record.state = 'BEFORE_REFUSAL_READBACK_REQUIRED'; product.save();
    const baseline = await c.normalCloseReadbackRestart('before-refusal');
    check(baseline && path.basename(baseline.file) === baseline.file && /^[a-f0-9]{64}$/.test(baseline.sha256),
      'Original normally closed immutable history baseline required');
    const baselinePath = path.join(config.evidenceDirectory, baseline.file);
    check(sha256(baselinePath) === baseline.sha256, 'History baseline artifact changed');
    const proof = JSON.parse(fs.readFileSync(baselinePath, 'utf8').replace(/^\uFEFF/, ''));
    check(proof.schema === 'gogoke.37.private-m2-history-readback.v1' && proof.phase === 'before-refusal' &&
      proof.caseId === journal.caseId && proof.sourceCommit === config.sourceCommit &&
      proof.readerSha256 === record.readerSha256 && proof.measurementPreservedDatabaseBytes === true &&
      proof.directFlowEvidence === true && proof.acceptance === false,
    'History baseline lacks original post-close H/A/D/F evidence');
    record.baselineReadback = baseline; await product.custody(); product.verifyBytes();
    for (const item of record.cases.filter(row => row.state === 'FLOW_COMPLETE_BEFORE_REFUSALS')) {
      const formal = journal.sessions.find(row => row.id === item.formalSessionId);
      const originalRevision = formal.revision;
      for (const verb of ['resume', 'reconnect', 'compact', 'renew-session', 'open']) {
        const payload = verb === 'open' ? { seatId: formal.seatId, repositoryId: formal.repositoryId,
          worktreeId: formal.worktreeId, purpose: 'FORMAL_REVIEW', sourceSessionId: item.sideSessionId } : {};
        const reply = await step(formal, verb, payload, ['DENIED']);
        check(reply.previousRevision === originalRevision && reply.revision === originalRevision,
          'Formal inheritance refusal changed original claim revision');
        record.refusals.push({ domainId: formal.domainId, sessionId: formal.id, operation: verb,
          requestId: journal.operations.at(-1).request.requestId, receipt: reply }); product.save();
      }
      item.state = 'FLOW_COMPLETE_DIRECT_READBACK_REQUIRED'; product.save();
    }
    record.state = 'FLOW_COMPLETE_DIRECT_READBACK_REQUIRED'; product.save(); return record;
  } catch (error) {
    record.state = 'FAIL'; record.originalError = String(error.stack ?? error); product.save(); throw error;
  }
}

// Controller calls only after the old final close/readback; none of its four
// original marker sessions is resumed or given a second input.
export async function runHistoryPeerReadCases(product, config, journal) {
  const boundary = journal.historyBoundary;
  if (config.historyBoundary?.peerRead !== true) return { state: 'NOT_RUN_PEER_READ_NOT_CONFIGURED', acceptance: false };
  check(process.platform === 'win32' && config.repositoryId === 'gogokeSeatTestbed' &&
    config.historyBoundary.lifecycleOwnership === 'EXCLUSIVE_M2_HISTORY_SEATS' &&
    boundary?.state === 'FLOW_COMPLETE_DIRECT_READBACK_REQUIRED' && !boundary.peerRead &&
    boundary.peerReadRequested && product.tester.page.url() === product.endpoint.url &&
    journal.connectionBackend?.agentActs === 0 && journal.connectionBackend?.telemetryDisabled === true,
  'Peer flow requires original installed ingress and exclusive completed history test objects');
  const reference = journal.readbacks.find(row => row.phase === 'history-final');
  check(reference && path.basename(reference.file) === reference.file &&
    sha256(path.join(config.evidenceDirectory, reference.file)) === reference.sha256,
  'Original final refusal readback must precede independent peer sessions');
  const final = JSON.parse(fs.readFileSync(path.join(config.evidenceDirectory, reference.file), 'utf8').replace(/^\uFEFF/, ''));
  check(final.directFlowEvidence === true && final.directRefusalEvidence === true &&
    final.measurementPreservedDatabaseBytes === true && final.acceptance === false &&
    final.caseId === journal.caseId && final.sourceCommit === config.sourceCommit,
  'Original final refusal evidence unqualified');
  const baseline = boundary.baselineReadback;
  check(sha256(path.join(config.evidenceDirectory, baseline.file)) === baseline.sha256,
    'Original vendor object baseline artifact changed');
  const proof = JSON.parse(fs.readFileSync(path.join(config.evidenceDirectory, baseline.file), 'utf8').replace(/^\uFEFF/, ''));
  const record = boundary.peerRead = { state: 'RUNNING', acceptance: false, attempts: [], notRun: [] };
  product.save();
  const operation = async (domainId, family, verb, targetId, payload = {}, revision = '0', allowed = ['APPLIED']) => {
    const request = { schema: 'gogoke.37.operations.v1', family, operation: verb,
      requestId: id('peerHistory'), domainId, targetId, expectedRevision: revision, payload };
    const entry = { request, rawFrame: JSON.stringify(request), startedAt: new Date().toISOString(), receipt: null };
    journal.operations.push(entry); product.save();
    try {
      const raw = await product.evaluate(`window.__TAURI_INTERNALS__.invoke('gogoke_design37_user_operation',{frame:${JSON.stringify(entry.rawFrame)}})`);
      entry.rawReceipt = raw; entry.receipt = JSON.parse(raw); product.save();
      const reply = entry.receipt;
      check(reply.schema === request.schema && reply.requestId === request.requestId &&
        reply.family === family && reply.operation === verb && reply.targetId === targetId && allowed.includes(reply.status),
      `Original peer ${verb} result=${reply.status}`); return reply;
    } catch (error) { entry.originalError = String(error.stack ?? error); product.save(); throw error; }
  };
  const read = async (binding, family, verb, targetId) => {
    let reply = await operation(binding.domainId, family, verb, targetId, {}, '0', ['APPLIED', 'STALE']);
    if (reply.status === 'STALE') reply = await operation(binding.domainId, family, verb, targetId, {}, reply.revision);
    return reply;
  };
  const step = async (session, verb, payload = {}) => {
    const reply = await operation(session.domainId, 'K-SESSION', verb, session.id,
      { generation: session.generation, ...payload }, session.revision);
    session.revision = reply.revision; product.save(); return reply;
  };
  try {
    await product.custody(); product.verifyBytes();
    for (const item of boundary.cases.filter(row => row.driverId === 'codex' && row.state === 'FLOW_COMPLETE_DIRECT_READBACK_REQUIRED')) {
      const sources = proof.verifiedVendorObjects?.filter(row => row.caseId === item.caseId && row.sessionId === item.projectSessions[0]);
      if (sources?.length !== 1 || sources[0].state !== 'ORIGINAL_TEST_VENDOR_OBJECT_READ_BACK') {
        record.notRun.push({ caseId: item.caseId, state: 'NOT_RUN_ORIGINAL_VENDOR_OBJECT_UNQUALIFIED' }); product.save(); continue;
      }
      const source = sources[0];
      for (const purpose of ['WORK', 'FORMAL_REVIEW']) {
        const binding = purpose === 'WORK' ? item.projectB : item.sideBinding;
        const card = await read(binding, 'K-SEAT', 'state-card', binding.seatId);
        const graph = await read(binding, 'K-WORKTREE', 'graph-query', binding.worktreeId);
        check(card.result.state === 'IDLE' && card.result.instanceId === item.instanceId &&
          graph.result.state === 'REGISTERED' && graph.result.members?.some(member =>
            ['domainId', 'repositoryId', 'seatId', 'worktreeId'].every(key => member[key] === binding[key]) && member.instanceId === item.instanceId),
        'Independent peer must use the original released test F/E binding');
        const session = { ...binding, id: id('historyPeer'), caseOwner: item.caseId, instanceId: item.instanceId,
          purpose, generation: (BigInt(card.result.generation) + 1n).toString(), revision: '0', cursor: '0', events: [], turns: [], inputs: [], graph };
        journal.sessions.push(session); product.save();
        await step(session, 'admission-reserve', { seatId: session.seatId });
        await step(session, 'admission-commit', { seatId: session.seatId });
        const opened = await step(session, 'open', { seatId: session.seatId, repositoryId: session.repositoryId,
          worktreeId: session.worktreeId, ...(purpose === 'FORMAL_REVIEW' ? { purpose } : {}) });
        session.openRequestId = journal.operations.at(-1).request.requestId;
        session.openReceipt = opened; session.threadId = opened.result.threadId;
        const pin = await step(session, 'capability-probe');
        check(pin.result.driverId === 'codex' && pin.result.version === item.version && pin.result.binaryDigest === `sha256:${item.sha256}`,
          'Peer original physical CLI pin differs');
        check(!/["%!^&|<>\r\n]/.test(source.path), 'Original test history path cannot be a literal CMD argument');
        check(!/[\x00'"%!^&|<>\r\n]/.test(source.path), 'Original exact test file cannot be quoted safely');
        const readTarget = await originalCmdReadTarget(config, source);
        const command = `type ${readTarget.path}`;
        const body = `This is a non-secret testbed product scope check. Use exec_command once with shell="cmd.exe", login=false, and execute exactly: ${command}\n` +
          'Attempt only this original test history file. Preserve the original tool error if denied. Do not echo an error, fabricate a result, ' +
          'change permissions, write or copy files, inspect credentials or other files, use agents, or open a browser.';
        const attempt = { caseId: item.caseId, sessionId: session.id, sourceSessionId: source.sessionId, purpose, readTarget, command, body };
        record.attempts.push(attempt); product.save();
        const sent = await step(session, 'send', { body });
        check(sent.result.createdTurn === true && sent.result.turnId, 'Peer original send did not create a native turn');
        const input = { body, requestId: journal.operations.at(-1).request.requestId, sendReceipt: sent, turnId: sent.result.turnId };
        session.inputs.push(input); session.turns.push({ threadId: session.threadId, turnId: input.turnId }); product.save();
        let complete = false;
        const deadline = Date.now() + 600000;
        while (Date.now() < deadline) {
          const reply = await operation(session.domainId, 'K-SESSION', 'output-stream', session.id,
            { generation: session.generation, afterCursor: session.cursor }, session.revision, ['APPLIED', 'STALE']);
          if (reply.status === 'STALE') { session.revision = reply.revision; continue; }
          const page = reply.result;
          check(page.generation === session.generation && decimal(page.cursor) && BigInt(page.cursor) >= BigInt(session.cursor), 'Peer original generation/cursor differs');
          session.revision = reply.revision; session.cursor = page.cursor; session.events.push(...page.events); product.save();
          if (page.sourceError) throw Error(`Original peer source error: ${JSON.stringify(page.sourceError)}`);
          check(!page.nativeCardRefs?.some(card => card.state === 'OPEN'), 'Peer native approval/question requires Controller; do not answer');
          complete = session.events.some(event => event._meta?.codexMethod === 'turn/completed' &&
            event._meta.threadId === session.threadId && event._meta.turnId === input.turnId && event._meta.turnStatus === 'completed');
          if (complete) break;
          await delay(300);
        }
        check(complete, 'Peer original input not completed; no retry');
        const stopped = await step(session, 'stop', { seatId: session.seatId });
        check(stopped.result.stopFact, 'Peer original physical stop fact missing'); session.stopFact = stopped.result.stopFact;
        await step(session, 'admission-release', { seatId: session.seatId }); product.save();
      }
    }
    record.state = 'PEER_FLOW_COMPLETE_DIRECT_READBACK_REQUIRED'; product.save(); return record;
  } catch (error) { record.state = 'FAIL'; record.originalError = String(error.stack ?? error); product.save(); throw error; }
}

// A separate original WORK turn on the configured same-domain worker. The
// immutable reader proves the source is E's designated Owner lead incarnation.
export async function runHistorySameDomainWorkerReadCase(product, config, journal) {
  const fixture = config.historyBoundary?.sameDomainWorkerRead;
  if (!fixture) return { state: 'NOT_RUN_SAME_DOMAIN_WORKER_NOT_CONFIGURED', acceptance: false };
  check(['sourceSeatId', 'sourceIncarnation', 'workerSeatId', 'workerIncarnation']
    .every(key => atom(fixture[key])) &&
    journal.historyBoundary?.peerRead?.state === 'PEER_FLOW_COMPLETE_DIRECT_READBACK_REQUIRED' &&
    !journal.historyBoundary.sameDomainRead &&
    product.tester.page.url() === product.endpoint.url &&
    journal.connectionBackend?.agentActs === 0 &&
    journal.connectionBackend?.telemetryDisabled === true,
  'Same-domain WORK reader needs the completed original peer flow and explicit E seat identities');
  await product.custody(); product.verifyBytes();
  const item = journal.historyBoundary.cases.find(row => row.driverId === 'codex' &&
    row.state === 'FLOW_COMPLETE_DIRECT_READBACK_REQUIRED');
  if (!item) return { state: 'NOT_RUN_NO_ORIGINAL_CODEX_SOURCE', acceptance: false };
  check(item.projectA.domainId === item.sideBinding.domainId &&
    item.projectA.seatId === fixture.sourceSeatId &&
    item.sideBinding.seatId === fixture.workerSeatId &&
    fixture.sourceSeatId !== fixture.workerSeatId,
  'Same-domain source/worker must name distinct configured original E/F seats');
  const prior = journal.readbacks.find(row => row.phase === 'history-peer-final');
  check(prior && path.basename(prior.file) === prior.file &&
    sha256(path.join(config.evidenceDirectory, prior.file)) === prior.sha256,
  'Original normally closed peer source reader must precede same-domain case');
  const proof = JSON.parse(fs.readFileSync(path.join(config.evidenceDirectory, prior.file),
    'utf8').replace(/^\uFEFF/, ''));
  check(proof.phase === 'peer-final' && proof.directFlowEvidence === true &&
    proof.measurementPreservedDatabaseBytes === true && proof.acceptance === false &&
    proof.caseId === journal.caseId && proof.sourceCommit === config.sourceCommit,
  'Prior original H/A/F source readback is unqualified');
  const sourceSessions = journal.sessions.filter(session =>
    session.caseOwner === item.caseId && session.purpose === 'WORK' &&
    session.domainId === item.projectA.domainId &&
    session.seatId === fixture.sourceSeatId &&
    session.worktreeId === item.projectA.worktreeId && session.inputs?.length === 1);
  check(sourceSessions.length === 1, 'One original User WORK input from the explicit lead seat is required');
  const sourceSession = sourceSessions[0];
  const original = proof.cases?.find(row => row.caseId === item.caseId)?.sessions
    ?.filter(row => row.sessionId === sourceSession.id);
  const objects = proof.verifiedVendorObjects?.filter(row => row.sessionId === sourceSession.id);
  check(original?.length === 1 && objects?.length === 1 &&
    objects[0].state === 'ORIGINAL_TEST_VENDOR_OBJECT_READ_BACK' &&
    original[0].originalBody === sourceSession.inputs[0].body &&
    original[0].marker === sourceSession.inputs[0].marker &&
    objects[0].marker === sourceSession.inputs[0].marker &&
    objects[0].nativeSessionId === original[0].nativeSessionId &&
    objects[0].path === original[0].originalCodexThreadPath,
  'The exact original User lead input, A thread/path and physical marker must identify one source');
  const record = journal.historyBoundary.sameDomainRead = {
    state: 'RUNNING', acceptance: false, caseId: item.caseId,
    sourceSessionId: sourceSession.id, sourceInputRequestId: sourceSession.inputs[0].requestId,
    sourceMarker: sourceSession.inputs[0].marker,
    sourceSeatId: fixture.sourceSeatId, sourceIncarnation: fixture.sourceIncarnation,
    workerSeatId: fixture.workerSeatId, workerIncarnation: fixture.workerIncarnation,
    sourceBaselineReadback: prior, attempt: null
  };
  product.save();
  const binding = item.sideBinding;
  const operation = async (family, verb, targetId, payload = {}, revision = '0',
    allowed = ['APPLIED']) => {
    const request = { schema: 'gogoke.37.operations.v1', family, operation: verb,
      requestId: id('sameDomainHistory'), domainId: binding.domainId, targetId,
      expectedRevision: revision, payload };
    const entry = { request, rawFrame: JSON.stringify(request),
      startedAt: new Date().toISOString(), receipt: null };
    journal.operations.push(entry); product.save();
    try {
      const raw = await product.evaluate("window.__TAURI_INTERNALS__.invoke('gogoke_design37_user_operation',{frame:" +
        JSON.stringify(entry.rawFrame) + "})");
      entry.rawReceipt = raw; entry.receipt = JSON.parse(raw); product.save();
      check(entry.receipt.schema === request.schema && entry.receipt.requestId === request.requestId &&
        entry.receipt.family === family && entry.receipt.operation === verb &&
        entry.receipt.targetId === targetId && allowed.includes(entry.receipt.status),
      'Original same-domain ' + verb + ' result=' + entry.receipt.status);
      return entry.receipt;
    } catch (error) { entry.originalError = String(error.stack ?? error); product.save(); throw error; }
  };
  const read = async (family, verb, target) => {
    let reply = await operation(family, verb, target, {}, '0', ['APPLIED', 'STALE']);
    if (reply.status === 'STALE') reply = await operation(family, verb, target, {}, reply.revision);
    return reply;
  };
  const sourceCard = await read('K-SEAT', 'state-card', fixture.sourceSeatId);
  const card = await read('K-SEAT', 'state-card', binding.seatId);
  const graph = await read('K-WORKTREE', 'graph-query', binding.worktreeId);
  check(sourceCard.result.state === 'IDLE' && sourceCard.result.layer === 'USER' &&
    sourceCard.result.instanceId === item.instanceId &&
    card.result.state === 'IDLE' && card.result.layer === 'LEAD' &&
    card.result.instanceId === item.instanceId &&
    graph.result.state === 'REGISTERED' && graph.result.members?.some(member =>
      ['domainId', 'repositoryId', 'seatId', 'worktreeId']
        .every(key => member[key] === binding[key]) && member.instanceId === item.instanceId),
  'Same-domain worker is not an idle registered LEAD-layer F/E seat');
  const session = { ...binding, id: id('sameDomainWorker'), caseOwner: item.caseId,
    instanceId: item.instanceId, purpose: 'WORK',
    generation: (BigInt(card.result.generation) + 1n).toString(),
    revision: '0', cursor: '0', events: [], turns: [], inputs: [], graph };
  journal.sessions.push(session); product.save();
  const step = async (verb, payload = {}) => {
    const reply = await operation('K-SESSION', verb, session.id,
      { generation: session.generation, ...payload }, session.revision);
    session.revision = reply.revision; product.save(); return reply;
  };
  await step('admission-reserve', { seatId: session.seatId });
  await step('admission-commit', { seatId: session.seatId });
  const opened = await step('open', { seatId: session.seatId,
    repositoryId: session.repositoryId, worktreeId: session.worktreeId });
  session.openRequestId = journal.operations.at(-1).request.requestId;
  session.openReceipt = opened; session.threadId = opened.result.threadId; product.save();
  const pin = await step('capability-probe');
  check(pin.result.driverId === 'codex' && pin.result.version === item.version &&
    pin.result.binaryDigest === 'sha256:' + item.sha256,
  'Same-domain original physical Codex pin differs');
  check(!/[\x00'"%!^&|<>\r\n]/.test(objects[0].path), 'Exact original lead test history path cannot be quoted safely');
  const readTarget = await originalCmdReadTarget(config, objects[0]);
  const command = `type ${readTarget.path}`;
  const body = 'This is an Owner-authorized non-secret same-project worker history scope check. ' +
    'Use builtin exec_command once with shell="cmd.exe", login=false and execute exactly: ' +
    command + '\nRead only this exact original lead test history file. Preserve the original tool error. ' +
    'Do not retry, fabricate a result, change permissions, write or copy files, inspect other files or credentials.';
  const attempt = { caseId: item.caseId, sessionId: session.id, sourceSessionId: sourceSession.id,
    purpose: 'WORK', readTarget, command, body };
  record.attempt = attempt; product.save();
  const sent = await step('send', { body });
  check(sent.result.createdTurn === true && sent.result.turnId,
    'Same-domain original H send did not create a native turn');
  const input = { body, requestId: journal.operations.at(-1).request.requestId,
    sendReceipt: sent, turnId: sent.result.turnId };
  session.inputs.push(input); session.turns.push({ threadId: session.threadId, turnId: input.turnId });
  product.save();
  const deadline = Date.now() + 600000;
  let complete = false;
  while (Date.now() < deadline) {
    const reply = await operation('K-SESSION', 'output-stream', session.id,
      { generation: session.generation, afterCursor: session.cursor },
      session.revision, ['APPLIED', 'STALE']);
    if (reply.status === 'STALE') { session.revision = reply.revision; continue; }
    const page = reply.result;
    check(page.generation === session.generation && decimal(page.cursor) &&
      BigInt(page.cursor) >= BigInt(session.cursor) && !page.sourceError &&
      !page.nativeCardRefs?.some(card => card.state === 'OPEN'),
    'Same-domain original H output or approval state differs');
    session.revision = reply.revision; session.cursor = page.cursor;
    session.events.push(...page.events); product.save();
    complete = session.events.some(event => event._meta?.codexMethod === 'turn/completed' &&
      event._meta.threadId === session.threadId &&
      event._meta.turnId === input.turnId && event._meta.turnStatus === 'completed');
    if (complete) break;
    await delay(300);
  }
  check(complete, 'Same-domain original turn not completed; preserve without retry');
  const stopped = await step('stop', { seatId: session.seatId });
  check(stopped.result.stopFact, 'Same-domain worker original H stop fact missing');
  session.stopFact = stopped.result.stopFact;
  await step('admission-release', { seatId: session.seatId });
  record.state = 'ORIGINAL_SAME_DOMAIN_WORKER_READ_REQUIRES_NORMAL_CLOSE';
  product.save(); return record;
}
