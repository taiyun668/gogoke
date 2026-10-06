// Standalone installed-candidate entry for the M2 stop-worktree cases.
// It opens and stops the original H process without a prompt; no install, login or model call.
import fs from 'node:fs';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { ActualProduct, readJson, sha256, id } from './product-cdp.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const configPath = process.argv[2];
if (!configPath) throw Error('Private M2 stop-worktree config path is required');
const config = readJson(configPath);
const atom = value => typeof value === 'string' && /^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/.test(value);
const inside = (child, parent) => child === parent || child.startsWith(parent + path.sep);
const required = ['installed', 'version', 'sourceCommit', 'installedSha256', 'registryKey',
  'pwsh', 'python', 'evidenceDirectory', 'result', 'stateRoot', 'testbedSource',
  'domainId', 'repositoryId', 'instanceId', 'seatId',
  'stopWorktree'];

if (process.platform !== 'win32' || required.some(key => config[key] === undefined) ||
    !/^\d+\.\d+\.\d+$/.test(config.version) || !/^[a-f0-9]{40}$/.test(config.sourceCommit) ||
    config.repositoryId !== 'gogokeSeatTestbed' || !atom(config.domainId) || !atom(config.instanceId) ||
    !atom(config.seatId) ||
    typeof config.installedSha256 !== 'object' || config.installedSha256 === null ||
    !['gogoke.exe', 'gogoke-native-host.exe', 'resource-index.json']
      .every(name => /^[a-f0-9]{64}$/.test(config.installedSha256[name] ?? '')) ||
    config.stopWorktree?.lifecycleOwnership !== 'EXCLUSIVE_M2_STOP_WORKTREE' ||
    !atom(config.stopWorktree.worktreeId) || !atom(config.stopWorktree.seatId) ||
    config.stopWorktree.seatId === config.seatId ||
    !['installed', 'pwsh', 'python', 'stateRoot', 'testbedSource', 'evidenceDirectory', 'result']
      .every(key => typeof config[key] === 'string') ||
    !path.isAbsolute(config.installed) || !path.isAbsolute(config.stateRoot) ||
    !path.isAbsolute(config.pwsh) || !path.isAbsolute(config.python) ||
    !path.isAbsolute(config.testbedSource) || !path.isAbsolute(config.evidenceDirectory) ||
    !path.isAbsolute(config.result) || !fs.existsSync(config.evidenceDirectory) ||
    !fs.existsSync(config.testbedSource) || fs.existsSync(config.result) ||
    !inside(path.resolve(config.result), path.resolve(config.evidenceDirectory)) ||
    path.dirname(path.resolve(config.result)) !== path.resolve(config.evidenceDirectory) ||
    [config.installed, config.stateRoot, config.testbedSource, path.resolve(here, '..', '..')]
      .some(root => inside(path.resolve(config.evidenceDirectory), path.resolve(root))) ||
    inside(path.resolve(config.stateRoot), path.resolve(config.evidenceDirectory)) ||
    path.resolve(config.stateRoot) === path.resolve(config.testbedSource) ||
    config.testerArmy === false) {
  throw Error('Fresh private M2 stop-worktree run requires an exact installed candidate, isolated evidence and a real User bridge');
}

const journal = { schema: 'gogoke.37.m2-stop-worktree-win11.v1', caseId: id('m2StopWorktree'),
  sourceCommit: config.sourceCommit, domainId: config.domainId, repositoryId: config.repositoryId,
  stateRoot: path.resolve(config.stateRoot), evidenceDirectory: path.resolve(config.evidenceDirectory),
  candidateVersion: config.version, candidateInstalledSha256: config.installedSha256,
  state: 'RUNNING', acceptance: false, operations: [], launches: [], closes: [], readbacks: [], sessions: [] };
const product = new ActualProduct(config, journal);
journal.driverBytes['m2-stop-worktree-win11.mjs'] = sha256(path.join(here, 'm2-stop-worktree-win11.mjs'));
const check = (condition, reason) => { if (!condition) throw Error(reason); };

async function normalCloseReadbackRestart(phase) {
  check(phase === 'stopped' || phase === 'final', 'Unexpected stop-worktree readback phase');
  await product.closeNormally();
  const file = `m2-stop-worktree-${phase}-readback.json`;
  const output = path.join(config.evidenceDirectory, file);
  check(!fs.existsSync(output), `Original ${phase} readback output already exists`);
  await new Promise((resolve, reject) => {
    const child = spawn(config.python, [path.join(here, 'm2-stop-worktree-readback.py'),
      config.stateRoot, output, config.result, phase],
      { windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
    let stdout = '', stderr = '';
    child.stdout.on('data', bytes => { stdout = (stdout + bytes).slice(-8192); });
    child.stderr.on('data', bytes => { stderr = (stderr + bytes).slice(-8192); });
    child.once('error', reject);
    child.once('exit', code => code === 0 ? resolve() :
      reject(Error(`Original stop-worktree ${phase} readback exit=${code}; stdout=${stdout}; stderr=${stderr}`)));
  });
  const proof = readJson(output);
  const directCaseEvidence = phase === 'final';
  check(proof.schema === 'gogoke.37.private-m2-stop-worktree-readback.v1' &&
    proof.phase === phase && proof.caseId === journal.caseId &&
    proof.sourceCommit === config.sourceCommit && proof.domainId === config.domainId &&
    proof.stateRoot === journal.stateRoot && proof.evidenceDirectory === journal.evidenceDirectory &&
    proof.readerSha256 === sha256(path.join(here, 'm2-stop-worktree-readback.py')) &&
    proof.normalClose?.exitCode === 0 && proof.normalClose.forceKill === false &&
    proof.measurementPreservedDatabaseBytes === true && proof.acceptance === false &&
    proof.directCaseEvidence === directCaseEvidence &&
    JSON.stringify(proof.candidateInstalledSha256) === JSON.stringify(config.installedSha256),
  `Original ${phase} immutable readback is not bound to this normally closed candidate`);
  const reference = { file, sha256: sha256(output) };
  journal.readbacks.push({ phase, ...reference, acceptance: false }); product.save();
  await product.launch();
  return reference;
}

try {
  const { runStopWorktreeCase } = await import('./m2-stop-worktree.mjs');
  const result = await (async () => {
    await product.launch();
    return runStopWorktreeCase(product, { ...config, stopWorktree: {
      ...config.stopWorktree, normalCloseReadbackRestart,
    } }, journal);
  })();
  check(result.acceptance === false && journal.stopWorktree?.acceptance === false,
    'Stop-worktree evidence cannot declare acceptance');
  await product.closeNormally();
  journal.state = result.state === 'FLOW_COMPLETE_REVIEW_REQUIRED'
    ? result.state : 'NOT_RUN';
  journal.notRun = result.notRun ?? [];
  product.save();
} catch (error) {
  journal.state = 'FAIL_ORIGINAL_REQUESTS_RETAINED';
  journal.error = String(error?.stack ?? error);
  product.save();
  process.exitCode = 1;
  try { await product.preserveFailure(); }
  catch (preserveError) { journal.preserveError = String(preserveError?.stack ?? preserveError); product.save(); }
}
