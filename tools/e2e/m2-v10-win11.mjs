// Independent installed-candidate V10 entry using original H/A/D/F and tool facts.
// The immutable reader, not this driver, decides which physical facts qualify.
import fs from 'node:fs';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { ActualProduct, id, readJson, sha256 } from './product-cdp.mjs';
import { runHistoryBoundaryCases, runHistoryPeerReadCases,
  runHistorySameDomainWorkerReadCase } from './m2-history-boundaries.mjs';

process.env.E2E_TELEMETRY_DISABLED = '1';
const here = path.dirname(fileURLToPath(import.meta.url));
const config = readJson(process.argv[2]);
const atom = value => typeof value === 'string' && /^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/.test(value);
const inside = (child, parent) => child === parent || child.startsWith(parent + path.sep);
const check = (condition, reason) => { if (!condition) throw Error(reason); };
const cases = config.historyBoundary?.cases;
const sameDomain = config.historyBoundary?.sameDomainWorkerRead;
const required = ['installed', 'installedSha256', 'version', 'sourceCommit', 'registryKey',
  'pwsh', 'python', 'stateRoot', 'evidenceDirectory', 'result', 'domainId', 'repositoryId', 'observers'];
check(process.platform === 'win32' && process.argv[2] && required.every(key => config[key] !== undefined) &&
  config.testerArmy !== false && config.repositoryId === 'gogokeSeatTestbed' && atom(config.domainId) &&
  config.domainId !== 'global' &&
  /^[a-f0-9]{40}$/.test(config.sourceCommit) && Array.isArray(cases) && cases.length >= 1 && cases.length <= 2 &&
  new Set(cases.map(row => row.driverId)).size === cases.length &&
  cases.every(row => ['codex', 'claude'].includes(row.driverId) && row.projectA?.domainId === config.domainId &&
    row.projectB?.domainId !== config.domainId && row.projectB?.domainId !== 'global' &&
    row.sideBinding?.domainId === config.domainId) &&
  (!sameDomain || (['sourceSeatId', 'sourceIncarnation', 'workerSeatId', 'workerIncarnation']
    .every(key => atom(sameDomain[key])) &&
    cases.some(row => row.driverId === 'codex' &&
      row.projectA.seatId === sameDomain.sourceSeatId &&
      row.sideBinding.seatId === sameDomain.workerSeatId &&
      sameDomain.sourceSeatId !== sameDomain.workerSeatId))) &&
  config.historyBoundary.peerRead === true &&
  config.historyBoundary.lifecycleOwnership === 'EXCLUSIVE_M2_HISTORY_SEATS' &&
  Array.isArray(config.observers) && ['formal', 'memory', 'ledger'].every(name =>
    config.observers.some(row => row.name === name)) &&
  config.observers.every(row => atom(row.name) && typeof row.runtime === 'string' &&
    Array.isArray(row.args) && row.args.includes('{output}') && Array.isArray(row.equalFields) &&
    row.equalFields.length > 0) &&
  ['formal', 'registeredFormal', 'formalData', 'formalRegistry', 'shortcuts'].every(field =>
    config.observers.find(row => row.name === 'formal').equalFields.includes(field)) &&
  !fs.existsSync(config.result) && fs.existsSync(config.evidenceDirectory) &&
  [config.installed, config.stateRoot, config.evidenceDirectory, config.result].every(path.isAbsolute) &&
  inside(path.resolve(config.result), path.resolve(config.evidenceDirectory)) &&
  [config.installed, config.stateRoot, path.resolve(here, '..', '..')].every(root =>
    !inside(path.resolve(config.evidenceDirectory), path.resolve(root))),
  'Fresh private installed V10 fixture, exclusive F bindings and observers required');

const journal = { schema: 'gogoke.37.m2-win11-e2e.v1', entry: 'm2-v10-win11',
  caseId: id('m2V10'), sourceCommit: config.sourceCommit, domainId: config.domainId,
  repositoryId: config.repositoryId, state: 'RUNNING', acceptance: false, authenticationActions: false,
  credentialReads: false, launches: [], closes: [], operations: [], sessions: [], snapshots: {}, readbacks: [],
  v10: 'NOT_RUN', notRun: ['codex', 'claude', 'opencode', 'grok', 'antigravity']
    .filter(driverId => !cases.some(row => row.driverId === driverId))
    .map(driverId => ({ driverId, state: 'NOT_RUN_NO_CASE' })) };
const product = new ActualProduct(config, journal);
delete journal.driverBytes['m1-win11.mjs'];
delete journal.driverBytes['m1-readback.py'];
for (const file of ['m2-v10-win11.mjs', 'm2-history-boundaries.mjs',
  'm2-history-boundaries-readback.py']) journal.driverBytes[file] = sha256(path.join(here, file));

async function runChild(runtime, args, label) {
  await new Promise((resolve, reject) => {
    const child = spawn(runtime, args, { windowsHide: true, stdio: ['ignore', 'ignore', 'pipe'] });
    let error = '';
    child.stderr.on('data', bytes => { error = (error + bytes).slice(-8192); });
    child.once('error', reject);
    child.once('exit', code => code === 0 ? resolve() : reject(Error(`${label} exit=${code}: ${error}`)));
  });
}
async function snapshot(phase) {
  for (const observer of config.observers) {
    const file = path.join(config.evidenceDirectory, `${observer.name}-m2-v10-${phase}.json`);
    check(!fs.existsSync(file), `Existing ${observer.name} ${phase} snapshot`);
    await runChild(observer.runtime, observer.args.map(arg => arg === '{output}' ? file : arg),
      `${observer.name} ${phase} readonly observer`);
    journal.snapshots[`${observer.name}-${phase}`] = { file: path.basename(file), sha256: sha256(file) };
    product.save();
  }
}
function snapshotValue(name, phase) {
  const ref = journal.snapshots[`${name}-${phase}`];
  const file = path.join(config.evidenceDirectory, ref.file);
  check(sha256(file) === ref.sha256, `Original ${name} ${phase} bytes changed`);
  return readJson(file);
}
async function historyReadback(phase) {
  const file = `m2-v10-history-${phase}.json`;
  const output = path.join(config.evidenceDirectory, file);
  check(!fs.existsSync(output), `Existing ${phase} immutable readback`);
  await runChild(config.python, [path.join(here, 'm2-history-boundaries-readback.py'),
    config.stateRoot, output, config.result, phase], `Original ${phase} history readback`);
  const proof = readJson(output);
  check(proof.caseId === journal.caseId && proof.sourceCommit === config.sourceCommit &&
    proof.readerSha256 === journal.driverBytes['m2-history-boundaries-readback.py'] &&
    proof.measurementPreservedDatabaseBytes === true && proof.directFlowEvidence === true &&
    proof.acceptance === false && proof.databaseWrites === false && proof.credentialReads === false &&
    (phase !== 'final' || proof.directRefusalEvidence === true) &&
    (!['peer-final', 'same-domain-final'].includes(phase) ||
      typeof proof.directPeerReadEvidence === 'boolean') &&
    (phase !== 'same-domain-final' ||
      typeof proof.sameDomainWorkerRead?.directDeniedRead === 'boolean'),
  `${phase}: original H/A/F immutable facts are incomplete`);
  const ref = { phase: `history-${phase}`, file, sha256: sha256(output) };
  journal.readbacks.push(ref); product.save();
  return ref;
}

async function projectGlobalDenial() {
  // Use a fresh, live original H WORK reader: a released or nonexistent reader
  // would also be denied and would not measure the project/global boundary.
  const source = journal.historyBoundary.cases.find(row =>
    row.state === 'FLOW_COMPLETE_DIRECT_READBACK_REQUIRED');
  check(source?.projectSessions?.[0] && source.projectA.domainId !== 'global',
    'Original project WORK session required for global-scope refusal');
  const read = async (family, verb, target) => {
    let reply = await product.operation(family, verb, target, {}, '0', ['APPLIED', 'STALE']);
    if (reply.status === 'STALE') reply = await product.operation(family, verb, target, {}, reply.revision);
    return reply;
  };
  const binding = source.projectA;
  const card = await read('K-SEAT', 'state-card', binding.seatId);
  const graph = await read('K-WORKTREE', 'graph-query', binding.worktreeId);
  check(card.result.state === 'IDLE' && card.result.instanceId === source.instanceId &&
    graph.result.state === 'REGISTERED' && graph.result.members?.some(member =>
      ['domainId', 'repositoryId', 'seatId', 'worktreeId'].every(key => member[key] === binding[key]) &&
      member.instanceId === source.instanceId),
  'Original project reader F/E binding is not idle and registered');
  const session = { id: id('v10ProjectReader'), domainId: binding.domainId,
    seatId: binding.seatId, instanceId: source.instanceId,
    worktreeId: binding.worktreeId, generation: (BigInt(card.result.generation) + 1n).toString(),
    revision: '0', purpose: 'WORK', caseOwner: source.caseId };
  journal.sessions.push(session); journal.v10GlobalProbe = { sessionId: session.id,
    state: 'RUNNING', acceptance: false }; product.save();
  const step = async (verb, payload = {}) => {
    const reply = await product.operation('K-SESSION', verb, session.id,
      { generation: session.generation, ...payload }, session.revision);
    session.revision = reply.revision; product.save(); return reply;
  };
  await step('admission-reserve', { seatId: session.seatId });
  await step('admission-commit', { seatId: session.seatId });
  await step('open', { seatId: session.seatId, repositoryId: binding.repositoryId,
    worktreeId: session.worktreeId });
  const pin = await step('capability-probe');
  check(pin.result.driverId === source.driverId && pin.result.version === source.version &&
    pin.result.binaryDigest === `sha256:${source.sha256}`,
  'Original live project reader fixed CLI pin differs');
  const ledger = async (scope, revision, allowed) => {
    const req = { schema: 'gogoke.37.operations.v1', family: 'K-LEDGER',
      operation: 'scoped-query', requestId: id('v10LedgerScope'),
      domainId: binding.domainId, targetId: 'ledger', expectedRevision: revision,
      payload: { readerSessionId: session.id, scope, epoch: 'unselected', afterCursor: '0' } };
    const entry = { request: req, rawFrame: JSON.stringify(req),
      startedAt: new Date().toISOString(), receipt: null };
    journal.operations.push(entry); product.save();
    try {
      const raw = await product.evaluate(`window.__TAURI_INTERNALS__.invoke('gogoke_design37_user_operation',{frame:${JSON.stringify(entry.rawFrame)}})`);
      entry.rawReceipt = raw; entry.receipt = JSON.parse(raw); product.save();
    } catch (error) { entry.originalError = String(error.stack ?? error); product.save(); throw error; }
    const reply = entry.receipt;
    check(reply.schema === req.schema && reply.family === req.family &&
      reply.operation === req.operation && reply.requestId === req.requestId &&
      reply.targetId === req.targetId && allowed.includes(reply.status),
    `Original live project ${scope} reader returned ${reply.status}; preserve receipt`);
    return reply;
  };
  // Existing history events make the global A cursor nonzero. This STALE is
  // reached only after the current original WORK reader passes native custody.
  const project = await ledger('PROJECT', '0', ['STALE']);
  check(BigInt(project.revision) > 0n && project.previousRevision === project.revision,
    'Live project reader positive control did not reach original A cursor');
  const global = await ledger('GLOBAL', project.revision, ['DENIED']);
  check(global.previousRevision === project.revision && global.revision === project.revision,
    'Original live project reader acquired global ledger or changed revision');
  const stopped = await step('stop', { seatId: session.seatId });
  check(typeof stopped.result.stopFact === 'string' && stopped.result.stopFact,
    'Original project reader physical stop fact missing');
  session.stopFact = stopped.result.stopFact;
  await step('admission-release', { seatId: session.seatId });
  journal.v10ProjectGlobalUserRoute = 'ORIGINAL_PROJECT_SCOPE_DENIED_REVIEW_REQUIRED';
  journal.v10GlobalProbe.state = 'ORIGINAL_LIVE_PROJECT_CONTROL_AND_GLOBAL_DENIAL';
  product.save();
}

try {
  await snapshot('before');
  await product.launch();
  const instances = await product.instances();
  check(cases.every(row => instances.instances.some(instance => instance.instanceId === row.instanceId &&
    instance.driverId === row.driverId && instance.version === row.version && instance.state === 'LOGGED_IN')),
  'Every selected fixed instance must already be logged in; no partial case or login');
  const flow = await runHistoryBoundaryCases(product, { ...config, historyBoundary: {
    ...config.historyBoundary,
    normalCloseReadbackRestart: async phase => {
      await product.closeNormally();
      const ref = await historyReadback(phase);
      await product.launch();
      return ref;
    },
  } }, journal);
  check(flow.state === 'FLOW_COMPLETE_DIRECT_READBACK_REQUIRED', 'Original cross-project flow incomplete');
  await product.closeNormally();
  const ref = await historyReadback('final');
  const proof = readJson(path.join(config.evidenceDirectory, ref.file));
  check(proof.cases.length === cases.length &&
    new Set(proof.cases.map(row => row.driverId)).size === cases.length &&
    cases.every(row => proof.cases.some(observed => observed.driverId === row.driverId)) &&
    proof.cases.every(row =>
    row.actualHInputIsolation === true && row.projectDomains.length === 2),
  'Both configured live same-instance project inputs need direct proof');
  await product.launch();
  await projectGlobalDenial();
  const peer = await runHistoryPeerReadCases(product, config, journal);
  check(peer.state === 'PEER_FLOW_COMPLETE_DIRECT_READBACK_REQUIRED',
    'Original V10 peer flow incomplete');
  await product.closeNormally();
  const peerRef = await historyReadback('peer-final');
  const peerProof = readJson(path.join(config.evidenceDirectory, peerRef.file));
  check(peerProof.phase === 'peer-final' && peerProof.directFlowEvidence === true &&
    typeof peerProof.directPeerReadEvidence === 'boolean',
  'Known prior test history has no original H/A/F readback');
  let sameDomainProof = null;
  if (sameDomain) {
    await product.launch();
    const worker = await runHistorySameDomainWorkerReadCase(product, config, journal);
    check(worker.state === 'ORIGINAL_SAME_DOMAIN_WORKER_READ_REQUIRES_NORMAL_CLOSE',
      'Same-domain original worker flow incomplete');
    await product.closeNormally();
    const ref = await historyReadback('same-domain-final');
    sameDomainProof = readJson(path.join(config.evidenceDirectory, ref.file));
    check(sameDomainProof.phase === 'same-domain-final' &&
      sameDomainProof.directFlowEvidence === true &&
      sameDomainProof.sameDomainWorkerRead?.state,
    'Same-domain original worker source/tool readback missing');
  }
  await snapshot('after');
  for (const observer of config.observers) {
    const before = snapshotValue(observer.name, 'before');
    const after = snapshotValue(observer.name, 'after');
    for (const field of observer.equalFields) check(Object.hasOwn(before, field) &&
      Object.hasOwn(after, field) && JSON.stringify(before[field]) === JSON.stringify(after[field]),
    `${observer.name}.${field} changed across original run`);
  }
  for (const phase of ['before', 'after']) {
    const memory = snapshotValue('memory', phase);
    check(memory.memoryDataUnchangedByRead === true && memory.stage1OutputCount === 0 &&
      memory.memoryJobCount === 0, `${phase} product memory observer failed`);
  }
  journal.state = 'DIRECT_V10_FACTS_REVIEW_REQUIRED';
  journal.v10 = 'NOT_RUN_FULL_V10_SCOPE';
  journal.v10PeerFileRead = peerProof.directPeerReadEvidence === true ?
    'DIRECT_CODEX_TOOL_DENIAL_REVIEW_REQUIRED' : peerProof.peerState;
  journal.v10SameDomainWorkerFileRead = sameDomainProof?.sameDomainWorkerRead?.state ??
    'NOT_RUN_SAME_DOMAIN_WORKER_FIXTURE_NOT_CONFIGURED';
  journal.v10NotRun = ['WORKER_OWNERLEAD_HISTORY_MODEL_QUERY',
    'PROJECT_GLOBAL_STORE_AND_EXPLICIT_CITED_DISTRIBUTION',
    'NON_CODEX_VENDOR_HISTORY_TOOL_RECEIPTS', 'ANTIGRAVITY'];
  journal.acceptance = false; product.save();
} catch (error) {
  journal.state = 'FAIL_OR_NOT_RUN_PRESERVE_ORIGINAL';
  journal.originalError = String(error.stack ?? error);
  journal.currentEndpoint = product.endpoint ?? null; product.save(); process.exitCode = 1;
  try { await product.preserveFailure(); }
  catch (disconnectError) { journal.disconnectError = String(disconnectError); product.save(); }
}
