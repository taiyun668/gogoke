import fs from 'node:fs';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { spawn } from 'node:child_process';
import { ActualProduct, readJson, id, delay } from './product-cdp.mjs';

const config = readJson(process.argv[2]);
if (process.platform !== 'win32' || config.instanceId !== 'codexTestM1' ||
    config.domainId !== 'gogokeSeatTestbedM1' || config.repositoryId !== 'gogokeSeatTestbed' ||
    !/^[a-f0-9]{40}$/.test(config.sourceCommit) || fs.existsSync(config.result)) {
  throw Error('New Win11 case, exact candidate source and codexTestM1 required; existing journals are not replayed');
}
const journal = { schema: 'gogoke.37.win11-e2e.v1', sourceCommit: config.sourceCommit,
  version: config.version, domainId: config.domainId, instanceId: config.instanceId,
  state: 'RUNNING', acceptance: false, authenticationActions: false,
  launches: [], closes: [], operations: [], sessions: [], subscriptions: [], snapshots: {}, assertions: [] };
const product = new ActualProduct(config, journal);
const check = (condition, name) => {
  if (!condition) throw Error(name);
  // Check every value, but record the same assertion name only once.
  if (!journal.assertions.includes(name)) { journal.assertions.push(name); product.save(); }
};

async function snapshot(phase) {
  for (const observer of config.observers) {
    const output = path.join(config.evidenceDirectory, `${observer.name}-${phase}.json`);
    if (fs.existsSync(output)) throw Error('Snapshot already exists');
    const args = observer.args.map(value => value === '{output}' ? output : value);
    await new Promise((resolve, reject) => {
      const child = spawn(observer.runtime, args, { windowsHide: true, stdio: ['ignore', 'ignore', 'pipe'] });
      let stderr = ''; child.stderr.on('data', bytes => { stderr = (stderr + bytes).slice(-8192); });
      child.once('error', reject); child.once('exit', code => code === 0 ? resolve() : reject(Error(`Readonly ${observer.name}: exit=${code}; ${stderr}`)));
    });
    const value = readJson(output);
    journal.snapshots[`${observer.name}-${phase}`] = {
      file: path.basename(output), sha256: createHash('sha256').update(fs.readFileSync(output)).digest('hex'),
      ...(observer.name === 'ledger' ? { epoch: value.epoch, cursor: value.cursor } : {}),
    };
    product.save();
  }
}

function observedSnapshot(name, phase) {
  const reference = journal.snapshots[`${name}-${phase}`];
  const file = path.join(config.evidenceDirectory, reference.file);
  if (createHash('sha256').update(fs.readFileSync(file)).digest('hex') !== reference.sha256) {
    throw Error(`Original ${name}-${phase} snapshot bytes changed`);
  }
  return readJson(file);
}

async function sessionOperation(session, operation, payload = {}) {
  const change = ['compact', 'renew-session', 'resume'].includes(operation);
  let reply = await product.operation('K-SESSION', operation, session.id,
    { generation: session.generation, ...payload }, session.revision, change ? ['APPLIED', 'UNKNOWN'] : ['APPLIED']);
  let record = journal.operations.at(-1);
  const deadline = Date.now() + 600000;
  while (reply.status === 'UNKNOWN') {
    if (reply.result.reason || Date.now() >= deadline) throw Error(`Original generation outcome UNKNOWN: ${reply.result.reason ?? 'observation deadline'}`);
    // The product resumes its durable original operation. Its RPC journal
    // fences a second vendor write; exact original bytes/id stay unchanged.
    await delay(300);
    record = await product.reconcile(record); reply = record.receipt;
  }
  session.revision = reply.revision;
  if (reply.result.newGeneration) {
    check(reply.result.oldGeneration === session.generation, `${operation}: exact old generation`);
    check(BigInt(reply.result.newGeneration) > BigInt(session.generation), `${operation}: generation advances`);
    session.generation = reply.result.newGeneration;
  }
  product.save(); return reply;
}

async function openSession() {
  const seat = await product.seat();
  check(seat.result.state === 'IDLE' && seat.result.instanceId === config.instanceId, 'Actual seat IDLE and bound to original instance');
  check(seat.result.settings?.model === 'gpt-6.1-sol' && seat.result.settings?.effort === 'high', 'Actual seat GPT-6.1-Sol/high');
  const session = { id: id('m1E2e'), generation: (BigInt(seat.result.generation) + 1n).toString(),
    revision: '0', cursor: '0', events: [], artifacts: [], turns: [], marker: id('M1_E2E'), file: `${id('m1-e2e')}.json` };
  journal.sessions.push(session); product.save();
  await sessionOperation(session, 'admission-reserve', { seatId: config.seatId });
  await sessionOperation(session, 'admission-commit', { seatId: config.seatId });
  const opened = await sessionOperation(session, 'open', { seatId: config.seatId,
    repositoryId: config.repositoryId, worktreeId: config.worktreeId });
  session.threadId = opened.result.threadId; product.save();
  await capabilities(session);
  return session;
}

async function capabilities(session) {
  const reply = await sessionOperation(session, 'capability-probe');
  check(reply.result.version === config.cliVersion && reply.result.binaryDigest === `sha256:${config.cliSha256}`,
    'Actual fixed CLI version and bytes');
  check(reply.result.loadedThreadFeatures?.memories === false, 'Actual loaded memories=false');
  if (session.threadId) check(reply.result.threadId === session.threadId, 'Same native thread');
}

async function instancePage() {
  if (!product.tester) throw Error('The selected e2e connection is required for hard UI navigation');
  const page = product.tester.page;
  await page.getByRole('button', { name: /^(Open settings|打开设置)$/ }).click();
  await page.locator('.settings-sidebar').getByRole('button', { name: /^(Instances|实例)$/ }).click();
  const card = page.locator('.settings-toggle-row').filter({
    has: page.getByText('Codex 测试实例', { exact: true }),
  });
  await card.getByRole('status').filter({ hasText: '状态：已登录' }).waitFor({ state: 'visible', timeout: 15000 });
  check(await card.count() === 1, 'Actual instance UI contains exactly the original test instance');
  check(await card.getByRole('button', { name: '一键登录', exact: true }).isDisabled(),
    'Actual instance UI automatically shows logged in without authentication');
  await page.locator('.settings-close').click();
  check(await page.locator('.home-product-entry').isVisible(), 'Actual Home restored after instance page close');
}

async function output(session) {
  const reply = await sessionOperation(session, 'output-stream', { afterCursor: session.cursor });
  check(BigInt(reply.result.cursor) >= BigInt(session.cursor), 'Output cursor does not regress');
  session.cursor = reply.result.cursor;
  session.events.push(...reply.result.events); product.save();
  if (reply.result.sourceError) throw Error(`Original CLI output error: ${reply.result.sourceError}`);
  return reply.result;
}

async function observeUntil(session, predicate) {
  const deadline = Date.now() + 600000;
  while (Date.now() < deadline) {
    const page = await output(session);
    if (predicate(page)) return page;
    // Only new readonly requests are sampled; never repeat send/answer/steer.
    await delay(300);
  }
  throw Error('Real CLI observation deadline; original mutation retained, no resend');
}

async function runTurn(session, question) {
  const target = path.resolve(config.worktreeRoot, session.file);
  check(path.dirname(target) === path.resolve(config.worktreeRoot) && !fs.existsSync(target), 'New file only in host-created test worktree');
  const prompt = `Owner-authorized M1 end-to-end test in the private gogoke-seat-testbed repository. ` +
    `Only write in the current host-created worktree; no agents, remotes, publishing or outside paths. ` +
    (question ? 'First use native request_user_input to ask the result format, with JSON and Plain text options. Wait for the answer. ' : '') +
    `Use an actual tool to create the NEW relative file ${session.file}, JSON with marker property exactly ${session.marker}. ` +
    `Do not claim success if a tool fails; report its original error. Reply DONE ${session.marker}.`;
  await sessionOperation(session, 'send', { body: prompt });
  if (question) {
    let card;
    await observeUntil(session, page => {
      card = page.nativeCardRefs.find(row => row.state === 'OPEN');
      return Boolean(card);
    });
    const recovered = await product.operation('K-QCARD', 'recover', card.cardId, {}, card.revision);
    const q = recovered.result.nativeQuestion;
    check(recovered.result.availableForAnswer && recovered.result.seatId === config.seatId &&
      recovered.result.generation === session.generation && q.threadId === session.threadId, 'Original native question binding and recovery');
    const messageId = id('m1Steer');
    session.steerMarker = id('STEER_M1_E2E');
    const queued = await product.operation('K-INBOX', 'enqueue', messageId, { seatId: config.seatId,
      turnId: q.turnId, generation: session.generation,
      body: `After answering the current question, keep the file marker unchanged and include ${session.steerMarker} in the final reply; this is guidance for the same turn.` });
    const steered = await product.operation('K-INBOX', 'steer', messageId, { turnId: q.turnId,
      generation: session.generation }, queued.revision);
    check(steered.result.state === 'DELIVERED' && steered.result.mode === 'NATIVE', 'Same-turn native steer write receipt');
    const answers = {};
    for (const row of q.questions) {
      const option = row.options.find(value => /^JSON(?: \(Recommended\))?$/.test(value.label));
      check(Boolean(option) && !row.isSecret, 'Non-secret real question has JSON choice'); answers[row.id] = [option.label];
    }
    const answered = await product.operation('K-QCARD', 'answer', card.cardId,
      { generation: session.generation, answers }, recovered.revision);
    check(answered.result.state === 'ANSWERED' && answered.result.deliveryBasis === 'NATIVE_EXACT_WRITE_RECEIPT', 'Native answer receipt, not vendor-consumption claim');
  }
  const start = session.events.length;
  await observeUntil(session, () => session.events.slice(start).some(event => event._meta?.codexMethod === 'turn/completed'));
  const events = session.events.slice(start);
  const completed = events.find(event => event._meta?.codexMethod === 'turn/completed');
  check(completed._meta.turnStatus === 'completed', 'Actual CLI turn completed successfully');
  session.turns.push({ threadId: session.threadId, turnId: completed._meta.turnId,
    generation: session.generation, marker: session.marker, file: session.file });
  const entry = fs.lstatSync(target);
  check(entry.isFile() && !entry.isSymbolicLink() && entry.nlink === 1, 'Tool result is one ordinary new file');
  const contents = fs.readFileSync(target);
  check(JSON.parse(contents).marker === session.marker, 'Actual tool file marker, not assistant self-report');
  session.fileSha256 = createHash('sha256').update(contents).digest('hex');
  session.artifacts.push({ file: session.file, marker: session.marker, sha256: session.fileSha256 });
  if (question) {
    const assistantOutput = events.filter(event => event._meta?.codexMethod === 'item/agentMessage/delta' &&
      event._meta?.turnId === completed._meta.turnId && event._meta?.threadId === session.threadId)
      .map(event => event.content?.type === 'text' ? event.content.text : '').join('');
    session.liveSteerObserved = assistantOutput.includes(session.steerMarker);
    // The protocol permits completed agentMessage without text deltas. The
    // required consumption assertion uses that exact raw frame after close.
    session.steerVerification = 'DIRECT_ORIGINAL_COMPLETION_FRAME_REQUIRED';
  }
  session.turnCompleted = true; product.save();
}

async function stop(session, release) {
  const stopped = await sessionOperation(session, 'stop', { seatId: config.seatId });
  check(typeof stopped.result.stopFact === 'string' && stopped.result.stopFact.length > 0, 'Durable native whole-Job stop fact');
  if (release) await sessionOperation(session, 'admission-release', { seatId: config.seatId });
}

let subscription;
async function ledger(session, start = false, end = false) {
  if (start) {
    const prior = journal.subscriptions.at(-1);
    subscription = { id: id('m1Ledger'), readerSessionId: session.id,
      epoch: journal.snapshots['ledger-before'].epoch,
      cursor: prior?.cursor ?? journal.snapshots['ledger-before'].cursor, revision: '0', events: [] };
    journal.subscriptions.push(subscription); product.save();
  }
  const operation = start ? 'subscribe' : end ? 'end-subscription' : 'resume-subscription';
  const payload = end ? {} : { epoch: subscription.epoch, afterCursor: subscription.cursor,
    ...(start ? { readerSessionId: session.id, scope: 'PROJECT' } : {}) };
  const reply = await product.operation('K-LEDGER', operation, subscription.id, payload, subscription.revision);
  check(reply.result.epoch === subscription.epoch && BigInt(reply.result.cursor) >= BigInt(subscription.cursor), 'Same ledger epoch and non-regressing cursor');
  const seen = new Set(subscription.events.map(event => event.sourceEventId));
  let cursor = BigInt(subscription.cursor);
  for (const event of reply.result.events) {
    check(BigInt(event.cursor) > cursor && !seen.has(event.sourceEventId), 'Subscribed original event has no duplicate or rewind');
    cursor = BigInt(event.cursor); seen.add(event.sourceEventId); subscription.events.push(event);
  }
  subscription.cursor = reply.result.cursor; subscription.revision = reply.revision; product.save();
  return reply.result.events.length;
}
async function drainLedger(session) {
  while (await ledger(session)) { /* Read the next actual subscription page. */ }
}

try {
  check(Array.isArray(config.observers) && ['formal', 'memory', 'ledger'].every(name => config.observers.some(row => row.name === name)),
    'Mandatory real formal, memory and ledger snapshots');
  const formal = config.observers.find(row => row.name === 'formal');
  check(['formal', 'registeredFormal', 'formalData', 'formalRegistry', 'shortcuts'].every(field => formal.equalFields.includes(field)),
    'All five formal protection fields must be compared');
  await snapshot('before');
  await product.launch();
  const instances = await product.instances(); journal.instances = instances; product.save();
  const instance = instances.instances.find(row => row.instanceId === config.instanceId);
  check(instance?.state === 'LOGGED_IN', 'Original instance automatically detected LOGGED_IN; no login/logout');
  await instancePage();
  const first = await openSession();
  await ledger(first, true);
  await runTurn(first, true);
  const compacted = await sessionOperation(first, 'compact');
  check(compacted.result.state === 'RUNNING' && compacted.result.newGeneration, 'Authenticated compact receipt changes physical generation');
  await capabilities(first);
  const renewed = await sessionOperation(first, 'renew-session');
  check(renewed.result.state === 'RUNNING' && renewed.result.newGeneration, 'Renewed physical generation');
  await capabilities(first);
  await stop(first, false);
  await drainLedger(first);
  await product.closeNormally();
  await product.launch();
  await instancePage();
  await drainLedger(first);
  const resumed = await sessionOperation(first, 'resume');
  check(resumed.result.state === 'RUNNING' && resumed.result.newGeneration, 'Actual product restart resumes logical session');
  await capabilities(first);
  first.marker = id('RESUME_M1_E2E'); first.file = `${id('m1-resume')}.json`; first.cursor = '0';
  await runTurn(first, false);
  await drainLedger(first); await ledger(first, false, true);
  await stop(first, true);
  const second = await openSession();
  check(second.id !== first.id, 'Two different logical sessions');
  await ledger(second, true);
  await runTurn(second, false);
  await drainLedger(second); await ledger(second, false, true);
  await stop(second, true);
  const seat = await product.seat(); check(seat.result.state === 'IDLE', 'Final native seat IDLE');
  await product.closeNormally();
  await snapshot('after');
  for (const observer of config.observers) {
    const before = observedSnapshot(observer.name, 'before'), after = observedSnapshot(observer.name, 'after');
    check(observer.equalFields.length > 0, 'Snapshot has explicit compared fields');
    for (const field of observer.equalFields) {
      check(Object.hasOwn(before, field) && Object.hasOwn(after, field) &&
        JSON.stringify(before[field]) === JSON.stringify(after[field]), `${observer.name}: unchanged ${field}`);
    }
  }
  for (const phase of ['before', 'after']) {
    const memory = observedSnapshot('memory', phase);
    check(memory.memoryDataUnchangedByRead && memory.stage1OutputCount === 0 && memory.memoryJobCount === 0,
      'Actual memory store unchanged by measurement and no memory jobs');
  }
  // Raw protocol/ledger and compact completion are checked separately after close.
  // The runner alone is not the M1 gate or Owner acceptance.
  journal.state = 'ACTUAL_FLOW_COMPLETE_DIRECT_LEDGER_READBACK_REQUIRED'; product.save();
} catch (error) {
  journal.state = 'FAIL'; journal.error = String(error.stack ?? error);
  journal.currentEndpoint = product.endpoint ?? null; product.save(); process.exitCode = 1;
  // Preserve a still-running real product and its original request for Controller.
  // No forced termination, credential reset, mutation replay or fake fallback.
  try { await product.preserveFailure(); }
  catch (disconnectError) { journal.disconnectError = String(disconnectError); product.save(); }
}
