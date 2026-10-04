// Actual installed H/A/D path; importing this module starts nothing.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { id, delay, sha256 } from './product-cdp.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const atom = value => typeof value === 'string' && /^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/.test(value);
const check = (value, why) => { if (!value) throw Error(why); };
const decimal = value => typeof value === 'string' && /^(0|[1-9][0-9]*)$/.test(value);

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
    evidenceDirectory: config.evidenceDirectory, cases: [], refusals: [],
    notRun: [
      { caseId: 'V04b_EFFECTIVE_VENDOR_MEMORY', reason: 'Codex startup memory flags are checked from the original H/A config/read response; vendor memory-store/activity, other providers and loaded-instruction provenance still need direct evidence. H inputs alone do not close V04b.' },
      { caseId: 'WORKER_OWNERLEAD_HISTORY', reason: 'No model history-query tool or production locator for an exact non-secret OwnerLead test-history object. User reader controls are not model scope evidence.' },
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
