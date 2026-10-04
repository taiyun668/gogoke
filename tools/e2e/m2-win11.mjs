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
    config.instanceId !== 'codexTestM1' || config.repositoryId !== 'gogokeSeatTestbed' ||
    !atom(config.domainId) || !atom(config.seatId) || !atom(config.childSeatId) ||
    !atom(config.templateId) || !atom(config.childInstanceId) ||
    !atom(config.takeoverQuestionId) || config.seatId === config.childSeatId ||
    !/^[a-f0-9]{40}$/.test(config.sourceCommit) ||
    !Array.isArray(config.providerCases) || config.providerCases.length !== 3 ||
    new Set(config.providerCases.map(row => row.driverId)).size !== 3 ||
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
    (phase !== 'final' || value.directRefusalEvidence === true),
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
  const choice = q.questions[0].options.find(value => value.label === config.takeoverOption);
  check(Boolean(choice), 'Exact prescribed takeover option present');
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
    stopRulesSession: session => stopUser(session, true),
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
  check(instances.instances.some(row => row.instanceId === config.instanceId && row.state === 'LOGGED_IN'),
    'Actual Codex lead login status already qualified');
  check(instances.instances.some(row => row.instanceId === config.childInstanceId &&
    row.driverId === 'codex' && row.state === 'LOGGED_IN'),
  'Actual child Codex instance pre-admitted without any login action');
  const lead = await openUserSession(config.seatId, config.instanceId, config.worktreeId, 'lead');
  const firstPrompt = `Owner-authorized M2 case ${journal.caseId} in the private gogoke-seat-testbed. ` +
    `Use the real native gogoke_seat state-card for your own seat. Its takeoverQuestions include ${config.takeoverQuestionId}. ` +
    `Ask exactly that question via native request_user_input, with non-secret option ${JSON.stringify(config.takeoverOption)}, then use gogoke_takeover takeover-answers with the original nativeAnswerSources. ` +
    `When takeoverReady, call gogoke_seat create-from-template for direct child ${config.childSeatId}, template ${config.templateId}, approved instance ${config.childInstanceId}; ` +
    `then gogoke_seat dispatch that child to repository ${config.repositoryId}, SINGLE layout. ` +
    `Child instruction: create only new relative file ${journal.markerFile} in its host-created worktree, JSON marker exactly ${journal.marker}; commit that file locally. No remote, push, other path, credential or authentication action. ` +
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
  check(Boolean(product.tester) && journal.connectionBackend?.agentActs === 0,
    'Resumed original e2e hard locator without agent.act');
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
  journal.providerCases.push({ driverId: 'antigravity', result: 'NOT_RUN_OWNER_DECISION_PENDING' }); product.save();
  await product.closeNormally();
  const final = await readback('final');
  check(final.worktree.id === captured.worktree.id && final.worktree.mergeTargetCommit === graph.result.mergeTargetCommit,
    'Original final F/Git merge receipt and graph agree');
  check(Array.isArray(final.providerWorktrees) && final.providerWorktrees.length === 3,
    'Three provider paths come from normally closed original F registrations');
  const boundaryRows = config.providerCases.map(row => {
    const tree = final.providerWorktrees.find(value => value.worktreeId === row.worktreeId);
    check(tree && tree.driverId === row.driverId && tree.instanceId === row.instanceId &&
      tree.seatId === row.seatId, `${row.driverId}: closed original F provider worktree identity`);
    return { ...row, worktreeRoot: tree.path };
  });
  await product.launch();
  const { runProviderBoundaryCases } = await import('./m2-provider-cases.mjs');
  await runProviderBoundaryCases(product, { ...config, providerBoundary: { cases: boundaryRows } }, journal);
  await product.closeNormally();
  await providerBoundaryReadback();
  // Complete the earlier readers before adding cross-domain history sessions;
  // their case scopes must never be widened by later journal entries.
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
