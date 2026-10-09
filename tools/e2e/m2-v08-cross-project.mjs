// Prepare one real B gate for the installed-product V08 cross-project model case.
// This entry does not invoke a model or change any A policy, seat, or session.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { ActualProduct, id, readJson, sha256 } from './product-cdp.mjs';

const configPath = process.argv[2];
if (!configPath) throw Error('Private V08 foreign-project config path required');
const config = readJson(configPath);
const here = path.dirname(fileURLToPath(import.meta.url));
const atom = value => typeof value === 'string' && /^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/.test(value);
const check = (condition, message) => { if (!condition) throw Error(message); };
const inside = (child, parent) => child === parent || child.startsWith(parent + path.sep);
const formalFields = ['formal', 'registeredFormal', 'formalData', 'formalRegistry', 'shortcuts'];
const foreign = config.foreignProject;

check(process.platform === 'win32' && config.repositoryId === 'gogokeSeatTestbed' &&
  /^[a-f0-9]{40}$/.test(config.sourceCommit ?? '') && /^\d+\.\d+\.\d+$/.test(config.version ?? '') &&
  [config.domainId, config.instanceId].every(atom) &&
  config.installedSha256 && typeof config.installedSha256 === 'object' &&
  ['installed', 'registryKey', 'pwsh', 'stateRoot', 'testbedSource', 'evidenceDirectory', 'result']
    .every(key => typeof config[key] === 'string' && config[key].length > 0) &&
  foreign?.ownership === 'EXCLUSIVE_V08_FOREIGN_TEST_DOMAIN' &&
  [foreign.domainId, foreign.gateId, foreign.submitterSeatId, foreign.reviewerSeatId,
    foreign.fromStage, foreign.toStage].every(atom) &&
  foreign.domainId !== config.domainId && foreign.submitterSeatId !== foreign.reviewerSeatId &&
  foreign.fromStage !== foreign.toStage && Number.isInteger(foreign.rejectCap) && foreign.rejectCap > 0 &&
  Array.isArray(config.observers) &&
  config.observers.some(row => row.name === 'formal' &&
    formalFields.every(field => row.equalFields?.includes(field))) &&
  foreign.ownerHead && Object.keys(foreign.ownerHead).sort().join(',') === 'rawFrame,rawReceipt' &&
  typeof foreign.ownerHead.rawFrame === 'string' && typeof foreign.ownerHead.rawReceipt === 'string',
'V08 B needs an existing real test domain, distinct E seats, original Owner head and protected observer');
const headRequest = JSON.parse(foreign.ownerHead.rawFrame);
const headReceipt = JSON.parse(foreign.ownerHead.rawReceipt);
check(headRequest.schema === 'gogoke.37.owner-configuration.v1' &&
  headRequest.command === 'policy-initialize' && headRequest.domainId === foreign.domainId &&
  headRequest.stage === foreign.fromStage && headRequest.expectedRevision === '0' &&
  Object.keys(headRequest).sort().join(',') === 'command,domainId,expectedRevision,requestId,schema,stage' &&
  atom(headRequest.requestId) &&
  headReceipt.schema === headRequest.schema && headReceipt.command === headRequest.command &&
  headReceipt.requestId === headRequest.requestId && headReceipt.status === 'APPLIED' &&
  headReceipt.revision === '1', 'B head must have original NativeUser initialization bytes');

const resultPath = path.resolve(config.result);
const evidencePath = path.resolve(config.evidenceDirectory);
check(!fs.existsSync(resultPath) && fs.existsSync(evidencePath) &&
  inside(resultPath, evidencePath) &&
  ![config.installed, config.stateRoot, config.testbedSource, path.resolve(here, '..', '..')]
    .some(root => inside(evidencePath, path.resolve(root))),
'Fresh private result must be outside installed, state, testbed and source roots');
config.result = resultPath;
config.evidenceDirectory = evidencePath;
const journal = {
  schema: 'gogoke.37.m2-v08-foreign-preparation.v1',
  state: 'RUNNING', acceptance: false, authenticationActions: false,
  sourceCommit: config.sourceCommit, domainId: config.domainId,
  foreignProject: { ...foreign, ownerGate: null },
  operations: [], launches: [], closes: [], snapshots: {},
};
const product = new ActualProduct(config, journal);
delete journal.driverBytes['m1-win11.mjs'];
delete journal.driverBytes['m1-readback.py'];
journal.driverBytes['m2-v08-cross-project.mjs'] = sha256(fileURLToPath(import.meta.url));
product.save();

async function userFrame(request, expectedStatus) {
  const rawFrame = JSON.stringify(request);
  const entry = { request, rawFrame, receipt: null, startedAt: new Date().toISOString() };
  journal.operations.push(entry); product.save();
  try {
    entry.rawReceipt = await product.evaluate(
      `window.__TAURI_INTERNALS__.invoke('gogoke_design37_user_operation',{frame:${JSON.stringify(rawFrame)}})`);
    entry.receipt = JSON.parse(entry.rawReceipt);
    entry.finishedAt = new Date().toISOString(); product.save();
  } catch (error) {
    entry.originalError = String(error?.stack ?? error); product.save(); throw error;
  }
  check(entry.receipt.schema === request.schema && entry.receipt.status === expectedStatus &&
    entry.receipt.requestId === request.requestId && entry.receipt.command === request.command,
  `Original ${request.command} did not produce its expected one-time native receipt`);
  return entry;
}

async function seatCard(seatId, revision = '0', staleRetried = false) {
  const request = { schema: 'gogoke.37.operations.v1', family: 'K-SEAT', operation: 'state-card',
    requestId: id('v08ForeignCard'), domainId: foreign.domainId, targetId: seatId,
    expectedRevision: revision, payload: {} };
  const rawFrame = JSON.stringify(request);
  const entry = { request, rawFrame, receipt: null, startedAt: new Date().toISOString() };
  journal.operations.push(entry); product.save();
  try {
    entry.rawReceipt = await product.evaluate(
      `window.__TAURI_INTERNALS__.invoke('gogoke_design37_user_operation',{frame:${JSON.stringify(rawFrame)}})`);
    entry.receipt = JSON.parse(entry.rawReceipt);
    entry.finishedAt = new Date().toISOString(); product.save();
  } catch (error) {
    entry.originalError = String(error?.stack ?? error); product.save(); throw error;
  }
  check(entry.receipt.schema === request.schema && entry.receipt.family === request.family &&
    entry.receipt.operation === request.operation && entry.receipt.domainId === foreign.domainId &&
    entry.receipt.targetId === seatId && entry.receipt.requestId === request.requestId &&
    ['APPLIED', 'STALE'].includes(entry.receipt.status), 'B E seat card identity differs');
  if (entry.receipt.status === 'STALE') {
    check(!staleRetried && /^[1-9][0-9]*$/.test(entry.receipt.revision),
      'B seat state-card is repeatedly stale');
    return seatCard(seatId, entry.receipt.revision, true);
  }
  return entry.receipt;
}

async function formalSnapshot(phase) {
  const observer = config.observers.find(row => row.name === 'formal');
  const output = path.join(evidencePath, `m2-v08-foreign-formal-${phase}-${id('snap')}.json`);
  check(!fs.existsSync(output), 'Formal snapshot must be fresh');
  const { spawn } = await import('node:child_process');
  await new Promise((resolve, reject) => {
    const child = spawn(observer.runtime, observer.args.map(value => value === '{output}' ? output : value),
      { windowsHide: true, stdio: ['ignore', 'ignore', 'pipe'] });
    let stderr = '';
    child.stderr.on('data', bytes => { stderr = (stderr + bytes).slice(-8192); });
    child.once('error', reject);
    child.once('exit', code => code === 0 ? resolve() :
      reject(Error(`Formal ${phase} exit=${code}: ${stderr}`)));
  });
  const ref = { file: path.basename(output), sha256: sha256(output) };
  journal.snapshots[phase] = ref; product.save();
  return readJson(output);
}

try {
  const before = await formalSnapshot('before');
  await product.launch();
  for (const seatId of [foreign.submitterSeatId, foreign.reviewerSeatId]) {
    const card = await seatCard(seatId);
    check(card.status === 'APPLIED' && card.result?.layer === 'USER' &&
      card.result?.state === 'IDLE', 'B must contain an actual distinct idle User seat');
  }
  const request = { schema: 'gogoke.37.owner-configuration.v1', command: 'policy-gate',
    domainId: foreign.domainId, requestId: id('v08ForeignGate'), gateId: foreign.gateId,
    submitterSeatId: foreign.submitterSeatId, reviewerSeatId: foreign.reviewerSeatId,
    fromStage: foreign.fromStage, toStage: foreign.toStage,
    rejectCap: foreign.rejectCap, expectedRevision: headReceipt.revision };
  const gate = await userFrame(request, 'APPLIED');
  check(gate.receipt.revision === '2', 'B gate must advance original fresh head by one CAS');
  journal.foreignProject.ownerGate = { rawFrame: gate.rawFrame, rawReceipt: gate.rawReceipt };
  product.save();
  await product.closeNormally();
  const after = await formalSnapshot('after');
  check(formalFields.every(field => Object.hasOwn(before, field) && Object.hasOwn(after, field) &&
    JSON.stringify(before[field]) === JSON.stringify(after[field])),
  'Five formal groups changed during foreign gate preparation');
  journal.state = 'B_GATE_PREPARED_REQUIRES_V08_ORIGINAL_CLOSED_READBACK';
  journal.rulesForeignProject = { domainId: foreign.domainId, gateId: foreign.gateId,
    ownerGate: journal.foreignProject.ownerGate };
  product.save();
} catch (error) {
  journal.state = 'FAILED_ORIGINAL_REQUESTS_RETAINED';
  journal.error = String(error?.stack ?? error);
  product.save();
  process.exitCode = 1;
  try { await product.preserveFailure(); }
  catch (preserveError) { journal.preserveError = String(preserveError?.stack ?? preserveError); product.save(); }
  try { await formalSnapshot('failure-after'); }
  catch (observerError) { journal.formalFailure = String(observerError?.stack ?? observerError); product.save(); }
}
