// Independent installed-product entry for the existing M2 V12 side-chat case.
// It does not replay the completed M2 main flow or decide acceptance.
import fs from 'node:fs';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { ActualProduct, id, readJson, sha256 } from './product-cdp.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const configPath = process.argv[2];
if (!configPath) throw Error('Private M2 side-chat config path is required');
const config = readJson(configPath);
const atom = value => typeof value === 'string' && /^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/.test(value);
const inside = (child, parent) => child === parent || child.startsWith(parent + path.sep);
const unique = values => new Set(values).size === values.length;
const required = ['installed', 'version', 'sourceCommit', 'installedSha256', 'registryKey',
  'pwsh', 'python', 'evidenceDirectory', 'result', 'stateRoot', 'testbedSource',
  'domainId', 'repositoryId', 'observers', 'sideChat', 'modelMemoryReader'];
if (process.platform !== 'win32' || required.some(key => config[key] === undefined) ||
    config.repositoryId !== 'gogokeSeatTestbed' || !atom(config.domainId) ||
    !/^[a-f0-9]{40}$/.test(config.sourceCommit) ||
    !/^\d+\.\d+\.\d+$/.test(config.version) ||
    !['gogoke.exe', 'gogoke-native-host.exe', 'resource-index.json'].every(name =>
      /^[a-f0-9]{64}$/.test(config.installedSha256[name] ?? '')) ||
    !Array.isArray(config.observers) || !['formal', 'memory', 'ledger'].every(name =>
      config.observers.some(row => row.name === name)) ||
    config.modelMemoryReader?.runtime !== config.python ||
    !Array.isArray(config.modelMemoryReader?.args) ||
    !['{home}', '{output}'].every(token => config.modelMemoryReader.args.filter(arg => arg === token).length === 1) ||
    !/^[a-f0-9]{64}$/.test(config.modelMemoryReader?.scriptSha256 ?? '') ||
    sha256(config.modelMemoryReader.args[0]) !== config.modelMemoryReader.scriptSha256 ||
    !config.observers.every(row => atom(row.name) && typeof row.runtime === 'string' &&
      path.isAbsolute(row.runtime) && Array.isArray(row.args) && row.args.includes('{output}') &&
      Array.isArray(row.equalFields) && row.equalFields.length > 0) ||
    !config.observers.find(row => row.name === 'formal').equalFields?.includes('formal') ||
    !['formal', 'registeredFormal', 'formalData', 'formalRegistry', 'shortcuts'].every(field =>
      config.observers.find(row => row.name === 'formal').equalFields.includes(field)) ||
    fs.existsSync(config.result) || !fs.existsSync(config.evidenceDirectory) ||
    !['installed', 'pwsh', 'python', 'stateRoot', 'testbedSource', 'evidenceDirectory', 'result']
      .every(key => typeof config[key] === 'string' && path.isAbsolute(config[key])) ||
    !inside(path.resolve(config.result), path.resolve(config.evidenceDirectory)) ||
    path.dirname(path.resolve(config.result)) !== path.resolve(config.evidenceDirectory) ||
    [config.installed, config.stateRoot, config.testbedSource, path.resolve(here, '..', '..')]
      .some(root => inside(path.resolve(config.evidenceDirectory), path.resolve(root))) ||
    path.resolve(config.stateRoot) === path.resolve(config.testbedSource)) {
  throw Error('Fresh private installed V12 entry requires the exact candidate, real testbed, observers and isolated evidence');
}

const side = config.sideChat;
const selections = ['sourceSeatId', 'sourceInstanceId', 'sourceWorktreeId',
  'sideSeatId', 'sideInstanceId', 'sideWorktreeId'];
if (side.lifecycleOwnership !== 'EXCLUSIVE_V12_SOURCE_AND_SIDE' ||
    side.sourceInstanceId !== side.sideInstanceId ||
    selections.some(key => !atom(side[key])) ||
    !atom(side.sourceTemplateId) || !atom(side.sideTemplateId) ||
    !atom(side.sourceIncarnation) ||
    !unique([side.sourceSeatId, side.sideSeatId]) ||
    !unique([side.sourceWorktreeId, side.sideWorktreeId]) ||
    !/^[A-Za-z0-9._:-]{1,256}$/.test(side.ledgerEpoch ?? '') ||
    !/^(0|[1-9][0-9]*)$/.test(side.sourceCursor ?? '') ||
    !['sourceSeatSettings', 'sideSeatSettings'].every(key =>
      typeof side[key]?.model === 'string' && side[key].model.length > 0 &&
      typeof side[key]?.effort === 'string' && side[key].effort.length > 0)) {
  throw Error('V12 requires Root-prepared distinct seats, fresh worktree IDs and measured ledger epoch/cursor');
}

config.result = path.resolve(config.result);
config.evidenceDirectory = path.resolve(config.evidenceDirectory);
const journal = { schema: 'gogoke.37.m2-win11-e2e.v1', entry: 'm2-sidechat-win11',
  caseId: id('m2SideChat'), sourceCommit: config.sourceCommit, domainId: config.domainId,
  repositoryId: config.repositoryId, testbedSource: config.testbedSource,
  candidateVersion: config.version, candidateInstalledSha256: config.installedSha256,
  state: 'RUNNING', acceptance: false, authenticationActions: false, credentialReads: false,
  launches: [], closes: [], operations: [], sessions: [], snapshots: {}, readbacks: [],
  sideChatCases: [], assertions: [], v12: 'RUNNING' };
const product = new ActualProduct(config, journal);
for (const file of ['m2-sidechat-win11.mjs', 'm2-sidechat.mjs', 'm2-readback.py', 'm2-provider-capture-readback.py'])
  journal.driverBytes[file] = sha256(path.join(here, file));
product.save();
const check = (condition, message) => {
  if (!condition) throw Error(message);
  if (!journal.assertions.includes(message)) { journal.assertions.push(message); product.save(); }
};

async function runChild(runtime, args, label) {
  await new Promise((resolve, reject) => {
    const child = spawn(runtime, args, { windowsHide: true, stdio: ['ignore', 'ignore', 'pipe'] });
    let stderr = '';
    child.stderr.on('data', bytes => { stderr = (stderr + bytes).slice(-8192); });
    child.once('error', reject);
    child.once('exit', code => code === 0 ? resolve() :
      reject(Error(`${label} exit=${code}: ${stderr}`)));
  });
}
async function snapshot(phase) {
  for (const observer of config.observers) {
    const output = path.join(config.evidenceDirectory, `${observer.name}-m2-sidechat-${phase}.json`);
    check(!fs.existsSync(output), `Original ${observer.name} ${phase} snapshot is fresh`);
    await runChild(observer.runtime, observer.args.map(arg => arg === '{output}' ? output : arg),
      `${observer.name} read-only observer`);
    journal.snapshots[`${observer.name}-${phase}`] = { file: path.basename(output), sha256: sha256(output) };
    product.save();
  }
}
function snapshotValue(name, phase) {
  const ref = journal.snapshots[`${name}-${phase}`];
  const file = path.join(config.evidenceDirectory, ref.file);
  check(sha256(file) === ref.sha256, `Original ${name} ${phase} snapshot bytes`);
  return readJson(file);
}
async function readback(phase) {
  const file = `m2-sidechat-readback-${phase}.json`;
  const output = path.join(config.evidenceDirectory, file);
  check(!fs.existsSync(output), `Original ${phase} readback is fresh`);
  await runChild(config.python, [path.join(here, 'm2-readback.py'),
    config.stateRoot, output, config.result, phase], `Original ${phase} immutable reader`);
  const proof = readJson(output);
  check(proof.schema === 'gogoke.37.private-m2-readback.v1' && proof.phase === phase &&
    proof.entry === 'm2-sidechat-win11' && proof.caseId === journal.caseId &&
    proof.sourceCommit === config.sourceCommit && proof.databaseWrites === false &&
    proof.credentialReads === false && proof.measurementPreservedDatabaseBytes === true &&
    proof.directCaseEvidence === true, `Original ${phase} immutable direct readback`);
  const ref = { phase, file, sha256: sha256(output) };
  journal.readbacks.push(ref); product.save();
  return proof;
}
async function createRegisteredWorktree(prefix) {
  const seatId = side[`${prefix}SeatId`], instanceId = side[`${prefix}InstanceId`];
  let card = await product.operation('K-SEAT', 'state-card', seatId, {}, '0', ['APPLIED', 'STALE']);
  if (card.status === 'STALE')
    card = await product.operation('K-SEAT', 'state-card', seatId, {}, card.revision);
  check(card.result.state === 'IDLE' && card.result.layer === 'USER' &&
    card.result.templateId === side[`${prefix}TemplateId`] && card.result.instanceId === instanceId,
    `V12 ${prefix} seat is the preconfigured IDLE USER seat from its selected template`);
  const worktreeId = side[`${prefix}WorktreeId`];
  const created = await product.operation('K-WORKTREE', 'create', worktreeId,
    { repositoryId: config.repositoryId, seatId, layout: 'single' }, '0');
  check(created.result.worktreeId === worktreeId && created.result.state === 'CREATED' &&
    created.result.classification === 'SINGLE', `V12 ${prefix} original F create receipt`);
  const registered = await product.operation('K-WORKTREE', 'register', worktreeId, {}, '1');
  check(registered.result.worktreeId === worktreeId, `V12 ${prefix} original F register receipt`);
}

try {
  await snapshot('before');
  const initialMemory = snapshotValue('memory', 'before');
  check(initialMemory.memoryDataUnchangedByRead && initialMemory.stage1OutputCount === 0 &&
    initialMemory.memoryJobCount === 0, 'Registration HOME memory observation is unchanged; actual model HOME is checked separately');
  const initialLedger = snapshotValue('ledger', 'before');
  check(initialLedger.epoch === side.ledgerEpoch && initialLedger.cursor === side.sourceCursor,
    'Root-provided V12 epoch and source cursor match the fresh native read-only ledger observation');
  await product.launch();
  const instances = await product.instances();
  for (const instanceId of new Set([side.sourceInstanceId, side.sideInstanceId])) {
    check(instances.instances.some(row => row.instanceId === instanceId && row.driverId === 'codex' &&
      row.state === 'LOGGED_IN'), 'V12 selected Codex instances are already admitted and logged in');
  }
  for (const prefix of ['source', 'side']) {
    const row = side[`${prefix}SeatSettings`];
    let card = await product.operation('K-SEAT', 'state-card', side[`${prefix}SeatId`], {}, '0', ['APPLIED', 'STALE']);
    if (card.status === 'STALE')
      card = await product.operation('K-SEAT', 'state-card', side[`${prefix}SeatId`], {}, card.revision);
    check(card.result.state === 'IDLE' && card.result.layer === 'USER' &&
      card.result.templateId === side[`${prefix}TemplateId`] &&
      card.result.instanceId === side[`${prefix}InstanceId`] &&
      card.result.settings?.model === row.model && card.result.settings?.effort === row.effort,
      `V12 ${prefix} USER template seat model and effort match Root-provided settings`);
  }
  journal.sideChatPlan = { ...Object.fromEntries(selections.map(key => [key, side[key]])),
    lifecycleOwnership: side.lifecycleOwnership, ledgerEpoch: side.ledgerEpoch,
    sourceCursor: side.sourceCursor, sourceSeatSettings: side.sourceSeatSettings,
    sideSeatSettings: side.sideSeatSettings, sourceTemplateId: side.sourceTemplateId,
    sideTemplateId: side.sideTemplateId, sourceIncarnation: side.sourceIncarnation }; product.save();
  const leadFrame = { schema: 'gogoke.37.owner-configuration.v1', command: 'seat-designate-lead',
    domainId: config.domainId, seatId: side.sourceSeatId, incarnation: side.sourceIncarnation };
  const designation = { kind: 'V12_SOURCE_LEAD_DESIGNATION', request: leadFrame,
    rawFrame: JSON.stringify(leadFrame), receipt: null };
  journal.operations.push(designation); product.save();
  const leadReply = await product.evaluate(`window.__TAURI_INTERNALS__.invoke('gogoke_design37_user_operation',{frame:${JSON.stringify(designation.rawFrame)}})`);
  designation.receipt = JSON.parse(leadReply); product.save();
  await createRegisteredWorktree('source');
  await createRegisteredWorktree('side');
  await product.closeNormally();
  const original = await readback('side-worktrees');
  check(original.epoch === initialLedger.epoch &&
    BigInt(original.cursor) >= BigInt(initialLedger.cursor) && original.worktrees?.length === 2 &&
    original.freshModelHistoriesAbsentBeforeOpen === true,
    'V12 original F readback binds both roots and the same measured ledger epoch');
  const sourceTree = original.worktrees.find(row => row.worktreeId === side.sourceWorktreeId);
  const sideTree = original.worktrees.find(row => row.worktreeId === side.sideWorktreeId);
  check(Boolean(sourceTree && sideTree), 'V12 F logical IDs resolve to two actual roots');
  side.sideWorktreeRoot = sideTree.path;
  await product.launch();
  check(Boolean(product.tester) && journal.connectionBackend?.agentActs === 0,
    'V12 restart retains the installed-product hard locator and no agent actions');
  const readbackRef = journal.readbacks.at(-1);
  const prepared = { ...side, sourceWorktreeRoot: sourceTree.path,
    sideWorktreeRoot: sideTree.path,
    worktreeReadback: { file: readbackRef.file, sha256: readbackRef.sha256 },
    worktrees: original.worktrees, ledgerEpoch: initialLedger.epoch,
    sourceCursor: initialLedger.cursor, restartProduct: async () => {
      await product.closeNormally(); await product.launch(); await product.instances();
      check(Boolean(product.tester) && journal.connectionBackend?.agentActs === 0,
        'V12 restart retains the installed-product hard locator and no agent actions');
    } };
  const { runSideChatCase } = await import('./m2-sidechat.mjs');
  const result = await runSideChatCase(product, { ...config, sideChat: prepared }, journal);
  check(result.state === 'FLOW_COMPLETE_DIRECT_LEDGER_READBACK_REQUIRED',
    'Original V12 module completed and requires final direct readback');
  journal.v12 = result.state; product.save();
  await product.closeNormally();
  const final = await readback('final');
  check(final.sideChatCases?.length === 1 &&
    final.sideChatCases[0].result === 'DIRECT_A_H_F_AND_AUTHORIZED_TOOL_EXPORTED_REQUIRES_V12_REVIEW',
    'Existing immutable reader confirms actual V12 A/H/F and authorized tool evidence');
  check(final.actualModelMemoryHomes?.length === 2 &&
    new Set(final.actualModelMemoryHomes.map(row => row.path)).size === 2,
    'Two real H/F-bound model CODEX_HOME directories are observed');
  journal.actualModelMemory = [];
  for (const [index, home] of final.actualModelMemoryHomes.entries()) {
    check(home.effectiveMemory === 'ORIGINAL_H_CONFIG_READ_DISABLED',
      'Actual H config/read confirms the model memory configuration is disabled');
    const output = path.join(config.evidenceDirectory, `model-memory-${index}-final.json`);
    check(!fs.existsSync(output) && sha256(config.modelMemoryReader.args[0]) === config.modelMemoryReader.scriptSha256,
      'Original model memory reader and fresh output are unchanged');
    await runChild(config.modelMemoryReader.runtime,
      config.modelMemoryReader.args.map(arg => arg === '{home}' ? home.path : arg === '{output}' ? output : arg),
      'Actual closed model CODEX_HOME memory observation');
    const memory = readJson(output);
    check(memory.credentialsRead === false && memory.memoryDataUnchangedByRead === true &&
      memory.stage1OutputCount === 0 && memory.memoryJobCount === 0,
      'Actual final model HOME has no memory jobs or stage1 outputs and observation preserves its bytes');
    journal.actualModelMemory.push({ ...home, file: path.basename(output), sha256: sha256(output),
      readerSha256: config.modelMemoryReader.scriptSha256,
      scope: 'ORIGINAL_EFFECTIVE_CONFIG_AND_FINAL_STORE_NOT_CONTINUOUS_WRITE_MONITOR' }); product.save();
  }
  await snapshot('after');
  const finalMemory = snapshotValue('memory', 'after');
  check(finalMemory.memoryDataUnchangedByRead && finalMemory.stage1OutputCount === 0 &&
    finalMemory.memoryJobCount === 0, 'Registration HOME memory observation remains unchanged');
  for (const observer of config.observers) {
    const before = snapshotValue(observer.name, 'before');
    const after = snapshotValue(observer.name, 'after');
    for (const field of observer.equalFields) check(Object.hasOwn(before, field) &&
      Object.hasOwn(after, field) && JSON.stringify(before[field]) === JSON.stringify(after[field]),
    `${observer.name}.${field} preserved by the V12 case`);
  }
  journal.v12 = 'FLOW_COMPLETE_DIRECT_LEDGER_READBACK_REVIEW_REQUIRED';
  journal.state = 'DIRECT_V12_FACTS_REVIEW_REQUIRED'; journal.acceptance = false; product.save();
} catch (error) {
  journal.state = 'FAIL_OR_NOT_RUN_PRESERVE_ORIGINAL';
  journal.v12 = journal.v12 === 'RUNNING' ? 'FAIL_ORIGINAL_EVIDENCE_RETAINED' : journal.v12;
  journal.originalError = String(error?.stack ?? error); journal.currentEndpoint = product.endpoint ?? null;
  product.save(); process.exitCode = 1;
  try { await product.preserveFailure(); }
  catch (preserveError) { journal.preserveError = String(preserveError?.stack ?? preserveError); product.save(); }
}
