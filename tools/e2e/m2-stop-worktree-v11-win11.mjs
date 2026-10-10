// Standalone installed-candidate entry for the existing original V11 file cases.
// It does not install, log in, or invent an H/Model result.
import fs from 'node:fs';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { ActualProduct, readJson, sha256, id } from './product-cdp.mjs';
import { runV11FileBoundaries } from './m2-v11-boundaries.mjs';
import { runV11OutsideTree } from './m2-stop-worktree-v11-outside.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const configPath = process.argv[2];
if (!configPath) throw Error('Private V11 file-boundary config path is required');
const config = readJson(configPath);
const atom = value => typeof value === 'string' && /^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/.test(value);
const inside = (child, parent) => child.toUpperCase() === parent.toUpperCase() ||
  child.toUpperCase().startsWith(parent.toUpperCase() + path.sep);
const required = ['installed', 'version', 'sourceCommit', 'installedSha256', 'registryKey',
  'pwsh', 'python', 'evidenceDirectory', 'result', 'stateRoot', 'testbedSource',
  'domainId', 'repositoryId', 'v11FileBoundaries', 'v11OutsideTree'];
const c = config.v11FileBoundaries;
if (process.platform !== 'win32' || required.some(key => config[key] === undefined) ||
    !/^\d+\.\d+\.\d+$/.test(config.version) || !/^[a-f0-9]{40}$/.test(config.sourceCommit) ||
    config.repositoryId !== 'gogokeSeatTestbed' || !atom(config.domainId) ||
    typeof config.installedSha256 !== 'object' || config.installedSha256 === null ||
    !['gogoke.exe', 'gogoke-native-host.exe', 'resource-index.json']
      .every(name => /^[a-f0-9]{64}$/.test(config.installedSha256[name] ?? '')) ||
    c?.ownership !== 'EXCLUSIVE_V11_FILE_BOUNDARIES' ||
    !['mainWrite', 'readOnlyWrite'].every(name => c[name] &&
      ['seatId', 'instanceId', 'worktreeId'].every(key => atom(c[name][key])) &&
      c[name].attemptMode === 'execCommand' && path.isAbsolute(c[name].worktreePath ?? '') &&
      !c[name].worktreePath.startsWith('\\\\?\\')) ||
    c.mainWrite.seatId === c.readOnlyWrite.seatId ||
    c.mainWrite.worktreeId === c.readOnlyWrite.worktreeId ||
    config.v11OutsideTree?.ownership !== 'EXCLUSIVE_V11_OUTSIDE_TREE' ||
    !path.isAbsolute(config.v11OutsideTree.outsideRoot ?? '') ||
    !fs.existsSync(config.v11OutsideTree.outsideRoot) ||
    !fs.statSync(config.v11OutsideTree.outsideRoot).isDirectory() ||
    fs.readdirSync(config.v11OutsideTree.outsideRoot).length !== 0 ||
    !['installed', 'pwsh', 'python', 'stateRoot', 'testbedSource', 'evidenceDirectory', 'result']
      .every(key => typeof config[key] === 'string' && path.isAbsolute(config[key])) ||
    !fs.existsSync(config.evidenceDirectory) || !fs.existsSync(config.testbedSource) ||
    config.testbedSource.startsWith('\\\\?\\') ||
    fs.existsSync(config.result) ||
    !inside(path.resolve(config.result), path.resolve(config.evidenceDirectory)) ||
    path.dirname(path.resolve(config.result)) !== path.resolve(config.evidenceDirectory) ||
    [config.installed, config.stateRoot, config.testbedSource, path.resolve(here, '..', '..')]
      .some(root => inside(path.resolve(config.evidenceDirectory), path.resolve(root))) ||
    inside(path.resolve(config.stateRoot), path.resolve(config.evidenceDirectory)) ||
    path.resolve(config.stateRoot) === path.resolve(config.testbedSource) || config.testerArmy === false) {
  throw Error('V11 needs two exclusive registered F trees, exact installed bytes and a fresh private evidence directory');
}

const journal = { schema: 'gogoke.37.m2-v11-file-boundaries-win11.v1',
  caseId: id('m2V11File'), sourceCommit: config.sourceCommit,
  domainId: config.domainId, repositoryId: config.repositoryId,
  stateRoot: path.resolve(config.stateRoot), evidenceDirectory: path.resolve(config.evidenceDirectory),
  candidateVersion: config.version, candidateInstalledSha256: config.installedSha256,
  state: 'RUNNING', acceptance: false, operations: [], launches: [], closes: [], readbacks: [], sessions: [] };
const product = new ActualProduct(config, journal);
journal.driverBytes['m2-stop-worktree-v11-win11.mjs'] = sha256(path.join(here, 'm2-stop-worktree-v11-win11.mjs'));
journal.driverBytes['m2-v11-boundaries.mjs'] = sha256(path.join(here, 'm2-v11-boundaries.mjs'));
journal.driverBytes['m2-v11-readback.py'] = sha256(path.join(here, 'm2-v11-readback.py'));
journal.driverBytes['m2-stop-worktree-v11-outside.mjs'] = sha256(path.join(here, 'm2-stop-worktree-v11-outside.mjs'));
journal.driverBytes['m2-stop-worktree-v11-readback.py'] = sha256(path.join(here, 'm2-stop-worktree-v11-readback.py'));
journal.driverBytes['m2-stop-worktree-v11-census.ps1'] = sha256(path.join(here, 'm2-stop-worktree-v11-census.ps1'));
const check = (condition, reason) => { if (!condition) throw Error(reason); };

async function requireClosedCandidate() {
  const original = await new Promise((resolve, reject) => {
    const child = spawn(config.pwsh, ['-NoProfile', '-NonInteractive', '-File',
      path.join(here, 'm2-stop-worktree-v11-census.ps1'), '-Installed', config.installed],
    { windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
    let stdout = '', stderr = '';
    child.stdout.on('data', bytes => { stdout = (stdout + bytes).slice(-8192); });
    child.stderr.on('data', bytes => { stderr = (stderr + bytes).slice(-8192); });
    child.once('error', reject);
    child.once('exit', code => code === 0 ? resolve(stdout.trim()) :
      reject(Error(`V11 closed-candidate census exit=${code}; stderr=${stderr}`)));
  });
  check(original === 'NO_INSTALLED_CANDIDATE_PROCESS',
    'V11 installed-candidate process absence was not directly observed');
}

async function preflightOutside() {
  const file = 'm2-v11-outside-closed-preflight.json';
  const output = path.join(config.evidenceDirectory, file);
  check(!fs.existsSync(output), 'V11 closed preflight output already exists');
  await new Promise((resolve, reject) => {
    const child = spawn(config.python, [path.join(here, 'm2-stop-worktree-v11-readback.py'),
      'preflight', config.stateRoot, output, config.v11OutsideTree.outsideRoot],
      { windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
    let stdout = '', stderr = '';
    child.stdout.on('data', bytes => { stdout = (stdout + bytes).slice(-8192); });
    child.stderr.on('data', bytes => { stderr = (stderr + bytes).slice(-8192); });
    child.once('error', reject);
    child.once('exit', code => code === 0 ? resolve() :
      reject(Error(`V11 closed outside preflight exit=${code}; stdout=${stdout}; stderr=${stderr}`)));
  });
  const proof = readJson(output);
  check(proof.schema === 'gogoke.37.private-v11-outside-preflight.v1' &&
    proof.stateRoot === path.resolve(config.stateRoot) &&
    proof.outsideRoot === path.resolve(config.v11OutsideTree.outsideRoot) &&
    proof.measurementPreservedDatabaseBytes === true && proof.acceptance === false &&
    proof.inventory?.trees?.length > 0 && proof.inventory?.sources?.length > 0,
  'V11 closed F/source object preflight differs from this private run');
  journal.v11OutsidePreflight = { file, sha256: sha256(output) };
  product.save();
}

async function readback() {
  const file = 'm2-v11-file-boundaries-readback.json';
  const output = path.join(config.evidenceDirectory, file);
  check(!fs.existsSync(output), 'V11 readback output already exists');
  await new Promise((resolve, reject) => {
    const child = spawn(config.python, [path.join(here, 'm2-v11-readback.py'),
      config.stateRoot, output, config.result, 'file'],
      { windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
    let stdout = '', stderr = '';
    child.stdout.on('data', bytes => { stdout = (stdout + bytes).slice(-8192); });
    child.stderr.on('data', bytes => { stderr = (stderr + bytes).slice(-8192); });
    child.once('error', reject);
    child.once('exit', code => code === 0 ? resolve() :
      reject(Error(`V11 original file readback exit=${code}; stdout=${stdout}; stderr=${stderr}`)));
  });
  const proof = readJson(output);
  check(proof.schema === 'gogoke.37.private-v11-file-boundaries-readback.v1' &&
    proof.sourceCommit === config.sourceCommit &&
    proof.normalClosePid === journal.closes.at(-1)?.pid &&
    proof.measurementPreservedDatabaseBytes === true && proof.acceptance === false &&
    proof.directCaseEvidence === false && Array.isArray(proof.cases) && proof.cases.length === 2,
  'V11 immutable readback is not bound to the original normal close');
  journal.readbacks.push({ phase: 'file', file, sha256: sha256(output), acceptance: false });
  product.save();
  return proof;
}

async function readbackOutside() {
  const file = 'm2-v11-outside-readback.json';
  const output = path.join(config.evidenceDirectory, file);
  check(!fs.existsSync(output), 'V11 outside readback output already exists');
  await new Promise((resolve, reject) => {
    const child = spawn(config.python, [path.join(here, 'm2-stop-worktree-v11-readback.py'),
      config.stateRoot, output, config.result],
      { windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
    let stdout = '', stderr = '';
    child.stdout.on('data', bytes => { stdout = (stdout + bytes).slice(-8192); });
    child.stderr.on('data', bytes => { stderr = (stderr + bytes).slice(-8192); });
    child.once('error', reject);
    child.once('exit', code => code === 0 ? resolve() :
      reject(Error(`V11 original outside readback exit=${code}; stdout=${stdout}; stderr=${stderr}`)));
  });
  const proof = readJson(output);
  check(proof.schema === 'gogoke.37.private-v11-outside-readback.v1' &&
    proof.sourceCommit === config.sourceCommit &&
    proof.normalClosePid === journal.closes.at(-1)?.pid &&
    proof.measurementPreservedDatabaseBytes === true && proof.acceptance === false &&
    proof.directCaseEvidence === false && proof.permissionCause === 'UNATTRIBUTED' &&
    proof.sessionId === journal.v11OutsideTree.sessionId &&
    proof.outsideTarget === journal.v11OutsideTree.target,
  'V11 outside reader did not bind the original H/A/F target and normal close');
  journal.readbacks.push({ phase: 'outside', file, sha256: sha256(output), acceptance: false });
  product.save();
  return proof;
}

try {
  // The original custody helper checks registration; this separate process
  // census checks no exact installed candidate is live before immutable IO.
  await product.custody(true); product.verifyBytes();
  await requireClosedCandidate();
  await preflightOutside();
  await requireClosedCandidate();
  await product.launch();
  await product.custody(); product.verifyBytes();
  const ui = await product.evaluate('({url:location.href,home:!!document.querySelector(".home-product-entry"),tauri:!!window.__TAURI_INTERNALS__})');
  check(ui.url === product.endpoint.url && ui.home && ui.tauri,
    'Actual installed Home/User bridge identity missing');
  const instanceIds = [c.mainWrite.instanceId, c.readOnlyWrite.instanceId];
  const instances = await product.instances();
  check(instanceIds.every(instanceId => instances.instances.some(row =>
    row.instanceId === instanceId && row.state === 'LOGGED_IN')),
  'V11 both original test instances must be logged in');
  await runV11FileBoundaries(product, config, journal);
  check(sha256(path.join(config.evidenceDirectory, journal.v11OutsidePreflight.file)) ===
    journal.v11OutsidePreflight.sha256,
  'V11 closed F/source preflight changed before the outside request');
  await runV11OutsideTree(product, { ...config, v11OutsideTree: {
    seatId: c.mainWrite.seatId, instanceId: c.mainWrite.instanceId,
    worktreeId: c.mainWrite.worktreeId, worktreePath: c.mainWrite.worktreePath,
    ownership: config.v11OutsideTree.ownership,
    outsideRoot: config.v11OutsideTree.outsideRoot,
  } }, journal);
  await product.custody(); product.verifyBytes();
  await product.closeNormally();
  const proof = await readback();
  const outsideProof = await readbackOutside();
  journal.state = proof.directAttemptEvidence === true && outsideProof.directAttemptEvidence === true
    ? 'ORIGINAL_TOOL_ATTEMPTS_READ_BACK_CAUSES_REQUIRE_REVIEW'
    : 'NOT_RUN_NO_EXACT_FAILED_ORIGINAL_TOOL';
  journal.notRun = ['V11_NO_NETWORK_EFFECTIVE_BOUNDARY', 'V11_MIXED_TWO_PROJECTS',
    'V11_HOST_SEALED_MERGE'];
  product.save();
} catch (error) {
  journal.state = 'FAIL_ORIGINAL_REQUESTS_RETAINED';
  journal.error = String(error?.stack ?? error);
  product.save();
  process.exitCode = 1;
  try { await product.preserveFailure(); }
  catch (preserveError) { journal.preserveError = String(preserveError?.stack ?? preserveError); product.save(); }
}
