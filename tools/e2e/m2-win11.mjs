// Real installed M2 path. Development driver only: never a substitute host,
// model caller, login helper, or acceptance decision.
import fs from 'node:fs';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { ActualProduct, readJson, sha256, id, delay } from './product-cdp.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const config = readJson(process.argv[2]);
const atom = value => typeof value === 'string' && /^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/.test(value);
const inside = (child, parent) => child === parent || child.startsWith(parent + path.sep);
const required = ['installed', 'version', 'sourceCommit', 'installedSha256', 'registryKey',
  'pwsh', 'python', 'evidenceDirectory', 'result', 'stateRoot', 'testbedSource',
  'domainId', 'seatId', 'instanceId', 'repositoryId', 'worktreeId', 'templateId',
  'childSeatId', 'childInstanceId', 'takeoverQuestionId', 'takeoverPrompt',
  'takeoverOption', 'observers'];
if (process.platform !== 'win32' || !process.argv[2] ||
    config.testerArmy === false ||
    required.some(name => config[name] === undefined) ||
    !atom(config.instanceId) || config.repositoryId !== 'gogokeSeatTestbed' ||
    !atom(config.domainId) || !atom(config.seatId) || !atom(config.childSeatId) ||
    !atom(config.templateId) || !atom(config.childInstanceId) ||
    !atom(config.takeoverQuestionId) || config.seatId === config.childSeatId ||
    !/^[a-f0-9]{40}$/.test(config.sourceCommit) ||
    !Array.isArray(config.providerCases) || config.providerCases.length > 3 ||
    new Set(config.providerCases.map(row => row.driverId)).size !== config.providerCases.length ||
    config.providerCases.some(row => !['claude', 'opencode', 'grok'].includes(row.driverId) ||
      ![row.instanceId, row.seatId, row.worktreeId].every(atom) ||
      typeof row.version !== 'string' || !/^[a-f0-9]{64}$/.test(row.sha256)) ||
    !Array.isArray(config.observers) ||
    !['formal', 'memory', 'ledger'].every(name => config.observers.some(row => row.name === name)) ||
    fs.existsSync(config.result) ||
    !fs.existsSync(config.evidenceDirectory) ||
    !inside(path.resolve(config.result), path.resolve(config.evidenceDirectory)) ||
    [config.installed, config.stateRoot, config.testbedSource, path.resolve(here, '..', '..')]
      .some(root => inside(path.resolve(config.evidenceDirectory), path.resolve(root))) ||
    path.resolve(config.stateRoot) === path.resolve(config.testbedSource)) {
  throw Error('Fresh private M2 case with exact installed candidate, testbed, known seats and observers required');
}
if (config.providerCases.some(row => row.driverId === 'antigravity')) throw Error('Antigravity is not admitted');
if (!config.observers.find(row => row.name === 'formal').equalFields?.includes('formal') ||
    !['formal', 'registeredFormal', 'formalData', 'formalRegistry', 'shortcuts']
      .every(field => config.observers.find(row => row.name === 'formal').equalFields.includes(field))) {
  throw Error('Formal protection observer must compare all five original fields');
}
const journal = { schema: 'gogoke.37.m2-win11-e2e.v1', caseId: id('m2'),
  sourceCommit: config.sourceCommit, domainId: config.domainId, repositoryId: config.repositoryId,
  testbedSource: config.testbedSource,
  leadSeatId: config.seatId, childSeatId: config.childSeatId, templateId: config.templateId,
  childInstanceId: config.childInstanceId, takeoverQuestionId: config.takeoverQuestionId,
  state: 'RUNNING', acceptance: false,
  authenticationActions: false, modelPermissionByUser: false,
  marker: id('M2_MARKER'), markerFile: `${id('m2-marker')}.json`,
  launches: [], closes: [], operations: [], sessions: [], subscriptions: [], snapshots: {},
  providerCases: [], providerWorktreePlan: config.providerCases.map(row => ({
    driverId: row.driverId, instanceId: row.instanceId, seatId: row.seatId, worktreeId: row.worktreeId })),
  sideChatCases: [], v12: 'NOT_RUN_NOT_CONFIGURED',
  assertions: [], nativeCards: [], readbacks: [] };
for (const driverId of ['claude', 'opencode', 'grok']) {
  if (!config.providerCases.some(row => row.driverId === driverId)) {
    journal.providerCases.push({ driverId, result: 'NOT_RUN_NOT_CONFIGURED',
      reason: 'No providerCases row was supplied in the private M2 config' });
  }
}
const product = new ActualProduct(config, journal);
delete journal.driverBytes['m1-win11.mjs'];
delete journal.driverBytes['m1-readback.py'];
journal.driverBytes['m2-win11.mjs'] = sha256(path.join(here, 'm2-win11.mjs'));
journal.driverBytes['m2-readback.py'] = sha256(path.join(here, 'm2-readback.py'));
const check = (condition, name) => {
  if (!condition) throw Error(name);
  if (!journal.assertions.includes(name)) { journal.assertions.push(name); product.save(); }
};
const digest = bytes => createHash('sha256').update(bytes).digest('hex');

async function snapshot(phase) {
  for (const observer of config.observers) {
    const output = path.join(config.evidenceDirectory, `${observer.name}-${phase}.json`);
    if (fs.existsSync(output)) throw Error(`Original ${observer.name} snapshot exists`);
    const args = observer.args.map(value => value === '{output}' ? output : value);
    await new Promise((resolve, reject) => {
      const child = spawn(observer.runtime, args, { windowsHide: true, stdio: ['ignore', 'ignore', 'pipe'] });
      let stderr = ''; child.stderr.on('data', bytes => { stderr = (stderr + bytes).slice(-8192); });
      child.once('error', reject);
      child.once('exit', code => code === 0 ? resolve() : reject(Error(`Readonly ${observer.name} exit=${code}: ${stderr}`)));
    });
    journal.snapshots[`${observer.name}-${phase}`] = { file: path.basename(output), sha256: sha256(output) };
    product.save();
  }
}
function snapshotValue(name, phase) {
  const reference = journal.snapshots[`${name}-${phase}`];
  const file = path.join(config.evidenceDirectory, reference.file);
  check(sha256(file) === reference.sha256, `Original ${name}-${phase} snapshot hash`);
  return readJson(file);
}
async function readback(phase) {
  const output = path.join(config.evidenceDirectory, `m2-readback-${phase}.json`);
  if (fs.existsSync(output)) throw Error(`Existing ${phase} readback`);
  // The product has normally exited. immutable SQLite access is only here.
  await new Promise((resolve, reject) => {
    const child = spawn(config.python, [path.join(here, 'm2-readback.py'),
      config.stateRoot, output, config.result, phase],
    { windowsHide: true, stdio: ['ignore', 'ignore', 'pipe'] });
    let stderr = ''; child.stderr.on('data', bytes => { stderr = (stderr + bytes).slice(-8192); });
    child.once('error', reject);
    child.once('exit', code => code === 0 ? resolve() : reject(Error(`Original ${phase} readback exit=${code}: ${stderr}`)));
  });
  const value = readJson(output);
  journal.readbacks.push({ phase, file: path.basename(output), sha256: sha256(output),
    worktreeId: value.worktree?.id ?? null }); product.save();
  check(value.measurementPreservedDatabaseBytes && value.directCaseEvidence, `${phase}: immutable direct evidence`);
  return value;
}
async function providerBoundaryReadback() {
  const file = 'm2-provider-readback-final.json';
  const output = path.join(config.evidenceDirectory, file);
  if (fs.existsSync(output)) throw Error('Provider boundary readback already exists');
  await new Promise((resolve, reject) => {
    const child = spawn(config.python, [path.join(here, 'm2-provider-readback.py'),
      config.stateRoot, output, config.result],
      { windowsHide: true, stdio: ['ignore', 'ignore', 'pipe'] });
    let stderr = ''; child.stderr.on('data', bytes => { stderr = (stderr + bytes).slice(-8192); });
    child.once('error', reject);
    child.once('exit', code => code === 0 ? resolve() :
      reject(Error(`Original provider boundary readback exit=${code}: ${stderr}`)));
  });
  const value = readJson(output);
  check(value.measurementPreservedDatabaseBytes && value.acceptance === false &&
    value.databaseWrites === false && value.credentialReads === false,
    'Provider boundary immutable measurement preserved actual database bytes');
  journal.readbacks.push({ phase: 'provider-boundaries', file, sha256: sha256(output),
    checks: value.checks }); product.save();
  const original = journal.providerBoundaryCases.find(row => row.driverId === 'claude');
  if (original?.state === 'CLAUDE_QUESTION_FLOW_DIRECT_READBACK_REQUIRED') {
    check(value.checks.V03b === 'DIRECT_CLAUDE_EVIDENCE_REQUIRES_INDEPENDENT_REVIEW' &&
      value.cases.some(row => row.caseId === original.caseId && row.directQuestion &&
        row.directQuestion.hostTurnId === original.sendRequestId),
      'Original Claude question answer continuation read back from C/H/A and actual F marker');
  }
  return value;
}
async function recordProviderGoldens(originalReadback) {
  const reference = journal.readbacks.find(row => row.phase === 'final');
  check(reference && originalReadback.directCaseEvidence === true &&
    originalReadback.measurementPreservedDatabaseBytes === true &&
    originalReadback.sourceCommit === config.sourceCommit,
    'Provider goldens require the normally closed original M2 readback');
  const input = path.join(config.evidenceDirectory, reference.file);
  check(sha256(input) === reference.sha256, 'Original provider golden input bytes unchanged');
  journal.driverBytes['cli-protocol-golden.mjs'] = sha256(path.join(here, 'cli-protocol-golden.mjs'));
  journal.providerGoldens = []; product.save();
  for (const row of config.providerCases) {
    const observed = originalReadback.providerSessions.find(session => session.driverId === row.driverId);
    if (!observed || typeof observed.sessionId !== 'string') {
      journal.providerGoldens.push({ driverId: row.driverId,
        state: observed?.result ?? 'NOT_RUN_NO_ORIGINAL_PROVIDER_SESSION' });
      product.save(); continue;
    }
    check(observed.result === 'DIRECT_ORIGINAL_PROTOCOL_EXPORTED_NOT_OWNER_ACCEPTANCE',
      `${row.driverId}: original normalized output required for a golden`);
    const file = `m2-${row.driverId}-protocol-golden.json`;
    const output = path.join(config.evidenceDirectory, file);
    if (fs.existsSync(output)) throw Error(`${row.driverId}: golden output already exists`);
    await new Promise((resolve, reject) => {
      const child = spawn(process.execPath, [path.join(here, 'cli-protocol-golden.mjs'), 'import',
        '--frames', input, '--normalized', input, '--out', output,
        '--session-id', observed.sessionId, '--cli-version', row.version,
        '--binary-sha256', row.sha256, '--capture-id', `${journal.caseId}-${row.driverId}`,
        '--outcome', 'success'], { windowsHide: true, stdio: ['ignore', 'ignore', 'pipe'] });
      let stderr = ''; child.stderr.on('data', bytes => { stderr = (stderr + bytes).slice(-8192); });
      child.once('error', reject);
      child.once('exit', code => code === 0 ? resolve() :
        reject(Error(`${row.driverId}: original golden import exit=${code}: ${stderr}`)));
    });
    const golden = readJson(output);
    check(golden.schema === 'gogoke.cli-protocol-capture.v1' &&
      golden.manifest.cliDriver === row.driverId && golden.manifest.cliVersion === row.version &&
      golden.manifest.cliBinarySha256 === row.sha256 && golden.manifest.productSourceCommit === config.sourceCommit &&
      golden.manifest.inputSha256.privateFrames === reference.sha256 &&
      golden.manifest.inputSha256.normalizedOutput === reference.sha256 &&
      golden.manifest.baselineStatus === 'REVIEW_REQUIRED' && golden.manifest.acceptance === 'NOT_ASSESSED',
      `${row.driverId}: golden facts must bind original H/F identity and readback bytes`);
    journal.providerGoldens.push({ driverId: row.driverId, sessionId: observed.sessionId,
      file, sha256: sha256(output), state: 'REVIEW_REQUIRED_ACCEPTANCE_NOT_ASSESSED' }); product.save();
  }
}
async function historyBoundaryReadback(phase) {
  const file = `m2-history-boundaries-${phase}.json`;
  const output = path.join(config.evidenceDirectory, file);
  if (fs.existsSync(output)) throw Error(`Original history ${phase} readback already exists`);
  // Only after the exact candidate's normal close. The reader independently
  // verifies the latest close, empty WAL and each session's original domain.
  await new Promise((resolve, reject) => {
    const child = spawn(config.python, [path.join(here, 'm2-history-boundaries-readback.py'),
      config.stateRoot, output, config.result, phase],
      { windowsHide: true, stdio: ['ignore', 'ignore', 'pipe'] });
    let stderr = ''; child.stderr.on('data', bytes => { stderr = (stderr + bytes).slice(-8192); });
    child.once('error', reject);
    child.once('exit', code => code === 0 ? resolve() :
      reject(Error(`Original history ${phase} readback exit=${code}: ${stderr}`)));
  });
  const value = readJson(output);
  check(value.measurementPreservedDatabaseBytes === true && value.directFlowEvidence === true &&
    value.acceptance === false && value.databaseWrites === false && value.credentialReads === false &&
    (phase !== 'final' || value.directRefusalEvidence === true) &&
    (phase !== 'peer-final' || value.directPeerReadEvidence === true ||
      value.peerState === 'NOT_RUN_PEER_READ_DENIAL_UNQUALIFIED'),
    `History ${phase}: original H/A/D/F facts and immutable measurement`);
  const reference = { file, sha256: sha256(output) };
  journal.readbacks.push({ phase: `history-${phase}`, ...reference }); product.save();
  return reference;
}
async function runHistoryBoundaries() {
  if (!config.historyBoundary) {
    journal.historyFlow = 'NOT_RUN_NOT_CONFIGURED'; product.save(); return;
  }
  const bindings = config.historyBoundary.cases?.flatMap(row =>
    [row.projectA, row.projectB, row.sideBinding]);
  const forbiddenSeats = [config.seatId, config.childSeatId,
    config.sideChat?.sourceSeatId, config.sideChat?.sideSeatId,
    ...config.providerCases.map(row => row.seatId),
    config.rules?.submitter?.seatId, config.rules?.reviewer?.seatId,
    config.rules?.host?.destination?.seatId, config.rules?.host?.alternateDestination?.seatId].filter(Boolean);
  const forbiddenTrees = [config.worktreeId, journal.worktreeId,
    config.sideChat?.sourceWorktreeId, config.sideChat?.sideWorktreeId,
    ...config.providerCases.map(row => row.worktreeId),
    config.rules?.submitter?.worktreeId, config.rules?.reviewer?.worktreeId,
    config.rules?.host?.destination?.worktreeId, config.rules?.host?.alternateDestination?.worktreeId].filter(Boolean);
  check(Array.isArray(bindings) && bindings.length > 0 && bindings.every(binding => binding &&
    atom(binding.domainId) && binding.repositoryId === config.repositoryId && atom(binding.seatId) &&
    atom(binding.worktreeId) && !forbiddenSeats.includes(binding.seatId) &&
    !forbiddenTrees.includes(binding.worktreeId)),
    'History cases require exclusive pre-registered original E/F test objects in their actual domains');
  journal.driverBytes['m2-history-boundaries.mjs'] = sha256(path.join(here, 'm2-history-boundaries.mjs'));
  journal.driverBytes['m2-history-boundaries-readback.py'] = sha256(path.join(here, 'm2-history-boundaries-readback.py'));
  journal.historyFlow = 'RUNNING'; product.save();
  await product.launch();
  const { runHistoryBoundaryCases } = await import('./m2-history-boundaries.mjs');
  const record = await runHistoryBoundaryCases(product, { ...config, historyBoundary: {
    ...config.historyBoundary,
    normalCloseReadbackRestart: async phase => {
      await product.closeNormally();
      const reference = await historyBoundaryReadback(phase);
      await product.launch(); return reference;
    },
  } }, journal);
  check(record.state === 'FLOW_COMPLETE_DIRECT_READBACK_REQUIRED', 'Actual history and formal-refusal flow completed');
  await product.closeNormally();
  await historyBoundaryReadback('final');
  journal.historyFlow = 'DIRECT_FACTS_COMPLETE_ACCEPTANCE_FALSE'; product.save();
  if (config.historyBoundary.peerRead === true) {
    await product.launch();
    const { runHistoryPeerReadCases } = await import('./m2-history-boundaries.mjs');
    const peer = await runHistoryPeerReadCases(product, config, journal);
    check(peer.state === 'PEER_FLOW_COMPLETE_DIRECT_READBACK_REQUIRED',
      'History peer flow must preserve its original outcomes for direct readback');
    await product.closeNormally();
    const reference = await historyBoundaryReadback('peer-final');
    const facts = readJson(path.join(config.evidenceDirectory, reference.file));
    journal.historyPeerFlow = facts.directPeerReadEvidence === true
      ? 'DIRECT_PEER_DENIAL_FACTS_COMPLETE_ACCEPTANCE_FALSE'
      : facts.peerState;
    product.save();
  }
}
async function seatCard(seatId) {
  let reply = await product.operation('K-SEAT', 'state-card', seatId, {}, '0', ['APPLIED', 'STALE']);
  if (reply.status === 'STALE') reply = await product.operation('K-SEAT', 'state-card', seatId, {}, reply.revision);
  return reply;
}
async function sessionOp(session, operation, payload = {}, allowed = ['APPLIED']) {
  const reply = await product.operation('K-SESSION', operation, session.id,
    { generation: session.generation, ...payload }, session.revision, allowed);
  session.revision = reply.revision;
  if (reply.result.newGeneration) session.generation = reply.result.newGeneration;
  product.save(); return reply;
}
async function openUserSession(seatId, instanceId, worktreeId, tag) {
  const card = await seatCard(seatId);
  check(card.result.state === 'IDLE' && card.result.instanceId === instanceId, `${tag}: original seat Idle and bound`);
  const session = { id: id('m2Session'), seatId, instanceId, worktreeId,
    generation: (BigInt(card.result.generation) + 1n).toString(), revision: '0', cursor: '0', events: [], turns: [] };
  journal.sessions.push(session); product.save();
  await sessionOp(session, 'admission-reserve', { seatId });
  await sessionOp(session, 'admission-commit', { seatId });
  const open = await sessionOp(session, 'open', { seatId, repositoryId: config.repositoryId, worktreeId });
  session.threadId = open.result.threadId; product.save();
  return session;
}
async function openRetainedLead() {
  const reference = config.retainedPrelaunchFailure;
  check(reference && typeof reference.result === 'string' &&
    inside(path.resolve(reference.result), path.resolve(config.evidenceDirectory)) &&
    /^[a-f0-9]{64}$/.test(reference.sha256) && sha256(reference.result) === reference.sha256,
  'Retained lead uses the exact original failed journal');
  const prior = readJson(reference.result);
  const original = prior.operations?.at(-1);
  const saved = prior.sessions?.find(row => row.id === original?.request?.targetId);
  check(prior.schema === journal.schema && prior.state === 'FAIL' &&
    prior.domainId === config.domainId && prior.repositoryId === config.repositoryId &&
    prior.error?.includes('GOGOKE_DESIGN37_NATIVE_USER_OPERATION_FAILED:ERR') &&
    prior.error.includes('legacy account scope requires original stopped observer custody') &&
    original?.receipt === null && original.request.family === 'K-SESSION' &&
    original.request.operation === 'open' && original.request.domainId === config.domainId &&
    saved?.seatId === config.seatId && saved.instanceId === config.instanceId &&
    saved.worktreeId === config.worktreeId && !saved.threadId &&
    saved.events?.length === 0 && saved.turns?.length === 0 &&
    original.request.expectedRevision === saved.revision &&
    original.request.payload.generation === saved.generation &&
    original.request.payload.seatId === config.seatId &&
    original.request.payload.repositoryId === config.repositoryId &&
    original.request.payload.worktreeId === config.worktreeId &&
    JSON.stringify(JSON.parse(original.rawFrame)) === JSON.stringify(original.request),
  'Retained open is only the original definite prelaunch refusal, never an uncertain model request');
  const card = await seatCard(config.seatId);
  check(card.result.state === 'BUSY' && card.result.instanceId === config.instanceId &&
    card.result.generation === saved.generation,
  'Original committed lead claim remains bound; do not reserve or release another claim');
  const session = { ...saved, events: [], turns: [] };
  journal.sessions.push(session);
  journal.retainedPrelaunchFailure = { sha256: reference.sha256,
    requestId: original.request.requestId, sessionId: session.id };
  const record = { request: original.request, rawFrame: original.rawFrame,
    startedAt: new Date().toISOString(), receipt: null, originalPrelaunchRefusal: true };
  journal.operations.push(record); product.save();
  // One explicit recovery of the recorded prelaunch refusal. No reserve,
  // commit, general retry, altered frame or automatic resend is performed.
  const raw = await product.evaluate(`window.__TAURI_INTERNALS__.invoke('gogoke_design37_user_operation',{frame:${JSON.stringify(record.rawFrame)}})`);
  record.receipt = JSON.parse(raw); record.finishedAt = new Date().toISOString(); product.save();
  check(record.receipt.requestId === original.request.requestId &&
    record.receipt.targetId === session.id && record.receipt.family === 'K-SESSION' &&
    record.receipt.operation === 'open' && ['APPLIED', 'REPLAYED'].includes(record.receipt.status) &&
    typeof record.receipt.result.threadId === 'string' && record.receipt.result.threadId.length > 0,
  'Exact retained open returns its actual native thread');
  session.revision = record.receipt.revision; session.threadId = record.receipt.result.threadId;
  product.save(); return session;
}
async function output(session) {
  let reply = await product.operation('K-SESSION', 'output-stream', session.id,
    { generation: session.generation, afterCursor: session.cursor }, session.revision, ['APPLIED', 'STALE']);
  if (reply.status === 'STALE') {
    session.revision = reply.revision; product.save();
    reply = await product.operation('K-SESSION', 'output-stream', session.id,
      { generation: session.generation, afterCursor: session.cursor }, session.revision);
  }
  check(BigInt(reply.result.cursor) >= BigInt(session.cursor), 'Original output cursor monotonic');
  session.cursor = reply.result.cursor;
  session.events.push(...reply.result.events); product.save();
  if (reply.result.sourceError) throw Error(`Original ${session.id} source error: ${reply.result.sourceError}`);
  return reply.result;
}
async function observe(session, predicate, label) {
  const deadline = Date.now() + 600000;
  while (Date.now() < deadline) {
    const page = await output(session);
    if (await predicate(page)) return page;
    await delay(300); // Readonly facts only. Never repeat a send or tool call.
  }
  throw Error(`${label}: original observation deadline, no mutation replay`);
}
async function answerPrescribedCard(session, card) {
  check(session.seatId === config.seatId && card.state === 'OPEN', 'Only lead current native card may be answered');
  const recovered = await product.operation('K-QCARD', 'recover', card.cardId, {}, card.revision);
  const q = recovered.result.nativeQuestion;
  check(recovered.result.availableForAnswer && recovered.result.seatId === config.seatId &&
    recovered.result.generation === session.generation && q.threadId === session.threadId &&
    q.questions.length === 1 && q.questions[0].id === config.takeoverQuestionId &&
    q.questions[0].question === config.takeoverPrompt && !q.questions[0].isSecret,
  'Only preauthorized non-secret takeover question');
  // The fixed CLI can append its presentation-only recommendation suffix.
  // Accept only these two exact spellings of the already authorized answer,
  // reject ambiguous matches, and send back the original native label.
  const choices = q.questions[0].options.filter(value =>
    value.label === config.takeoverOption || value.label === `${config.takeoverOption} (Recommended)`);
  check(choices.length === 1, 'Unique prescribed takeover option present');
  const choice = choices[0];
  const answered = await product.operation('K-QCARD', 'answer', card.cardId,
    { generation: session.generation, answers: { [q.questions[0].id]: [choice.label] } }, recovered.revision);
  check(answered.result.state === 'ANSWERED' && answered.result.deliveryBasis === 'NATIVE_EXACT_WRITE_RECEIPT',
    'Original native C answer write receipt');
  journal.nativeCards.push({ cardId: card.cardId, answerRequestId: journal.operations.at(-1).request.requestId,
    questionId: q.questions[0].id, turnId: q.turnId, generation: session.generation }); product.save();
}
async function leadTurn(session, prompt, answerTakeover = false) {
  const start = session.events.length;
  const sent = await sessionOp(session, 'send', { body: prompt });
  session.turns.push({ sendRequestId: journal.operations.at(-1).request.requestId,
    sendReceipt: sent.status, startedAt: new Date().toISOString() }); product.save();
  let cardAnswered = false;
  await observe(session, async page => {
    const opened = page.nativeCardRefs.find(row => row.state === 'OPEN');
    if (opened && answerTakeover && !cardAnswered) {
      cardAnswered = true; await answerPrescribedCard(session, opened);
    } else if (opened) throw Error('Unexpected native question card');
    return session.events.slice(start).some(row => row._meta?.codexMethod === 'turn/completed');
  }, 'Lead turn completion');
  const events = session.events.slice(start);
  const completed = events.find(row => row._meta?.codexMethod === 'turn/completed');
  check(completed?._meta?.turnStatus === 'completed', 'Lead original CLI turn completed');
  if (answerTakeover) check(cardAnswered, 'Prescribed native takeover question was answered');
  session.turns.at(-1).turnId = completed._meta.turnId; product.save();
  return events;
}
function locatorFromLeadEvents(events) {
  // The tool lifecycle update carries the native host's original H send ACK.
  // Its IDs select a read only H output stream; closed A/F/H readback proves
  // their authority. Model prose is never the assertion source.
  const original = events.filter(row => row._meta?.codexMethod === 'item/completed' &&
    row._meta?.codexItemType === 'dynamicToolCall' && row.status === 'completed').flatMap(row => {
    // Fixed Codex item completion carries the H response as one inputText
    // content item. Preserve its exact receipt text; reject other shapes.
    const content = Array.isArray(row.rawOutput) ? row.rawOutput : row.rawOutput?.contentItems;
    const raw = Array.isArray(content) && content.length === 1 &&
      content[0]?.type === 'inputText' ? content[0].text : row.rawOutput;
    try {
      const parsed = typeof raw === 'string' ? JSON.parse(raw) : raw;
      return parsed && typeof parsed === 'object' ? [parsed] : [];
    } catch { return []; }
  }).filter(reply => reply.family === 'K-SESSION' && reply.operation === 'send' &&
    ['APPLIED', 'REPLAYED'].includes(reply.status) &&
    reply.result?.seatId === config.childSeatId && reply.result?.worktreeId);
  if (original.length === 1 && /^native-session-[a-f0-9]{40}$/.test(original[0].targetId) &&
      /^\d+$/.test(original[0].revision)) {
    return { sessionId: original[0].targetId, revision: original[0].revision,
      worktreeId: original[0].result.worktreeId, source: 'ORIGINAL_NATIVE_TOOL_ACK' };
  }
  throw Error('Original completed dynamic-tool H send ACK lacks one child/worktree selector; model prose is not a substitute');
}
async function childCompletion(locator) {
  const card = await seatCard(config.childSeatId);
  check(card.result.state === 'BUSY' && card.result.instanceId === config.childInstanceId,
    'Actual child seat Busy on approved instance');
  const child = { id: locator.sessionId, seatId: config.childSeatId,
    instanceId: config.childInstanceId, generation: card.result.generation.toString(),
    revision: locator.revision, cursor: '0', events: [], turns: [] };
  journal.sessions.push(child); product.save();
  await observe(child, () => child.events.some(event => event._meta?.codexMethod === 'turn/completed'),
    'Original child turn completion');
  const terminal = child.events.find(event => event._meta?.codexMethod === 'turn/completed');
  check(terminal?._meta?.turnStatus === 'completed', 'Child original CLI turn completed');
  child.turns.push({ turnId: terminal._meta.turnId, threadId: terminal._meta.threadId }); product.save();
  return child;
}
async function awaitChildIdle() {
  const deadline = Date.now() + 600000;
  while (Date.now() < deadline) {
    const card = await seatCard(config.childSeatId);
    if (card.result.state === 'IDLE') return card;
    await delay(300);
  }
  throw Error('Original model child stop not observed; no User substitute');
}
async function stopUser(session, release) {
  const stopped = await sessionOp(session, 'stop', { seatId: session.seatId });
  check(typeof stopped.result.stopFact === 'string' && stopped.result.stopFact.length > 0,
    `${session.id}: durable actual process stop`);
  if (release) await sessionOp(session, 'admission-release', { seatId: session.seatId });
  return stopped;
}
async function providerCase(row, instances) {
  const observed = instances.instances.find(item => item.instanceId === row.instanceId && item.driverId === row.driverId);
  const entry = { driverId: row.driverId, instanceId: row.instanceId,
    fixedVersion: row.version, fixedSha256: row.sha256, state: observed?.state ?? 'ABSENT' };
  journal.providerCases.push(entry); product.save();
  if (observed?.state !== 'LOGGED_IN') { entry.result = 'NOT_RUN_NOT_LOGGED_IN'; product.save(); return; }
  const session = await openUserSession(row.seatId, row.instanceId, row.worktreeId, row.driverId);
  const capability = await sessionOp(session, 'capability-probe');
  check(capability.result.version === row.version &&
    capability.result.binaryDigest === `sha256:${row.sha256}`, `${row.driverId}: original fixed CLI pin`);
  entry.capability = { version: capability.result.version, binaryDigest: capability.result.binaryDigest };
  const marker = id('M2_PROVIDER');
  const sent = await sessionOp(session, 'send', { body: `Private M2 protocol capture. Answer only ${marker}. Do not use tools, modify files, open a browser, or call a model other than this current turn.` }, ['APPLIED', 'UNKNOWN']);
  if (sent.status === 'UNKNOWN' && sent.result.reason) {
    throw Error(`${row.driverId}: original send UNKNOWN reason=${sent.result.reason}; no resend`);
  }
  entry.sendRequestId = journal.operations.at(-1).request.requestId; entry.sessionId = session.id;
  entry.sendStatus = sent.status; product.save();
  await observe(session, page => page.nativeInputReceipts?.some(fact => fact.requestId === entry.sendRequestId &&
    fact.receipt?.status === 'APPLIED'), `${row.driverId}: exact original input completion`);
  entry.result = 'ORIGINAL_INPUT_RECEIPTED_RAW_READBACK_REQUIRED'; product.save();
  await stopUser(session, true);
}

async function prepareSideWorktree(side, prefix) {
  const seatId = side[`${prefix}SeatId`], instanceId = side[`${prefix}InstanceId`];
  const worktreeId = side[`${prefix}WorktreeId`];
  const card = await seatCard(seatId);
  check(card.result.state === 'IDLE' && card.result.instanceId === instanceId,
    `V12 ${prefix}: original preconfigured Owner test seat Idle`);
  const create = await product.operation('K-WORKTREE', 'create', worktreeId,
    { repositoryId: config.repositoryId, seatId, layout: 'single' }, '0');
  check(create.result.worktreeId === worktreeId && create.result.state === 'CREATED' &&
    create.result.classification === 'SINGLE', `V12 ${prefix}: original F create receipt`);
  const register = await product.operation('K-WORKTREE', 'register', worktreeId, {}, '1');
  check(register.result.worktreeId === worktreeId, `V12 ${prefix}: original F register receipt`);
}

async function runSideChat() {
  if (!config.sideChat) return;
  journal.v12 = 'RUNNING'; product.save();
  const side = config.sideChat;
  const names = ['sourceSeatId', 'sourceInstanceId', 'sourceWorktreeId',
    'sideSeatId', 'sideInstanceId', 'sideWorktreeId'];
  check(names.every(name => atom(side[name])) &&
    side.lifecycleOwnership === 'EXCLUSIVE_V12_SOURCE_AND_SIDE' &&
    side.sourceSeatId !== side.sideSeatId &&
    ![config.seatId, config.childSeatId].includes(side.sourceSeatId) &&
    ![config.seatId, config.childSeatId].includes(side.sideSeatId) &&
    new Set([side.sourceWorktreeId, side.sideWorktreeId, config.worktreeId,
      journal.worktreeId]).size === 4,
  'V12 explicit independent test seats and fresh logical worktrees');
  const instances = await product.instances();
  for (const prefix of ['source', 'side']) {
    check(instances.instances.some(row => row.instanceId === side[`${prefix}InstanceId`] &&
      row.driverId === 'codex' && row.state === 'LOGGED_IN'),
    `V12 ${prefix}: actual admitted Codex login state`);
  }
  journal.sideChatPlan = Object.fromEntries(names.map(name => [name, side[name]])); product.save();
  await prepareSideWorktree(side, 'source');
  await prepareSideWorktree(side, 'side');
  await product.closeNormally();
  const original = await readback('side-worktrees');
  check(original.worktrees?.length === 2, 'V12 both original registered F paths read after close');
  const sideSource = original.worktrees.find(row => row.worktreeId === side.sourceWorktreeId);
  const sideTarget = original.worktrees.find(row => row.worktreeId === side.sideWorktreeId);
  check(Boolean(sideSource && sideTarget), 'V12 F logical IDs bind both original physical trees');
  await product.launch();
  check(Boolean(product.tester) && journal.connectionBackend?.agentActs === 0,
    'V12 restarted original e2e hard locator without agent.act');
  const artifact = journal.readbacks.at(-1);
  const prepared = { ...side, sourceWorktreeRoot: sideSource.path,
    sideWorktreeRoot: sideTarget.path, worktreeReadback: { file: artifact.file, sha256: artifact.sha256 },
    worktrees: original.worktrees, ledgerEpoch: snapshotValue('ledger', 'before').epoch,
    sourceCursor: '0', restartProduct: async () => {
      await product.closeNormally(); await product.launch();
      await product.instances();
      check(Boolean(product.tester) && journal.connectionBackend?.agentActs === 0,
        'V12 restart retained original hard locator');
    } };
  const { runSideChatCase } = await import('./m2-sidechat.mjs');
  journal.driverBytes['m2-sidechat.mjs'] = sha256(path.join(here, 'm2-sidechat.mjs'));
  product.save();
  const sideCase = await runSideChatCase(product, { ...config, sideChat: prepared }, journal);
  if (sideCase.state === 'NOT_RUN') {
    journal.v12 = 'NOT_RUN_MISSING_ORIGINAL_F_ARTIFACT'; product.save();
    throw Error('V12 original module did not run; preserve its reason and do not claim M2 flow complete');
  }
  check(sideCase.state === 'FLOW_COMPLETE_DIRECT_LEDGER_READBACK_REQUIRED',
    'V12 original module flow requires independent direct readback');
  journal.v12 = 'FLOW_COMPLETE_DIRECT_READBACK_INDEPENDENT_REVIEW_REQUIRED'; product.save();
}

async function rulesReadback(phase) {
  const file = `m2-rules-${phase}-${id('snapshot')}.json`;
  const output = path.join(config.evidenceDirectory, file);
  await new Promise((resolve, reject) => {
    const child = spawn(config.python, [path.join(here, 'm2-rules-readback.py'),
      config.stateRoot, output, config.result, phase],
    { windowsHide: true, stdio: ['ignore', 'ignore', 'pipe'] });
    let stderr = ''; child.stderr.on('data', bytes => { stderr = (stderr + bytes).slice(-8192); });
    child.once('error', reject);
    child.once('exit', code => code === 0 ? resolve() :
      reject(Error(`Original rules ${phase} readback exit=${code}: ${stderr}`)));
  });
  const value = readJson(output);
  check(value.measurementPreservedDatabaseBytes === true && value.databaseWrites === false &&
    value.credentialReads === false, `Rules ${phase}: immutable original measurement`);
  check(phase !== 'final' || value.directCaseEvidence === true,
    'Rules final requires direct evidence; baseline and checkpoint are not results');
  const reference = { file, sha256: sha256(output) };
  journal.readbacks.push({ phase: `rules-${phase}`, ...reference }); product.save();
  return reference;
}

async function runRules() {
  if (!config.rules) { journal.v08 = 'NOT_RUN_NOT_CONFIGURED'; product.save(); return; }
  const selections = [config.rules.submitter, config.rules.reviewer,
    config.rules.host?.destination, config.rules.host?.alternateDestination].filter(Boolean);
  check(selections.length === (config.rules.host ? 4 : 2) &&
    selections.every(row => ['seatId', 'instanceId', 'worktreeId'].every(name => atom(row[name]))) &&
    new Set(selections.map(row => row.seatId)).size === selections.length &&
    new Set(selections.map(row => row.worktreeId)).size === selections.length,
  'Rules require distinct explicit case-owned seats and fresh worktrees');
  const forbiddenSeats = [config.seatId, config.childSeatId,
    config.sideChat?.sourceSeatId, config.sideChat?.sideSeatId,
    ...config.providerCases.map(row => row.seatId)];
  const forbiddenTrees = [config.worktreeId, journal.worktreeId,
    config.sideChat?.sourceWorktreeId, config.sideChat?.sideWorktreeId,
    ...config.providerCases.map(row => row.worktreeId)];
  const instances = await product.instances();
  for (const row of selections) {
    check(!forbiddenSeats.includes(row.seatId) && !forbiddenTrees.includes(row.worktreeId) &&
      instances.instances.some(instance => instance.instanceId === row.instanceId &&
        instance.driverId === 'codex' && instance.state === 'LOGGED_IN'),
    'Rules use qualified exclusive Codex test identities');
  }
  for (const row of selections) {
    await prepareSideWorktree({ rulesSeatId: row.seatId, rulesInstanceId: row.instanceId,
      rulesWorktreeId: row.worktreeId }, 'rules');
  }
  journal.driverBytes['m2-rules.mjs'] = sha256(path.join(here, 'm2-rules.mjs'));
  journal.driverBytes['m2-rules-readback.py'] = sha256(path.join(here, 'm2-rules-readback.py'));
  journal.v08 = 'RUNNING'; product.save();
  await product.closeNormally();
  const baselineReadback = await rulesReadback('before');
  await product.launch();
  const submitterSession = await openUserSession(...['seatId', 'instanceId', 'worktreeId']
    .map(name => config.rules.submitter[name]), 'V08 submitter');
  const reviewerSession = await openUserSession(...['seatId', 'instanceId', 'worktreeId']
    .map(name => config.rules.reviewer[name]), 'V08 reviewer');
  const { runRulesCase } = await import('./m2-rules.mjs');
  const record = await runRulesCase(product, { ...config, rules: { ...config.rules,
    baselineReadback, submitterSession, reviewerSession,
    hostCheckpoint: async () => {
      await product.closeNormally(); const reference = await rulesReadback('checkpoint');
      await product.launch(); return reference;
    },
    openHostSession: row => openUserSession(row.seatId, row.instanceId, row.worktreeId, 'V08 Host recipient'),
    resumeRulesSession: async session => {
      const originalThread = session.threadId;
      const receipt = await sessionOp(session, 'resume');
      check(receipt.result.state === 'RUNNING' && receipt.result.newGeneration &&
        (!receipt.result.threadId || receipt.result.threadId === originalThread),
      'Rules resume preserves the original native logical thread');
      session.cursor = '0'; product.save();
    },
    stopRulesSession: session => stopUser(session, false),
    releaseStoppedRulesSession: session => sessionOp(session, 'admission-release', { seatId: session.seatId }),
  } }, journal);
  check(record.state === 'FLOW_COMPLETE_DIRECT_LEDGER_READBACK_REQUIRED', 'Rules actual flow completed');
  for (const session of [submitterSession, reviewerSession]) await stopUser(session, true);
  await product.closeNormally();
  await rulesReadback('final');
  journal.v08 = 'IMPLEMENTED_CASES_HAVE_DIRECT_EVIDENCE_V08_INCOMPLETE'; product.save();
  await product.launch();
}

try {
  check(fs.existsSync(config.testbedSource) && fs.statSync(config.testbedSource).isDirectory(), 'Private testbed source exists');
  check(!fs.existsSync(path.join(config.testbedSource, journal.markerFile)), 'Unique marker absent from main tree');
  await snapshot('before');
  await product.launch();
  check(Boolean(product.tester) && journal.connectionBackend?.agentActs === 0 &&
    journal.connectionBackend?.telemetryDisabled === true,
  'Original e2e hard locator with telemetry off and no agent.act');
  const instances = await product.instances(); journal.instances = instances; product.save();
  check(instances.instances.some(row => row.instanceId === config.instanceId &&
    row.driverId === 'codex' && row.state === 'LOGGED_IN'),
    'Actual Codex lead login status already qualified');
  check(instances.instances.some(row => row.instanceId === config.childInstanceId &&
    row.driverId === 'codex' && row.state === 'LOGGED_IN'),
  'Actual child Codex instance pre-admitted without any login action');
  if (config.policyInitialization) {
    check(config.policyInitialization.stage === 'OPEN' && config.policyInitialization.expectedRevision === '0',
      'Explicit fresh test-domain policy initialization only');
    const request = { schema: 'gogoke.37.owner-configuration.v1', command: 'policy-initialize',
      domainId: config.domainId, requestId: id('m2PolicyInit'), stage: 'OPEN', expectedRevision: '0' };
    const entry = { kind: 'CONTROLLER_TEST_DOMAIN_POLICY_INITIALIZATION', request,
      rawFrame: JSON.stringify(request), receipt: null };
    journal.operations.push(entry); product.save();
    entry.rawReceipt = await product.evaluate(`window.__TAURI_INTERNALS__.invoke('gogoke_design37_user_operation',{frame:${JSON.stringify(entry.rawFrame)}})`);
    entry.receipt = JSON.parse(entry.rawReceipt); product.save();
    check(entry.receipt.status === 'APPLIED' && entry.receipt.requestId === request.requestId &&
      entry.receipt.revision === '1', 'Original fresh test-domain policy initialized once');
  }
  const lead = config.retainedPrelaunchFailure ? await openRetainedLead() :
    await openUserSession(config.seatId, config.instanceId, config.worktreeId, 'lead');
  const leadControl = await seatCard(config.seatId);
  check(leadControl.result.instanceId === config.instanceId && leadControl.result.state === 'BUSY',
    'Original lead control facts identify this admitted seat');
  const firstPrompt = `Owner-authorized M2 case ${journal.caseId} in the private gogoke-seat-testbed. ` +
    `Your actual native seat ID is ${config.seatId}. Call gogoke_seat state-card with targetId ${JSON.stringify(config.seatId)}, ` +
    `expectedRevision ${JSON.stringify(leadControl.revision)}, payload {}. The literal "self" is not a seat ID. ` +
    `Read later seat revisions from the actual native state-card; session revisions are different. Its takeoverQuestions include ${config.takeoverQuestionId}. ` +
    `Ask exactly that question via native request_user_input, with non-secret option ${JSON.stringify(config.takeoverOption)}, then use gogoke_takeover takeover-answers with the original nativeAnswerSources. ` +
    `When takeoverReady, call gogoke_seat create-from-template for direct child ${config.childSeatId}, template ${config.templateId}, approved instance ${config.childInstanceId}; ` +
    `then gogoke_seat dispatch that child to repository ${config.repositoryId}, SINGLE layout. ` +
    `Child instruction: create only new relative file ${journal.markerFile} in its host-created worktree. Its exact UTF-8 bytes must be ${JSON.stringify(JSON.stringify({ marker: journal.marker }) + '\n')}. Do not run Git or touch .git; the host seals the stopped child's file. No remote, push, other path, credential or authentication action. ` +
    `Do not call User-origin operations for any model-authorized step. Do not stop the child yet. ` +
    `Finish only after the native dispatch tool returns. Report its original error if it fails; do not invent an ID or success.`;
  const firstEvents = await leadTurn(lead, firstPrompt, true);
  const locator = locatorFromLeadEvents(firstEvents);
  journal.childLocator = locator; product.save();
  const child = await childCompletion(locator);
  journal.childSessionId = child.id; product.save();
  await leadTurn(lead, `The original child turn completed. Call your native gogoke_seat state-card for child ${config.childSeatId}, then call gogoke_seat stop on that exact child with the current revision. Do not create or dispatch another child.`, false);
  await awaitChildIdle();
  await stopUser(lead, false);
  const stoppedLead = await seatCard(config.seatId);
  check(stoppedLead.result.state === 'BUSY' && stoppedLead.result.instanceId === config.instanceId &&
    (await product.instances()).instances.some(row => row.instanceId === config.instanceId),
  'Original lead stopped while same actual host remains alive for later merge');
  await product.closeNormally();
  const captured = await readback('capture');
  check(captured.worktree?.id && captured.worktree?.childSessionId === child.id,
    'Original A/F/H child and worktree binding');
  if (locator.worktreeId) check(locator.worktreeId === captured.worktree.id,
    'Original native ACK logical worktree ID agrees with A/F');
  journal.worktreeId = captured.worktree.id; product.save();
  await product.launch();
  // A reopened WebView does not imply that its lazy native User host exists.
  await product.instances();
  check(Boolean(product.tester) && journal.connectionBackend?.agentActs === 0,
    'Resumed original e2e hard locator without agent.act');
  const decision = captured.controllerMergeDecision;
  check(decision?.decision === 'MERGE_EXACT_PRIVATE_TEST_MARKER_ONLY' &&
    decision.scope === config.repositoryId && decision.childHead === captured.worktree.childHeadBeforeSeal &&
    decision.markerSha256 === captured.worktree.markerSha256 &&
    JSON.stringify(decision.changedPaths) === JSON.stringify([journal.markerFile]) &&
    /^[1-9][0-9]*$/.test(decision.policyRevision), 'Controller narrow private testbed merge decision');
  const grant = { schema: 'gogoke.37.owner-configuration.v1', command: 'policy-call-grant',
    domainId: config.domainId, requestId: id('m2MergeGrant'), callerSeatId: config.seatId,
    targetId: 'MAIN', action: 'MERGE', expiresAtMs: String(Date.now() + 3600000),
    expectedRevision: decision.policyRevision };
  const entry = { kind: 'CONTROLLER_TESTBED_MERGE_CONFIGURATION', request: grant,
    rawFrame: JSON.stringify(grant), decision, receipt: null };
  journal.operations.push(entry); product.save();
  entry.rawReceipt = await product.evaluate(`window.__TAURI_INTERNALS__.invoke('gogoke_design37_user_operation',{frame:${JSON.stringify(entry.rawFrame)}})`);
  entry.receipt = JSON.parse(entry.rawReceipt); product.save();
  check(entry.receipt.status === 'APPLIED' && entry.receipt.requestId === grant.requestId &&
    entry.receipt.command === grant.command && entry.receipt.revision ===
    (BigInt(decision.policyRevision) + 1n).toString(), 'Original narrow MERGE grant CAS receipt');
  const resumed = await sessionOp(lead, 'resume');
  check(resumed.result.state === 'RUNNING' && resumed.result.newGeneration,
    'Original lead H session resumed after normal close');
  lead.cursor = '0'; product.save();
  await leadTurn(lead, `Owner-authorized M2 merge of the exact registered testbed worktree ${captured.worktree.id}. ` +
    `Call native gogoke_worktree merge with targetId ${captured.worktree.id}, expectedRevision ${captured.worktree.revision}, ` +
    `payload decision MERGE and reason "M2 original child file ${journal.markerFile}". ` +
    `Only this original worktree; no User substitute, new create, push or remote operation.`, false);
  let graph = await product.operation('K-WORKTREE', 'graph-query', captured.worktree.id, {},
    captured.worktree.revision.toString(), ['APPLIED', 'STALE']);
  if (graph.status === 'STALE') graph = await product.operation('K-WORKTREE', 'graph-query',
    captured.worktree.id, {}, graph.revision);
  check(graph.result.state === 'MERGED' && /^[a-f0-9]{40}$/.test(graph.result.mergeTargetCommit),
    'Actual F graph reports merged original worktree');
  await stopUser(lead, true);
  await runSideChat();
  await runRules();
  const latest = await product.instances();
  for (const row of config.providerCases) await providerCase(row, latest);
  journal.providerCases.push({ driverId: 'antigravity', result: 'NOT_RUN_UNSUPPORTED_INSTANCE_LOGIN',
    reason: 'Shared Windows-user keyring with no evidenced login-only flow; Owner deferred B5 from this OT4b batch' }); product.save();
  await product.closeNormally();
  const final = await readback('final');
  await recordProviderGoldens(final);
  check(final.worktree.id === captured.worktree.id && final.worktree.mergeTargetCommit === graph.result.mergeTargetCommit,
    'Original final F/Git merge receipt and graph agree');
  check(final.worktree.childHeadBeforeSeal === captured.worktree.childHeadBeforeSeal &&
    final.worktree.markerSha256 === captured.worktree.markerSha256 &&
    /^[a-f0-9]{40}$/.test(final.worktree.childCommit),
    'Host sealed the exact captured original marker and child HEAD');
  check(Array.isArray(final.providerWorktrees) && final.providerWorktrees.length === config.providerCases.length,
    'Configured provider paths come from normally closed original F registrations');
  const boundaryRows = config.providerCases.map(row => {
    const tree = final.providerWorktrees.find(value => value.worktreeId === row.worktreeId);
    check(tree && tree.driverId === row.driverId && tree.instanceId === row.instanceId &&
      tree.seatId === row.seatId, `${row.driverId}: closed original F provider worktree identity`);
    return { ...row, worktreeRoot: tree.path };
  });
  if (boundaryRows.length >= 1 && boundaryRows.length <= 3) {
    await product.launch();
    const { runProviderBoundaryCases } = await import('./m2-provider-cases.mjs');
    await runProviderBoundaryCases(product, { ...config, providerBoundary: { cases: boundaryRows } }, journal);
    await product.closeNormally();
    await providerBoundaryReadback();
  } else {
    journal.providerBoundarySummary = { V03b: 'NOT_RUN_NOT_CONFIGURED',
      V04b: 'NOT_RUN_NOT_CONFIGURED',
      V10: 'NOT_RUN_NOT_CONFIGURED', acceptance: false };
    product.save();
  }
  await runHistoryBoundaries();
  await snapshot('after');
  for (const observer of config.observers) {
    const before = snapshotValue(observer.name, 'before'), after = snapshotValue(observer.name, 'after');
    check(Array.isArray(observer.equalFields) && observer.equalFields.length > 0,
      `${observer.name}: explicit formal comparison fields`);
    for (const field of observer.equalFields) check(Object.hasOwn(before, field) && Object.hasOwn(after, field) &&
      JSON.stringify(before[field]) === JSON.stringify(after[field]), `${observer.name}: unchanged ${field}`);
  }
  for (const phase of ['before', 'after']) {
    const memory = snapshotValue('memory', phase);
    check(memory.memoryDataUnchangedByRead && memory.stage1OutputCount === 0 && memory.memoryJobCount === 0,
      'Actual memory store unchanged by measurement and without memory jobs');
  }
  journal.state = 'ACTUAL_FLOW_COMPLETE_DIRECT_READBACK_REVIEW_REQUIRED'; product.save();
} catch (error) {
  journal.state = 'FAIL'; journal.error = String(error.stack ?? error);
  if (journal.v12 === 'RUNNING') journal.v12 = 'FAIL_ORIGINAL_CASE_RETAINED';
  if (journal.historyFlow === 'RUNNING') journal.historyFlow = 'FAIL_ORIGINAL_CASE_RETAINED';
  journal.currentEndpoint = product.endpoint ?? null; product.save(); process.exitCode = 1;
  // Keep the original product and request custody available for Controller.
  try { await product.preserveFailure(); }
  catch (disconnectError) { journal.disconnectError = String(disconnectError); product.save(); }
}
