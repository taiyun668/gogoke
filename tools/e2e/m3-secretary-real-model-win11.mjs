// One installed-product Secretary USER turn. Development evidence, not V14 acceptance.
import fs from 'node:fs';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { ActualProduct, delay, id, readJson, sha256 } from './product-cdp.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const configPath = process.argv[2];
if (!configPath) throw Error('Private installed Secretary model config required');
const config = readJson(configPath);
const selection = config.m3SecretaryRealModel;
const check = (value, message) => { if (!value) throw Error(message); };
const absolute = value => typeof value === 'string' && path.isAbsolute(value);
const inside = (child, parent) => child === parent || child.startsWith(parent + path.sep);
const decimal = value => typeof value === 'string' && /^(0|[1-9][0-9]*)$/.test(value);
const expectedNames = ['formal', 'memory', 'ledger'];
check(process.platform === 'win32' && config.domainId === 'global' &&
  config.testerArmy === true && /^[a-f0-9]{40}$/.test(config.sourceCommit ?? '') &&
  /^\d+\.\d+\.\d+$/.test(config.version ?? '') &&
  ['installed', 'pwsh', 'python', 'stateRoot', 'evidenceDirectory', 'result'].every(key => absolute(config[key])) &&
  ['gogoke.exe', 'gogoke-native-host.exe', 'resource-index.json'].every(name =>
    /^[a-f0-9]{64}$/.test(config.installedSha256?.[name] ?? '')) &&
  typeof config.registryKey === 'string' && config.registryKey.length > 0 &&
  fs.existsSync(config.evidenceDirectory) && !fs.existsSync(config.result) &&
  path.dirname(path.resolve(config.result)) === path.resolve(config.evidenceDirectory) &&
  [config.installed, config.stateRoot, path.resolve(here, '..', '..')].every(root =>
    !inside(path.resolve(config.evidenceDirectory), path.resolve(root))) &&
  selection && ['seatId', 'incarnation', 'instanceId', 'model', 'effort', 'permissionTier']
    .every(key => typeof selection[key] === 'string' && selection[key].length > 0) &&
  selection.globalParallelCap === '4' &&
  Array.isArray(config.observers) && config.observers.length === 3 &&
  expectedNames.every(name => config.observers.some(row => row.name === name)) &&
  config.observers.every(row => absolute(row.runtime) && Array.isArray(row.args) &&
    row.args.every(arg => typeof arg === 'string') && Array.isArray(row.equalFields) && row.equalFields.length > 0) &&
  ['formal', 'registeredFormal', 'formalData', 'formalRegistry', 'shortcuts'].every(field =>
    config.observers.find(row => row.name === 'formal').equalFields.includes(field)),
  'Exact installed candidate, separate exclusive evidence, live Secretary selection and three observers required');

const marker = id('SECRETARY_USER_MODEL');
const prompt = `Answer with exactly ${marker} and no other text. Do not use tools, read or write files, open a browser, access credentials, or switch models.`;
const journal = { schema: 'gogoke.37.private-m3-v14-secretary-real-model.v1', caseId: id('m3SecretaryModel'),
  sourceCommit: config.sourceCommit, candidateVersion: config.version, installedSha256: config.installedSha256,
  stateRoot: path.resolve(config.stateRoot), evidenceDirectory: path.resolve(config.evidenceDirectory),
  selection, marker, prompt, state: 'RUNNING', acceptance: false, credentialReads: false,
  authenticationActions: false, observerDatabaseWrites: false, operations: [], launches: [], closes: [],
  snapshots: {}, readbacks: [], model: { state: 'NOT_DISPATCHED', steered: false, newTurn: null,
    unknown: false, requestId: null, originalReceipt: null, terminalReceipt: null },
  notRun: ['CROSS_PROJECT', 'SCHEDULE_SCOPE_AND_DELIVERY', 'V14_FULL_ACCEPTANCE'] };
const product = new ActualProduct(config, journal);
journal.driverBytes['m3-secretary-real-model-win11.mjs'] = sha256(path.join(here, 'm3-secretary-real-model-win11.mjs'));
journal.driverBytes['m3-secretary-real-model-readback.py'] = sha256(path.join(here, 'm3-secretary-real-model-readback.py'));

async function snapshot(phase) {
  for (const observer of config.observers) {
    const file = path.join(config.evidenceDirectory, `${observer.name}-secretary-model-${phase}.json`);
    check(!fs.existsSync(file), `Observer ${phase} output already exists`);
    await new Promise((resolve, reject) => {
      const child = spawn(observer.runtime, observer.args.map(arg => arg === '{output}' ? file : arg),
        { windowsHide: true, stdio: ['ignore', 'ignore', 'pipe'] });
      let stderr = '';
      child.stderr.on('data', bytes => { stderr = (stderr + bytes).slice(-8192); });
      child.once('error', reject);
      child.once('exit', code => code === 0 ? resolve() : reject(Error(`${observer.name} observer exit=${code}: ${stderr}`)));
    });
    journal.snapshots[`${observer.name}-${phase}`] = { file: path.basename(file), sha256: sha256(file) };
    product.save();
  }
}

function compareProtection() {
  let fields = 0;
  for (const observer of config.observers) {
    const values = ['before', 'after'].map(phase => {
      const ref = journal.snapshots[`${observer.name}-${phase}`];
      const file = path.join(config.evidenceDirectory, ref.file);
      check(sha256(file) === ref.sha256, `Original ${observer.name} observer bytes changed`);
      return readJson(file);
    });
    for (const field of observer.equalFields) {
      check(Object.hasOwn(values[0], field) && Object.hasOwn(values[1], field) &&
        JSON.stringify(values[0][field]) === JSON.stringify(values[1][field]),
      `Protected ${observer.name}/${field} changed`);
      fields++;
    }
    if (observer.name === 'memory') for (const value of values) {
      check(value.memoryDataUnchangedByRead === true && value.stage1OutputCount === 0 &&
        value.memoryJobCount === 0, 'Original memory observer failed its own integrity checks');
    }
  }
  check(fields === 13, `Expected 13 protected fields; observed ${fields}`);
  journal.protection = { state: 'PASS_FORMAL_MEMORY_LEDGER_FIELDS_UNCHANGED', fields };
  product.save();
}

async function readSecretary() {
  const frame = { schema: 'gogoke.37.owner-configuration.v1', command: 'secretary-configuration-read' };
  const rawFrame = JSON.stringify(frame);
  const entry = { kind: 'ORIGINAL_USER_SECRETARY_READ', frame, rawFrame, rawReply: null };
  journal.operations.push(entry); product.save();
  entry.rawReply = await product.evaluate(`window.__TAURI_INTERNALS__.invoke('gogoke_design37_user_operation',{frame:${JSON.stringify(rawFrame)}})`);
  entry.reply = JSON.parse(entry.rawReply); product.save();
  const value = entry.reply, conversation = value?.conversation;
  check(value?.schema === 'gogoke.37.secretary-configuration.v1' && value.state === 'DESIGNATED' &&
    ['seatId', 'incarnation', 'instanceId', 'model', 'effort', 'permissionTier', 'globalParallelCap']
      .every(key => value[key] === selection[key]) && value.seatState === 'BUSY' &&
    conversation?.state === 'FOUND' && conversation.runtimeAvailable === true &&
    conversation.historical === false && conversation.claimState === 'COMMITTED' &&
    conversation.stoppedFact === null && conversation.turnState === 'IDLE' &&
    typeof conversation.sessionId === 'string' && conversation.sessionId.length > 0 &&
    typeof conversation.threadId === 'string' && conversation.threadId.length > 0 &&
    decimal(conversation.generation) && decimal(conversation.revision),
  'Original USER read lacks the exact idle resident Secretary H; no input sent');
  journal.binding = { sessionId: conversation.sessionId, threadId: conversation.threadId,
    generation: conversation.generation, revision: conversation.revision,
    seatId: value.seatId, instanceId: value.instanceId };
  product.save();
  return journal.binding;
}

async function output(session) {
  let reply = await product.operation('K-SESSION', 'output-stream', session.id,
    { generation: session.generation, afterCursor: session.cursor }, session.revision, ['APPLIED', 'STALE']);
  if (reply.status === 'STALE') {
    session.revision = reply.revision; product.save();
    reply = await product.operation('K-SESSION', 'output-stream', session.id,
      { generation: session.generation, afterCursor: session.cursor }, session.revision);
  }
  check(decimal(reply.result.cursor) && BigInt(reply.result.cursor) >= BigInt(session.cursor),
    'Original H cursor moved backward');
  session.revision = reply.revision;
  session.cursor = reply.result.cursor;
  session.events.push(...reply.result.events);
  product.save();
  check(!reply.result.sourceError, 'Original H source reported an error');
  return reply.result;
}

async function readback() {
  const file = path.join(config.evidenceDirectory, 'm3-secretary-real-model-closed.json');
  check(!fs.existsSync(file), 'Immutable reader output already exists');
  await new Promise((resolve, reject) => {
    const child = spawn(config.python, [path.join(here, 'm3-secretary-real-model-readback.py'),
      config.stateRoot, file, config.result], { windowsHide: true, stdio: ['ignore', 'ignore', 'pipe'] });
    let stderr = '';
    child.stderr.on('data', bytes => { stderr = (stderr + bytes).slice(-8192); });
    child.once('error', reject);
    child.once('exit', code => code === 0 ? resolve() : reject(Error(`Immutable Secretary reader exit=${code}: ${stderr}`)));
  });
  const proof = readJson(file);
  check(proof.schema === 'gogoke.37.private-m3-v14-secretary-real-model-closed.v1' &&
    proof.databaseBytesPreserved === true && proof.acceptance === false &&
    proof.sessionId === journal.binding.sessionId &&
    proof.requestId === journal.model.requestId &&
    proof.stopFact?.startsWith('sha256:'), 'Original closed H/USER readback is incomplete');
  journal.readbacks.push({ file: path.basename(file), sha256: sha256(file), stopFact: proof.stopFact });
  product.save();
}

try {
  await snapshot('before');
  await product.launch();
  check(Boolean(product.tester) && journal.connectionBackend?.name === 'tester-army/e2e' &&
    journal.connectionBackend.telemetryDisabled === true && journal.connectionBackend.agentActs === 0,
  'Actual installed product tester-army binding differs');
  const binding = await readSecretary();
  const session = { id: binding.sessionId, generation: binding.generation,
    revision: binding.revision, threadId: binding.threadId, cursor: '0', events: [] };
  journal.session = session; product.save();
  let before;
  for (;;) {
    const previous = session.cursor;
    before = await output(session);
    check(decimal(before.rawHighwater) && decimal(before.ledgerHighwater) &&
      !before.nativeCardRefs?.some(card => card.state === 'OPEN'),
    'Original H pre-send source or card is incomplete');
    if (before.cursor === before.ledgerHighwater) break;
    check(BigInt(before.cursor) > BigInt(previous), 'Original H pre-send cursor did not advance');
  }
  journal.model.beforeSendRawHighwater = before.rawHighwater;
  journal.model.beforeSendEventCount = session.events.length;
  journal.model.state = 'ORIGINAL_REQUEST_PREPARED'; product.save();
  // An idle H permits one new turn. A running H would be a steer and fails above.
  const sent = await product.operation('K-SESSION', 'send', session.id,
    { generation: session.generation, body: prompt }, session.revision, ['APPLIED', 'UNKNOWN']);
  const original = journal.operations.at(-1);
  journal.model.requestId = original.request.requestId;
  journal.model.originalReceipt = sent;
  journal.model.unknown = sent.status === 'UNKNOWN';
  journal.model.state = 'ORIGINAL_REQUEST_DISPATCHED'; product.save();
  check(!(sent.status === 'UNKNOWN' && sent.result?.reason),
    'Original send UNKNOWN with reason; preserve H and original request, no resend');
  const deadline = Date.now() + 600000;
  let terminal = null;
  while (Date.now() < deadline) {
    const page = await output(session);
    const receipt = page.nativeInputReceipts?.find(row => row.requestId === journal.model.requestId);
    if (receipt?.receipt?.status === 'APPLIED') { terminal = receipt; break; }
    check(!receipt?.receipt?.status || receipt.receipt.status === 'UNKNOWN',
      'Original H returned a non-APPLIED receipt; no resend');
    await delay(300);
  }
  check(terminal, 'Original H receipt remains UNKNOWN; preserve H and request, no resend');
  const result = terminal.receipt.result;
  check(result?.createdTurn === true && typeof result.turnId === 'string' && result.turnId.length > 0,
  'Original H did not receipt a new Codex turn/start');
  journal.model.newTurn = true;
  journal.model.startReceipt = terminal;
  journal.model.state = 'ORIGINAL_H_NEW_TURN_STARTED'; product.save();
  let answer = false, completed = null;
  const terminalDeadline = Date.now() + 600000;
  while (Date.now() < terminalDeadline) {
    const page = await output(session);
    const text = session.events.slice(journal.model.beforeSendEventCount).filter(event =>
      event.sessionUpdate === 'agent_message_chunk' && event._meta?.threadId === session.threadId &&
      event._meta?.turnId === result.turnId &&
      decimal(event._meta?.rawSourceCursor) &&
      BigInt(event._meta.rawSourceCursor) > BigInt(before.rawHighwater))
      .map(event => event.content?.type === 'text' ? event.content.text : '').join('');
    const terminalEvent = session.events.slice(journal.model.beforeSendEventCount).find(event =>
      event._meta?.provider === 'codex' && event._meta?.codexMethod === 'turn/completed' &&
      event._meta?.threadId === session.threadId && event._meta?.turnId === result.turnId &&
      event._meta?.turnStatus === 'completed' && decimal(event._meta?.rawSourceCursor) &&
      BigInt(event._meta.rawSourceCursor) > BigInt(before.rawHighwater));
    if (terminalEvent && text.includes(marker) && decimal(page.rawHighwater) &&
        BigInt(page.rawHighwater) >= BigInt(terminalEvent._meta.rawSourceCursor) &&
        page.cursor === page.ledgerHighwater) {
      answer = true; completed = terminalEvent; break;
    }
    await delay(300);
  }
  check(answer && completed, 'Original completed Codex turn or answer marker absent from A');
  journal.model.terminalAEvent = completed;
  journal.model.answerObserved = true;
  journal.model.state = 'ORIGINAL_A_ANSWER_OBSERVED'; product.save();
  await product.closeNormally(); // Native host performs the real resident Secretary H stop.
  journal.state = 'CLOSED_PENDING_READBACK'; product.save();
  await readback();
  await snapshot('after');
  compareProtection();
  journal.state = 'REAL_MODEL_SLICE_READBACK_COMPLETE_REQUIRES_REVIEW'; product.save();
} catch (error) {
  journal.state = 'FAIL_OR_UNKNOWN_ORIGINAL_EVIDENCE_RETAINED';
  journal.error = String(error?.stack ?? error); product.save(); process.exitCode = 1;
  try { if (product.child?.exitCode === null) await product.preserveFailure(); }
  catch (preserveError) { journal.preserveError = String(preserveError?.stack ?? preserveError); product.save(); }
}
