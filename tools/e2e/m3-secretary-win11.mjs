// Installed-candidate V14 slice: original USER read, closed SQLite readback, cold read.
// This entry never configures a secretary, starts a session/model, or reads credentials.
import fs from 'node:fs';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { ActualProduct, id, readJson, sha256 } from './product-cdp.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const configPath = process.argv[2];
if (!configPath) throw Error('Private M3 V14 installed-candidate config path required');
const config = readJson(configPath);
const secret = config.m3Secretary;
const absolute = value => typeof value === 'string' && path.isAbsolute(value);
const inside = (child, parent) => child === parent || child.startsWith(parent + path.sep);
const check = (ok, reason) => { if (!ok) throw Error(reason); };
const same = (left, right) => JSON.stringify(left) === JSON.stringify(right);
const stable = value => value.state === 'UNSET' ? { state: value.state } : {
  state: value.state, seatId: value.seatId, incarnation: value.incarnation,
  generation: value.generation, revision: value.revision, instanceId: value.instanceId,
  model: value.model, effort: value.effort, permissionTier: value.permissionTier,
  seatState: value.seatState,
};
const expected = secret?.expected;
if (process.platform !== 'win32' || !secret || !expected ||
    !['UNSET', 'DESIGNATED'].includes(expected.state) ||
    !['installed', 'pwsh', 'python', 'stateRoot', 'testbedSource', 'evidenceDirectory', 'result']
      .every(key => absolute(config[key])) ||
    !/^\d+\.\d+\.\d+$/.test(config.version ?? '') ||
    !/^[a-f0-9]{40}$/.test(config.sourceCommit ?? '') ||
    !['gogoke.exe', 'gogoke-native-host.exe', 'resource-index.json']
      .every(name => /^[a-f0-9]{64}$/.test(config.installedSha256?.[name] ?? '')) ||
    typeof config.registryKey !== 'string' || !fs.existsSync(config.evidenceDirectory) ||
    !fs.existsSync(config.testbedSource) || fs.existsSync(config.result) ||
    path.dirname(path.resolve(config.result)) !== path.resolve(config.evidenceDirectory) ||
    [config.installed, config.stateRoot, config.testbedSource, path.resolve(here, '..', '..')]
      .some(root => inside(path.resolve(config.evidenceDirectory), path.resolve(root))) ||
    inside(path.resolve(config.stateRoot), path.resolve(config.evidenceDirectory)) ||
    path.resolve(config.stateRoot) === path.resolve(config.testbedSource) ||
    config.testerArmy === false) {
  throw Error('M3 V14 requires exact installed bytes, separate evidence and explicit expected real read state');
}
if (expected.state === 'DESIGNATED' &&
    !['seatId', 'incarnation', 'instanceId', 'model', 'effort', 'permissionTier']
      .every(key => typeof expected[key] === 'string' && expected[key].length > 0)) {
  throw Error('Configured read requires exact expected host-backed test selection');
}

const journal = { schema: 'gogoke.37.private-m3-v14-secretary-read.v1', caseId: id('m3V14'),
  sourceCommit: config.sourceCommit, stateRoot: path.resolve(config.stateRoot),
  evidenceDirectory: path.resolve(config.evidenceDirectory), candidateVersion: config.version,
  candidateInstalledSha256: config.installedSha256, expected, state: 'RUNNING', acceptance: false,
  coverage: ['ORIGINAL_USER_CONFIGURATION_READ', 'CLOSED_ORIGINAL_SQLITE_READ', 'COLD_USER_READ'],
  notRun: ['SECRETARY_DESIGNATE', 'UNSET_TO_CONFIGURED_RESTORE', 'NON_USER_ORIGIN_REFUSAL',
    'GLOBAL_CONVERSATION_MODEL', 'SCHEDULE_DELIVERY_SCOPE_AND_SEARCH', 'V14_ACCEPTANCE'],
  operations: [], launches: [], closes: [], readbacks: [] };
const product = new ActualProduct(config, journal);
journal.driverBytes['m3-secretary-win11.mjs'] = sha256(path.join(here, 'm3-secretary-win11.mjs'));
journal.driverBytes['m3-secretary-readback.py'] = sha256(path.join(here, 'm3-secretary-readback.py'));

async function originalRead(phase) {
  const frame = { schema: 'gogoke.37.owner-configuration.v1', command: 'secretary-configuration-read' };
  const rawFrame = JSON.stringify(frame);
  const entry = { phase, frame, rawFrame, startedAt: new Date().toISOString(), rawReply: null };
  journal.operations.push(entry); product.save();
  try {
    entry.rawReply = await product.evaluate(`window.__TAURI_INTERNALS__.invoke('gogoke_design37_user_operation',{frame:${JSON.stringify(rawFrame)}})`);
  } catch (error) { entry.originalError = String(error?.stack ?? error); product.save(); throw error; }
  entry.reply = JSON.parse(entry.rawReply); entry.finishedAt = new Date().toISOString(); product.save();
  const value = entry.reply;
  check(value?.schema === 'gogoke.37.secretary-configuration.v1' &&
    ['UNSET', 'REVOKED', 'DESIGNATED'].includes(value.state) &&
    value.conversation && ['NONE', 'UNKNOWN', 'CONFLICT', 'FOUND'].includes(value.conversation.state),
  'Original USER secretary read model is incomplete');
  check(value.state === expected.state &&
    (value.state !== 'UNSET' || value.conversation.state === 'NONE'),
  'Actual singleton state differs from the explicitly scoped read case');
  if (value.state === 'DESIGNATED') {
    check(['seatId', 'incarnation', 'instanceId', 'model', 'effort', 'permissionTier']
      .every(key => value[key] === expected[key]) &&
      /^\d+$/.test(value.generation) && /^\d+$/.test(value.revision) &&
      ['IDLE', 'BUSY', 'RECLAIMED'].includes(value.seatState),
    'Original designated E settings differ from the expected test selection');
  }
  check(value.conversation.state === 'NONE' ||
    (value.conversation.state === 'FOUND' && value.conversation.runtimeAvailable === false &&
      value.conversation.stoppedFact === true &&
      ['STOPPED', 'RELEASED'].includes(value.conversation.claimState)),
  'Secretary H is live or unresolved; Controller must settle original H before this read-only close');
  return value;
}

async function closedReadback(phase) {
  await product.closeNormally();
  const file = `m3-secretary-${phase}-readback.json`;
  const output = path.join(config.evidenceDirectory, file);
  check(!fs.existsSync(output), 'Closed readback output already exists');
  await new Promise((resolve, reject) => {
    const child = spawn(config.python, [path.join(here, 'm3-secretary-readback.py'),
      config.stateRoot, output, config.result, phase], { windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
    let stdout = '', stderr = '';
    child.stdout.on('data', bytes => { stdout = (stdout + bytes).slice(-8192); });
    child.stderr.on('data', bytes => { stderr = (stderr + bytes).slice(-8192); });
    child.once('error', reject);
    child.once('exit', code => code === 0 ? resolve() :
      reject(Error(`Closed secretary readback exit=${code}; stdout=${stdout}; stderr=${stderr}`)));
  });
  const proof = readJson(output);
  check(proof.schema === 'gogoke.37.private-m3-v14-secretary-sqlite.v1' &&
    proof.phase === phase && proof.caseId === journal.caseId &&
    proof.sourceCommit === config.sourceCommit && proof.acceptance === false &&
    proof.measurementPreservedDatabaseBytes === true &&
    proof.normalClose?.exitCode === 0 && proof.normalClose.forceKill === false &&
    proof.readerSha256 === journal.driverBytes['m3-secretary-readback.py'] &&
    same(proof.configuration, stable(journal.operations.at(-1).reply)),
  'Immutable secretary readback does not bind the same original USER read');
  journal.readbacks.push({ phase, file, sha256: sha256(output), configuration: proof.configuration });
  product.save();
  return proof;
}

try {
  await product.launch();
  const warm = await originalRead('warm');
  const before = await closedReadback('before-cold');
  await product.launch();
  const cold = await originalRead('cold');
  check(same(stable(warm), stable(cold)), 'Cold original secretary E configuration differs');
  const after = await closedReadback('after-cold');
  check(same(before.configuration, after.configuration), 'Cold persisted E configuration differs');
  journal.state = 'READBACK_COMPLETE_REQUIRES_REVIEW'; product.save();
} catch (error) {
  journal.state = 'FAIL_ORIGINAL_EVIDENCE_RETAINED';
  journal.error = String(error?.stack ?? error); product.save(); process.exitCode = 1;
  try { if (product.child?.exitCode === null) await product.preserveFailure(); }
  catch (preserveError) { journal.preserveError = String(preserveError?.stack ?? preserveError); product.save(); }
}
