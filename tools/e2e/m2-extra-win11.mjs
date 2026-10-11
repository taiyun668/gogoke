// Independent installed-product entry for M2 V08 rules cases.
// V12 remains NOT_RUN until its original closed F readback is available.
import fs from 'node:fs';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { ActualProduct, readJson, sha256, id } from './product-cdp.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const configPath = process.argv[2];
if (!configPath) throw Error('Private M2 extra config path is required');
const config = readJson(configPath);
const atom = value => typeof value === 'string' && /^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/.test(value);
const unique = values => new Set(values).size === values.length;
const inside = (child, parent) => child === parent || child.startsWith(parent + path.sep);
const formalFields = ['formal', 'registeredFormal', 'formalData', 'formalRegistry', 'shortcuts'];

if (process.platform !== 'win32' || !/^\d+\.\d+\.\d+$/.test(config.version ?? '') ||
    config.repositoryId !== 'gogokeSeatTestbed' ||
    !/^[a-f0-9]{40}$/.test(config.sourceCommit ?? '') ||
    !atom(config.domainId) || !atom(config.instanceId) ||
    typeof config.installedSha256 !== 'object' || config.installedSha256 === null ||
    !['installed', 'version', 'installedSha256', 'registryKey', 'pwsh', 'python',
      'evidenceDirectory', 'result', 'stateRoot', 'testbedSource', 'domainId', 'instanceId',
      'observers', 'rules'].every(key => config[key] !== undefined) ||
    config.policyInitialization !== undefined ||
    !Array.isArray(config.observers) || !['formal', 'memory', 'ledger'].every(name =>
      config.observers.some(observer => observer.name === name)) ||
    !formalFields.every(field => config.observers.find(observer => observer.name === 'formal')
      .equalFields?.includes(field)) ||
    fs.existsSync(config.result) || !fs.existsSync(config.evidenceDirectory) ||
    !inside(path.resolve(config.result), path.resolve(config.evidenceDirectory)) ||
    [config.installed, config.stateRoot, config.testbedSource, path.resolve(here, '..', '..')]
      .some(root => inside(path.resolve(config.evidenceDirectory), path.resolve(root))) ||
    path.resolve(config.stateRoot) === path.resolve(config.testbedSource)) {
  throw Error('Fresh private V08 run with exact installed candidate, testbed, observers, and an existing policy head is required');
}

const rules = config.rules;
const crossProjectOnly = rules.selection === 'CROSS_PROJECT_ONLY';
if (rules.selection !== undefined && !crossProjectOnly) {
  throw Error('The only V08 one-axis selection is CROSS_PROJECT_ONLY');
}
if (rules.foreignProject !== undefined && (
    !atom(rules.foreignProject?.domainId) || !atom(rules.foreignProject?.gateId) ||
    rules.foreignProject.domainId === config.domainId ||
    typeof rules.foreignProject.ownerGate?.rawFrame !== 'string' ||
    typeof rules.foreignProject.ownerGate?.rawReceipt !== 'string' ||
    (crossProjectOnly ? typeof rules.foreignProject.ownerHead?.rawFrame !== 'string' ||
      typeof rules.foreignProject.ownerHead?.rawReceipt !== 'string' ||
      Object.keys(rules.foreignProject).sort().join(',') !== 'domainId,gateId,ownerGate,ownerHead' ||
      Object.keys(rules.foreignProject.ownerHead).sort().join(',') !== 'rawFrame,rawReceipt' :
      Object.keys(rules.foreignProject).length !== 3) ||
    Object.keys(rules.foreignProject.ownerGate).length !== 2)) {
  throw Error('Foreign fixture requires a distinct real domain/gate and original NativeUser Owner bytes');
}
if (crossProjectOnly && (rules.foreignProject === undefined || rules.host !== undefined)) {
  throw Error('CROSS_PROJECT_ONLY requires the original B head/gate bytes and excludes Host cases');
}
if (crossProjectOnly) {
  const foreign = rules.foreignProject;
  const head = JSON.parse(foreign.ownerHead.rawFrame);
  const headReceipt = JSON.parse(foreign.ownerHead.rawReceipt);
  const gate = JSON.parse(foreign.ownerGate.rawFrame);
  const gateReceipt = JSON.parse(foreign.ownerGate.rawReceipt);
  const exactKeys = (value, names) => value && typeof value === 'object' &&
    Object.keys(value).sort().join(',') === [...names].sort().join(',');
  if (!exactKeys(head, ['schema','command','domainId','requestId','stage','expectedRevision']) ||
      head.schema !== 'gogoke.37.owner-configuration.v1' || head.command !== 'policy-initialize' ||
      head.domainId !== foreign.domainId || !atom(head.requestId) || !atom(head.stage) || head.expectedRevision !== '0' ||
      !exactKeys(headReceipt, ['schema','command','requestId','status','revision']) ||
      headReceipt.schema !== head.schema || headReceipt.command !== head.command ||
      headReceipt.requestId !== head.requestId || headReceipt.status !== 'APPLIED' || headReceipt.revision !== '1' ||
      !exactKeys(gate, ['schema','command','domainId','requestId','gateId','submitterSeatId','reviewerSeatId',
        'fromStage','toStage','rejectCap','expectedRevision']) ||
      gate.schema !== head.schema || gate.command !== 'policy-gate' || gate.domainId !== foreign.domainId ||
      gate.gateId !== foreign.gateId || !atom(gate.requestId) || !atom(gate.submitterSeatId) ||
      !atom(gate.reviewerSeatId) || gate.submitterSeatId === gate.reviewerSeatId || !atom(gate.fromStage) ||
      !atom(gate.toStage) || gate.fromStage !== head.stage || gate.fromStage === gate.toStage ||
      !Number.isInteger(gate.rejectCap) || gate.rejectCap < 1 ||
      gate.expectedRevision !== headReceipt.revision ||
      !exactKeys(gateReceipt, ['schema','command','requestId','status','revision']) ||
      gateReceipt.schema !== gate.schema || gateReceipt.command !== gate.command ||
      gateReceipt.requestId !== gate.requestId || gateReceipt.status !== 'APPLIED' || gateReceipt.revision !== '2') {
    throw Error('CROSS_PROJECT_ONLY requires complete original NativeUser B initialize/gate frames and APPLIED receipts before launch');
  }
  const local = rules.sameScopePolicy;
  if (!exactKeys(local, ['ownerHead','ownerGate','ownerGrant']) ||
      !['ownerHead','ownerGate','ownerGrant'].every(name => exactKeys(local[name], ['rawFrame','rawReceipt']) &&
        typeof local[name].rawFrame === 'string' && typeof local[name].rawReceipt === 'string')) {
    throw Error('CROSS_PROJECT_ONLY requires the original A NativeUser initialize/gate/REVIEW-grant frames and receipts before launch');
  }
  const aHead = JSON.parse(local.ownerHead.rawFrame), aHeadReceipt = JSON.parse(local.ownerHead.rawReceipt);
  const aGate = JSON.parse(local.ownerGate.rawFrame), aGateReceipt = JSON.parse(local.ownerGate.rawReceipt);
  const aGrant = JSON.parse(local.ownerGrant.rawFrame), aGrantReceipt = JSON.parse(local.ownerGrant.rawReceipt);
  if (!exactKeys(aHead, ['schema','command','domainId','requestId','stage','expectedRevision']) ||
      aHead.schema !== 'gogoke.37.owner-configuration.v1' || aHead.command !== 'policy-initialize' ||
      aHead.domainId !== config.domainId || !atom(aHead.requestId) || !atom(aHead.stage) || aHead.expectedRevision !== '0' ||
      !exactKeys(aHeadReceipt, ['schema','command','requestId','status','revision']) ||
      aHeadReceipt.schema !== aHead.schema || aHeadReceipt.command !== aHead.command ||
      aHeadReceipt.requestId !== aHead.requestId || aHeadReceipt.status !== 'APPLIED' || aHeadReceipt.revision !== '1' ||
      !exactKeys(aGate, ['schema','command','domainId','requestId','gateId','submitterSeatId','reviewerSeatId',
        'fromStage','toStage','rejectCap','expectedRevision']) ||
      aGate.schema !== aHead.schema || aGate.command !== 'policy-gate' || aGate.domainId !== config.domainId ||
      !atom(aGate.requestId) || !atom(aGate.gateId) || !atom(aGate.submitterSeatId) || !atom(aGate.reviewerSeatId) ||
      aGate.submitterSeatId !== rules.submitter.seatId || aGate.reviewerSeatId !== rules.reviewer.seatId ||
      aGate.fromStage !== aHead.stage || !atom(aGate.toStage) || aGate.toStage === aGate.fromStage ||
      !Number.isInteger(aGate.rejectCap) || aGate.rejectCap < 1 || aGate.expectedRevision !== aHeadReceipt.revision ||
      !exactKeys(aGateReceipt, ['schema','command','requestId','status','revision']) ||
      aGateReceipt.schema !== aGate.schema || aGateReceipt.command !== aGate.command ||
      aGateReceipt.requestId !== aGate.requestId || aGateReceipt.status !== 'APPLIED' || aGateReceipt.revision !== '2' ||
      !exactKeys(aGrant, ['schema','command','domainId','requestId','callerSeatId','targetId','action','expiresAtMs','expectedRevision']) ||
      aGrant.schema !== aHead.schema || aGrant.command !== 'policy-call-grant' || aGrant.domainId !== config.domainId ||
      !atom(aGrant.requestId) ||
      aGrant.callerSeatId !== rules.submitter.seatId || aGrant.targetId !== rules.reviewer.seatId ||
      aGrant.action !== 'REVIEW' || aGrant.expiresAtMs !== null || aGrant.expectedRevision !== aGateReceipt.revision ||
      !exactKeys(aGrantReceipt, ['schema','command','requestId','status','revision']) ||
      aGrantReceipt.schema !== aGrant.schema || aGrantReceipt.command !== aGrant.command ||
      aGrantReceipt.requestId !== aGrant.requestId || aGrantReceipt.status !== 'APPLIED' || aGrantReceipt.revision !== '3') {
    throw Error('CROSS_PROJECT_ONLY requires exact original A NativeUser policy receipts for the selected caller/reviewer and a permanent REVIEW grant');
  }
}
if (rules.lifecycleOwnership !== 'EXCLUSIVE_V08_SUBMITTER_AND_REVIEWER' ||
    rules.policyOwnership !== 'EXCLUSIVE_V08_POLICY_DOMAIN') {
  throw Error('V08 requires the two explicit exclusive ownership values');
}
const selections = [rules.submitter, rules.reviewer, rules.host?.destination,
  rules.host?.alternateDestination].filter(Boolean);
if (selections.length !== (rules.host ? 4 : 2) ||
    selections.some(row => !['seatId', 'instanceId', 'worktreeId'].every(key => atom(row[key]))) ||
    !unique(selections.map(row => row.seatId)) || !unique(selections.map(row => row.worktreeId))) {
  throw Error('V08 needs two or four distinct precreated Codex seats and fresh worktree IDs');
}
if (rules.host && (rules.host.lifecycleOwnership !== 'EXCLUSIVE_V08_HOST_RECIPIENTS' ||
    !atom(rules.host.busyQuestion?.questionId) ||
    typeof rules.host.busyQuestion?.optionLabel !== 'string' ||
    !rules.host.busyQuestion.optionLabel.trim())) {
  throw Error('V08 Host recipients require their exact ownership marker and nonsecret busy-question fields');
}
if (config.sideChat) {
  const side = config.sideChat;
  const sideSeats = [side.sourceSeatId, side.sideSeatId];
  const sideTrees = [side.sourceWorktreeId, side.sideWorktreeId];
  if (side.lifecycleOwnership !== 'EXCLUSIVE_V12_SOURCE_AND_SIDE' ||
      [...sideSeats, side.sourceInstanceId, side.sideInstanceId, ...sideTrees].some(value => !atom(value)) ||
      !unique(sideSeats) || !unique(sideTrees) ||
      sideSeats.some(value => selections.some(row => row.seatId === value)) ||
      sideTrees.some(value => selections.some(row => row.worktreeId === value))) {
    throw Error('Deferred V12 selections must remain valid and disjoint from V08');
  }
}

const resultPath = path.resolve(config.result);
const evidencePath = path.resolve(config.evidenceDirectory);
config.result = resultPath;
config.evidenceDirectory = evidencePath;
const journal = {
  schema: 'gogoke.37.m2-win11-e2e.v1',
  caseId: id('m2Extra'),
  sourceCommit: config.sourceCommit,
  domainId: config.domainId,
  repositoryId: config.repositoryId,
  testbedSource: config.testbedSource,
  state: 'RUNNING',
  acceptance: false,
  authenticationActions: false,
  modelPermissionByUser: false,
  launches: [],
  closes: [],
  operations: [],
  sessions: [],
  snapshots: {},
  extraSnapshots: {},
  readbacks: [],
  assertions: [],
  providerCases: [],
  providerWorktreePlan: [],
  sideChatCases: [],
  rulesCases: [],
  foreignProject: rules.foreignProject ?? null,
  rulesSelection: crossProjectOnly ? 'CROSS_PROJECT_ONLY' : 'ALL',
  ...(crossProjectOnly ? { rulesSourceSelection: { ...rules.submitter }, sameScopePolicy: rules.sameScopePolicy } : {}),
  v12: 'NOT_RUN_ORIGINAL_M2_SIDE_WORKTREE_READBACK_REQUIRED',
  v08: 'RUNNING',
};
if (config.sideChat) {
  journal.sideChatDeferred = {
    lifecycleOwnership: config.sideChat.lifecycleOwnership,
    sourceSeatId: config.sideChat.sourceSeatId,
    sourceInstanceId: config.sideChat.sourceInstanceId,
    sourceWorktreeId: config.sideChat.sourceWorktreeId,
    sideSeatId: config.sideChat.sideSeatId,
    sideInstanceId: config.sideChat.sideInstanceId,
    sideWorktreeId: config.sideChat.sideWorktreeId,
    reason: 'Existing m2-readback side-worktrees reader requires the original M2 child dispatch/merge chain; this entry does not substitute or replay it.',
  };
}

const product = new ActualProduct(config, journal);
delete journal.driverBytes['m1-win11.mjs'];
delete journal.driverBytes['m1-readback.py'];
journal.driverBytes['m2-extra-win11.mjs'] = sha256(fileURLToPath(import.meta.url));
journal.driverBytes['m2-rules.mjs'] = sha256(path.join(here, 'm2-rules.mjs'));
journal.driverBytes['m2-rules-readback.py'] = sha256(path.join(here, 'm2-rules-readback.py'));
product.save();

const check = (condition, message) => {
  if (!condition) throw Error(message);
  if (!journal.assertions.includes(message)) {
    journal.assertions.push(message);
    product.save();
  }
};

async function snapshot(phase, names = null) {
  for (const observer of config.observers.filter(row => names === null || names.includes(row.name))) {
    const file = `m2-extra-${observer.name}-${phase}-${id('snapshot')}.json`;
    const output = path.join(evidencePath, file);
    if (fs.existsSync(output)) throw Error(`Original ${observer.name} ${phase} snapshot exists`);
    const args = observer.args.map(value => value === '{output}' ? output : value);
    await new Promise((resolve, reject) => {
      const child = spawn(observer.runtime, args, { windowsHide: true, stdio: ['ignore', 'ignore', 'pipe'] });
      let stderr = '';
      child.stderr.on('data', bytes => { stderr = (stderr + bytes).slice(-8192); });
      child.once('error', reject);
      child.once('exit', code => code === 0 ? resolve() : reject(Error(`Readonly ${observer.name} ${phase} exit=${code}: ${stderr}`)));
    });
    const reference = { file, sha256: sha256(output) };
    journal.extraSnapshots[`${observer.name}-${phase}`] = reference;
    product.save();
  }
}

function snapshotValue(name, phase) {
  const reference = journal.extraSnapshots[`${name}-${phase}`];
  check(reference && path.basename(reference.file) === reference.file, `${name}-${phase}: private snapshot reference`);
  const file = path.join(evidencePath, reference.file);
  check(sha256(file) === reference.sha256, `${name}-${phase}: original snapshot hash`);
  return readJson(file);
}

async function compareSnapshots() {
  for (const observer of config.observers) {
    const before = snapshotValue(observer.name, 'before');
    const after = snapshotValue(observer.name, 'after');
    for (const field of observer.equalFields ?? []) {
      check(Object.hasOwn(before, field) && Object.hasOwn(after, field) &&
        JSON.stringify(before[field]) === JSON.stringify(after[field]),
      `${observer.name}: unchanged ${field}`);
    }
  }
  const memory = snapshotValue('memory', 'after');
  check(memory.memoryDataUnchangedByRead && memory.stage1OutputCount === 0 && memory.memoryJobCount === 0,
    'Actual memory store unchanged by measurement and without memory jobs');
}

async function rulesReadback(phase) {
  const file = `m2-extra-rules-${phase}-${id('snapshot')}.json`;
  const output = path.join(evidencePath, file);
  if (fs.existsSync(output)) throw Error(`Original V08 ${phase} readback exists`);
  await new Promise((resolve, reject) => {
    const child = spawn(config.python, [path.join(here, 'm2-rules-readback.py'),
      config.stateRoot, output, resultPath, phase], { windowsHide: true, stdio: ['ignore', 'ignore', 'pipe'] });
    let stderr = '';
    child.stderr.on('data', bytes => { stderr = (stderr + bytes).slice(-8192); });
    child.once('error', reject);
    child.once('exit', code => code === 0 ? resolve() :
      reject(Error(`Original V08 ${phase} readback exit=${code}: ${stderr}`)));
  });
  const value = readJson(output);
  check(value.measurementPreservedDatabaseBytes === true && value.databaseWrites === false &&
    value.credentialReads === false, `V08 ${phase}: immutable original measurement`);
  check(phase !== 'final' || value.directCaseEvidence === true,
    'V08 final readback requires direct case evidence');
  const reference = { file, sha256: sha256(output) };
  journal.readbacks.push({ phase: `rules-${phase}`, ...reference });
  product.save();
  return reference;
}

async function seatCard(seatId) {
  let reply = await product.operation('K-SEAT', 'state-card', seatId, {}, '0', ['APPLIED', 'STALE']);
  if (reply.status === 'STALE') {
    reply = await product.operation('K-SEAT', 'state-card', seatId, {}, reply.revision);
  }
  return reply;
}

async function sessionOp(session, operation, payload = {}, allowed = ['APPLIED']) {
  const reply = await product.operation('K-SESSION', operation, session.id,
    { generation: session.generation, ...payload }, session.revision, allowed);
  session.revision = reply.revision;
  if (reply.result?.newGeneration) session.generation = reply.result.newGeneration;
  product.save();
  return reply;
}

async function openUserSession(seatId, instanceId, worktreeId, label) {
  const card = await seatCard(seatId);
  check(card.result.state === 'IDLE' && card.result.instanceId === instanceId,
    `${label}: actual original User E seat is IDLE and bound`);
  const session = { id: id('m2ExtraSession'), seatId, instanceId, worktreeId,
    generation: (BigInt(card.result.generation) + 1n).toString(), revision: '0', cursor: '0',
    events: [], turns: [] };
  journal.sessions.push(session);
  product.save();
  await sessionOp(session, 'admission-reserve', { seatId });
  await sessionOp(session, 'admission-commit', { seatId });
  const opened = await sessionOp(session, 'open', { seatId, repositoryId: config.repositoryId, worktreeId });
  session.threadId = opened.result.threadId;
  product.save();
  return session;
}

async function stopUser(session, release) {
  const stopped = await sessionOp(session, 'stop', { seatId: session.seatId });
  check(typeof stopped.result.stopFact === 'string' && stopped.result.stopFact.length > 0,
    `${session.id}: durable actual process stop fact`);
  if (release) await sessionOp(session, 'admission-release', { seatId: session.seatId });
  return stopped;
}

async function prepareRulesWorktree(selection, label) {
  const card = await seatCard(selection.seatId);
  check(card.result.state === 'IDLE' && card.result.instanceId === selection.instanceId,
    `${label}: precreated original User E seat IDLE and instance-bound`);
  const created = await product.operation('K-WORKTREE', 'create', selection.worktreeId,
    { repositoryId: config.repositoryId, seatId: selection.seatId, layout: 'single' }, '0');
  check(created.result.worktreeId === selection.worktreeId && created.result.state === 'CREATED' &&
    created.result.classification === 'SINGLE', `${label}: original F create receipt`);
  const registered = await product.operation('K-WORKTREE', 'register', selection.worktreeId, {}, '1');
  check(registered.result.worktreeId === selection.worktreeId, `${label}: original F register receipt`);
}

async function runRules() {
  const instancePage = await product.instances();
  for (const selection of selections) {
    check(instancePage.instances.some(row => row.instanceId === selection.instanceId &&
      row.driverId === 'codex' && row.state === 'LOGGED_IN'),
    'V08 uses an already admitted logged-in Codex instance');
  }
  journal.v08 = 'RUNNING';
  journal.driverBytes['m2-rules.mjs'] = sha256(path.join(here, 'm2-rules.mjs'));
  journal.driverBytes['m2-rules-readback.py'] = sha256(path.join(here, 'm2-rules-readback.py'));
  product.save();

  let baselineReadback = null;
  if (crossProjectOnly) {
    await product.closeNormally();
    baselineReadback = await rulesReadback('before');
    const baseline = readJson(path.join(evidencePath, baselineReadback.file));
    journal.crossProjectQualification = baseline.sameScopeQualification ?? null;
    if (baseline.sameScopeQualification?.state !== 'QUALIFIED_SAME_SCOPE_GATE_SUBMIT') {
      journal.v08 = 'NOT_RUN_CROSS_PROJECT_SAME_SCOPE_AUTHORIZATION_MISSING';
      journal.state = 'V08_NOT_RUN_CROSS_PROJECT_CALLER_NOT_QUALIFIED_IN_A';
      journal.crossProjectQualificationStatus = baseline.sameScopeQualification?.state ?? 'UNKNOWN';
      product.save();
      return;
    }
    await product.launch();
    await product.instances();
  }
  for (const selection of selections) await prepareRulesWorktree(selection, 'V08');
  if (!baselineReadback) await product.closeNormally();
  if (!baselineReadback) baselineReadback = await rulesReadback('before');
  if (crossProjectOnly) await product.closeNormally();
  await product.launch();
  await product.instances();
  const submitterSession = await openUserSession(...['seatId', 'instanceId', 'worktreeId']
    .map(name => rules.submitter[name]), 'V08 submitter');
  const reviewerSession = await openUserSession(...['seatId', 'instanceId', 'worktreeId']
    .map(name => rules.reviewer[name]), 'V08 reviewer');
  const { runRulesCase } = await import('./m2-rules.mjs');
  const record = await runRulesCase(product, { ...config, rules: { ...rules,
    baselineReadback, submitterSession, reviewerSession,
    hostCheckpoint: async () => {
      await product.closeNormally();
      const reference = await rulesReadback('checkpoint');
      await product.launch();
      await product.instances();
      return reference;
    },
    openHostSession: row => openUserSession(row.seatId, row.instanceId, row.worktreeId, 'V08 Host recipient'),
    resumeRulesSession: async session => {
      const originalThread = session.threadId;
      const receipt = await sessionOp(session, 'resume');
      check(receipt.result.state === 'RUNNING' && receipt.result.newGeneration &&
        (!receipt.result.threadId || receipt.result.threadId === originalThread),
      'V08 resume preserves the original native logical thread');
      session.cursor = '0';
      product.save();
    },
    stopRulesSession: session => stopUser(session, false),
    releaseStoppedRulesSession: session => sessionOp(session, 'admission-release', { seatId: session.seatId }),
  } }, journal);
  check(record.state === 'FLOW_COMPLETE_DIRECT_LEDGER_READBACK_REQUIRED',
    'V08 original flow completed and requires independent readback review');
  await stopUser(submitterSession, true);
  await stopUser(reviewerSession, true);
  await product.closeNormally();
  await snapshot('after');
  await compareSnapshots();
  await rulesReadback('final');
  journal.v08 = 'IMPLEMENTED_CASES_HAVE_DIRECT_EVIDENCE_V08_INCOMPLETE';
  journal.state = 'RULES_FLOW_COMPLETE_DIRECT_READBACK_REVIEW_REQUIRED';
  product.save();
}

try {
  await snapshot('before');
  await product.launch();
  const instances = await product.instances();
  check(instances.instances.some(row => row.instanceId === config.instanceId &&
    row.driverId === 'codex' && row.state === 'LOGGED_IN'),
  'Original admitted Codex instance is logged in');
  check(journal.v12 === 'NOT_RUN_ORIGINAL_M2_SIDE_WORKTREE_READBACK_REQUIRED',
    'V12 remains NOT_RUN until its original M2 F readback exists');
  await runRules();
} catch (error) {
  journal.state = 'FAIL_ORIGINAL_REQUESTS_RETAINED';
  journal.error = String(error?.stack ?? error);
  if (journal.v08 === 'RUNNING') journal.v08 = 'FAIL_ORIGINAL_CASE_RETAINED';
  journal.currentEndpoint = product.endpoint ?? null;
  product.save();
  process.exitCode = 1;
  try { await product.preserveFailure(); }
  catch (preserveError) {
    journal.preserveError = String(preserveError?.stack ?? preserveError);
    product.save();
  }
  // Formal objects are outside the active candidate. Observe them even when
  // the original model/stop failed; never close or replay that request here.
  try {
    await snapshot('failure-after', ['formal']);
    const before = snapshotValue('formal', 'before'), after = snapshotValue('formal', 'failure-after');
    for (const field of formalFields) check(Object.hasOwn(before, field) && Object.hasOwn(after, field) &&
      JSON.stringify(before[field]) === JSON.stringify(after[field]), `failure: formal unchanged ${field}`);
    journal.failureFormalProtection = 'FIVE_GROUPS_UNCHANGED';
  } catch (observerError) {
    journal.failureFormalProtection = 'FAILED_OR_UNAVAILABLE';
    journal.failureFormalObservationError = String(observerError?.stack ?? observerError);
  }
  product.save();
}
