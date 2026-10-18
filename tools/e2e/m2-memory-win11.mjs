// Independent installed-candidate entry for the existing M2 history boundary flow.
// The original reader reports partial V04b facts; this driver never grants acceptance.
import fs from 'node:fs';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { ActualProduct, id, readJson, sha256 } from './product-cdp.mjs';
import { runHistoryBoundaryCases, closeProjectOnlyAfterOwnedHome } from './m2-history-boundaries.mjs';
import { createTemporaryHomeLifecycle } from './installed-temporary-home-lifecycle.mjs';
import { closeCausally, causalCloseSources } from './causal-close/causal-close.mjs';
import { validateProtectionObservers, captureProtection,
  compareProtection as compareSharedProtection, readProtectionSnapshot,
  protectionSources } from './installed-protection-observation.mjs';

process.env.E2E_TELEMETRY_DISABLED = '1';
const here = path.dirname(fileURLToPath(import.meta.url));
export function memoizeCaseFinish(homeLifecycle) {
  const finish = homeLifecycle.finishCase.bind(homeLifecycle);
  let caseId;
  let finishPromise;
  return requestedCaseId => {
    if (caseId !== undefined && caseId !== requestedCaseId) {
      return Promise.reject(Error('Temporary-HOME finishCase is scoped to one M2 case'));
    }
    caseId = requestedCaseId;
    return finishPromise ??= Promise.resolve().then(() => finish(requestedCaseId));
  };
}

export async function preserveActorFailure({ error, journal, product, homeLifecycle, caseBegun }) {
  journal.state = 'FAIL_OR_NOT_RUN_PRESERVE_ORIGINAL';
  journal.originalError = String(error.stack ?? error);
  journal.currentEndpoint = product.endpoint ?? null;
  if (homeLifecycle && caseBegun) {
    try {
      journal.projectOnlyTeardown = { result: await homeLifecycle.finishCase(journal.caseId), error: null };
    } catch (teardownError) {
      journal.projectOnlyTeardown = { result: null, error: String(teardownError.stack ?? teardownError) };
    }
  }
  product.save(); process.exitCode = 1;
  try { await product.preserveFailure(); }
  catch (disconnectError) { journal.disconnectError = String(disconnectError); product.save(); }
}

async function main() {
const config = readJson(process.argv[2]);
const atom = value => typeof value === 'string' && /^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/.test(value);
const inside = (child, parent) => child === parent || child.startsWith(parent + path.sep);
const check = (condition, reason) => { if (!condition) throw Error(reason); };
const cases = config.historyBoundary?.cases;
const projectOnly = config.historyBoundary?.selection === 'V04B_PROJECT_ISOLATION';
const required = ['installed', 'installedSha256', 'version', 'sourceCommit', 'registryKey',
  'pwsh', 'python', 'stateRoot', 'evidenceDirectory', 'result', 'domainId', 'repositoryId', 'observers'];
check(process.platform === 'win32' && process.argv[2] && required.every(key => config[key] !== undefined) &&
  config.testerArmy !== false && config.repositoryId === 'gogokeSeatTestbed' && atom(config.domainId) &&
  /^[a-f0-9]{40}$/.test(config.sourceCommit) && Array.isArray(cases) && cases.length >= 1 && cases.length <= 2 &&
  new Set(cases.map(row => row.driverId)).size === cases.length &&
  (projectOnly ? cases.length === 1 : config.historyBoundary?.selection === undefined) &&
  cases.every(row => ['codex', 'claude'].includes(row.driverId) && row.projectA?.domainId === config.domainId &&
    row.projectB?.domainId !== config.domainId &&
    (projectOnly ? row.sideBinding === undefined : row.sideBinding?.domainId === config.domainId)) &&
  (!projectOnly || (config.executionEnabled === true && config.acceptance === false &&
    (config.temporaryHomeDomains === undefined ||
      JSON.stringify(config.temporaryHomeDomains) ===
        JSON.stringify([config.domainId, cases[0].projectB.domainId])) &&
    cases.every(row => [row.projectA, row.projectB].every(binding =>
      binding && ['domainId', 'repositoryId', 'seatId', 'worktreeId'].every(key => atom(binding[key])) &&
      binding.repositoryId === config.repositoryId) &&
      row.projectA.worktreeId !== row.projectB.worktreeId) &&
    config.temporaryHomeReadback &&
    ['path', 'corePath'].every(key => path.isAbsolute(config.temporaryHomeReadback[key] ?? '')) &&
    ['sha256', 'coreSha256'].every(key => /^[a-f0-9]{64}$/.test(config.temporaryHomeReadback[key] ?? '')))) &&
  config.historyBoundary.peerRead !== true &&
  config.historyBoundary.lifecycleOwnership === 'EXCLUSIVE_M2_HISTORY_SEATS' &&
  Array.isArray(config.observers) &&
  !fs.existsSync(config.result) && fs.existsSync(config.evidenceDirectory) &&
  [config.installed, config.stateRoot, config.evidenceDirectory, config.result].every(path.isAbsolute) &&
  inside(path.resolve(config.result), path.resolve(config.evidenceDirectory)) &&
  [config.installed, config.stateRoot, path.resolve(here, '..', '..')].every(root =>
    !inside(path.resolve(config.evidenceDirectory), path.resolve(root))),
  'Fresh private installed M2 memory fixture, exclusive F bindings and observers required');
validateProtectionObservers(config.observers);
if (projectOnly) config.temporaryHomeDomains = [config.domainId, cases[0].projectB.domainId];

const journal = { schema: 'gogoke.37.m2-win11-e2e.v1', entry: 'm2-memory-win11',
  caseId: id('m2Memory'), sourceCommit: config.sourceCommit, domainId: config.domainId,
  repositoryId: config.repositoryId, state: 'RUNNING', acceptance: false, authenticationActions: false,
  credentialReads: false, launches: [], closes: [], operations: [], sessions: [], snapshots: {}, readbacks: [],
  notRun: ['codex', 'claude', 'opencode', 'grok', 'antigravity']
    .filter(driverId => !cases.some(row => row.driverId === driverId))
    .map(driverId => ({ driverId, state: 'NOT_RUN_NO_CASE' })) };
const product = new ActualProduct(config, journal);
if (projectOnly) journal.protectionInstrument = protectionSources();
const homeLifecycle = projectOnly ? createTemporaryHomeLifecycle(product,
  path.join(config.evidenceDirectory, 'temporary-home-attempts.json')) : null;
const finishCase = homeLifecycle ? memoizeCaseFinish(homeLifecycle) : null;
if (homeLifecycle) homeLifecycle.finishCase = finishCase;
let caseBegun = false;
if (projectOnly) journal.causalCloseSources = causalCloseSources();
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
  await captureProtection(config.observers, null,
    name => path.join(config.evidenceDirectory, `${name}-m2-memory-${phase}.json`),
    (name, reference) => { journal.snapshots[`${name}-${phase}`] = reference; product.save(); });
}
function snapshotValue(name, phase) {
  return readProtectionSnapshot(config.evidenceDirectory, journal.snapshots[`${name}-${phase}`]);
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
    (phase !== 'final' || proof.directRefusalEvidence === true) &&
    (phase !== 'project-only-final' || proof.directRefusalEvidence === false),
  `${phase}: original H/A/F immutable facts are incomplete`);
  const ref = { phase: `history-${phase}`, file, sha256: sha256(output) };
  journal.readbacks.push(ref); product.save();
  return ref;
}

try {
  await snapshot('before');
  if (projectOnly) {
    // The case anchor carries the two-domain declaration into the same
    // immutable baseline; no native HOME create has been dispatched yet.
    homeLifecycle.beginCase(journal.caseId);
    caseBegun = true;
    await homeLifecycle.captureBaseline();
  }
  await product.launch();
  // The Claude project-only flow reads its target through the existing Native
  // version/install/login operations in the history core. A different instance's
  // failed recovery must not become a prerequisite for this observation.
  if (!(projectOnly && cases.every(row => row.driverId === 'claude'))) {
    const instances = await product.instances();
    check(cases.every(row => instances.instances.some(instance => instance.instanceId === row.instanceId &&
      instance.driverId === row.driverId && instance.version === row.version && instance.state === 'LOGGED_IN')),
    'Every selected fixed instance must already be logged in; no partial case or login');
  }
  const flow = await runHistoryBoundaryCases(product, { ...config, historyBoundary: {
    ...config.historyBoundary,
    normalCloseReadbackRestart: async phase => {
      await product.closeNormally();
      const ref = await historyReadback(phase);
      await product.launch();
      return ref;
    },
  } }, journal);
  check(flow.state === (projectOnly ? 'PROJECT_ONLY_FLOW_COMPLETE_DIRECT_READBACK_REQUIRED' :
    'FLOW_COMPLETE_DIRECT_READBACK_REQUIRED'), 'Original cross-project flow incomplete');
  if (projectOnly) {
    await closeProjectOnlyAfterOwnedHome(homeLifecycle, product, closeCausally);
  } else {
    await product.closeNormally();
  }
  const ref = await historyReadback(projectOnly ? 'project-only-final' : 'final');
  const proof = readJson(path.join(config.evidenceDirectory, ref.file));
  check(proof.cases.length === cases.length &&
    new Set(proof.cases.map(row => row.driverId)).size === cases.length &&
    cases.every(row => proof.cases.some(observed => observed.driverId === row.driverId)) &&
    proof.cases.every(row =>
    row.actualHInputIsolation === true && row.projectDomains.length === 2),
  'Both configured live same-instance project inputs need direct proof');
  await snapshot('after');
  const protection = compareSharedProtection(config.observers, snapshotValue);
  check(protection.measuredFieldCount === 13 && protection.allEqual === true,
    'Original thirteen protection fields changed across history case');
  journal.protectionComparison = protection; product.save();
  for (const phase of ['before', 'after']) {
    const memory = snapshotValue('memory', phase);
    check(memory.memoryDataUnchangedByRead === true && memory.stage1OutputCount === 0 &&
      memory.memoryJobCount === 0, `${phase} product memory observer failed`);
  }
  journal.state = projectOnly ? 'V04B_PROJECT_INPUT_FACTS_REVIEW_REQUIRED' :
    'DIRECT_CROSS_PROJECT_FACTS_REVIEW_REQUIRED';
  journal.v04b = 'NOT_RUN_VENDOR_MEMORY_AND_INSTRUCTION_PROVENANCE';
  journal.acceptance = false; product.save();
} catch (error) {
  await preserveActorFailure({ error, journal, product, homeLifecycle, caseBegun });
}
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) await main();
