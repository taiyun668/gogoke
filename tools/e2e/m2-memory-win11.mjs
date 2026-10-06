// Independent installed-candidate entry for the existing M2 history boundary flow.
// The original reader reports partial V04b facts; this driver never grants acceptance.
import fs from 'node:fs';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { ActualProduct, id, readJson, sha256 } from './product-cdp.mjs';
import { runHistoryBoundaryCases } from './m2-history-boundaries.mjs';

process.env.E2E_TELEMETRY_DISABLED = '1';
const here = path.dirname(fileURLToPath(import.meta.url));
const config = readJson(process.argv[2]);
const atom = value => typeof value === 'string' && /^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/.test(value);
const inside = (child, parent) => child === parent || child.startsWith(parent + path.sep);
const check = (condition, reason) => { if (!condition) throw Error(reason); };
const cases = config.historyBoundary?.cases;
const required = ['installed', 'installedSha256', 'version', 'sourceCommit', 'registryKey',
  'pwsh', 'python', 'stateRoot', 'evidenceDirectory', 'result', 'domainId', 'repositoryId', 'observers'];
check(process.platform === 'win32' && process.argv[2] && required.every(key => config[key] !== undefined) &&
  config.testerArmy !== false && config.repositoryId === 'gogokeSeatTestbed' && atom(config.domainId) &&
  /^[a-f0-9]{40}$/.test(config.sourceCommit) && Array.isArray(cases) && cases.length >= 1 && cases.length <= 2 &&
  new Set(cases.map(row => row.driverId)).size === cases.length &&
  cases.every(row => ['codex', 'claude'].includes(row.driverId) && row.projectA?.domainId === config.domainId &&
    row.projectB?.domainId !== config.domainId && row.sideBinding?.domainId === config.domainId) &&
  config.historyBoundary.peerRead !== true &&
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
  'Fresh private installed M2 memory fixture, exclusive F bindings and observers required');

const journal = { schema: 'gogoke.37.m2-win11-e2e.v1', entry: 'm2-memory-win11',
  caseId: id('m2Memory'), sourceCommit: config.sourceCommit, domainId: config.domainId,
  repositoryId: config.repositoryId, state: 'RUNNING', acceptance: false, authenticationActions: false,
  credentialReads: false, launches: [], closes: [], operations: [], sessions: [], snapshots: {}, readbacks: [],
  notRun: ['codex', 'claude', 'opencode', 'grok', 'antigravity']
    .filter(driverId => !cases.some(row => row.driverId === driverId))
    .map(driverId => ({ driverId, state: 'NOT_RUN_NO_CASE' })) };
const product = new ActualProduct(config, journal);
delete journal.driverBytes['m1-win11.mjs'];
delete journal.driverBytes['m1-readback.py'];
for (const file of ['m2-memory-win11.mjs', 'm2-history-boundaries.mjs',
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
    const file = path.join(config.evidenceDirectory, `${observer.name}-m2-memory-${phase}.json`);
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
  const file = `m2-memory-history-${phase}.json`;
  const output = path.join(config.evidenceDirectory, file);
  check(!fs.existsSync(output), `Existing ${phase} immutable readback`);
  await runChild(config.python, [path.join(here, 'm2-history-boundaries-readback.py'),
    config.stateRoot, output, config.result, phase], `Original ${phase} history readback`);
  const proof = readJson(output);
  check(proof.caseId === journal.caseId && proof.sourceCommit === config.sourceCommit &&
    proof.readerSha256 === journal.driverBytes['m2-history-boundaries-readback.py'] &&
    proof.measurementPreservedDatabaseBytes === true && proof.directFlowEvidence === true &&
    proof.acceptance === false && proof.databaseWrites === false && proof.credentialReads === false &&
    (phase !== 'final' || proof.directRefusalEvidence === true),
  `${phase}: original H/A/F immutable facts are incomplete`);
  const ref = { phase: `history-${phase}`, file, sha256: sha256(output) };
  journal.readbacks.push(ref); product.save();
  return ref;
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
  journal.state = 'DIRECT_CROSS_PROJECT_FACTS_REVIEW_REQUIRED';
  journal.v04b = 'NOT_RUN_VENDOR_MEMORY_AND_INSTRUCTION_PROVENANCE';
  journal.acceptance = false; product.save();
} catch (error) {
  journal.state = 'FAIL_OR_NOT_RUN_PRESERVE_ORIGINAL';
  journal.originalError = String(error.stack ?? error);
  journal.currentEndpoint = product.endpoint ?? null; product.save(); process.exitCode = 1;
  try { await product.preserveFailure(); }
  catch (disconnectError) { journal.disconnectError = String(disconnectError); product.save(); }
}
