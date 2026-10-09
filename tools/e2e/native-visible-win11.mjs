// Repeated installed native USER conversation check; reviewed private inputs required.
// Uses the real installed WebView, exact USER pipe and DOM events. No agent.act or login.
import fs from 'node:fs';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { execFileSync, spawn } from 'node:child_process';
import { fileURLToPath, pathToFileURL } from 'node:url';

const fail = message => { throw Error(message); };
const check = (condition, message) => { if (!condition) fail(message); };
const nonempty = value => typeof value === 'string' && value.trim().length > 0;
const hex64 = value => typeof value === 'string' && /^[a-f0-9]{64}$/.test(value);
const within = (child, parent) => child === parent || child.startsWith(parent + path.sep);
const digest = file => createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const cfgFile = process.argv[2];
check(cfgFile && path.isAbsolute(cfgFile), 'An absolute private config JSON path is required.');
const privateRoot = path.dirname(cfgFile);
const config = JSON.parse(fs.readFileSync(cfgFile, 'utf8').replace(/^\uFEFF/, ''));
const SOURCE = config.sourceCommit;
check(typeof SOURCE === 'string' && /^[a-f0-9]{40}$/.test(SOURCE), 'Exact installed source commit required.');
for (const key of ['sourceRoot', 'installed', 'version', 'registryKey', 'pwsh', 'python', 'stateRoot',
  'evidenceDirectory', 'domainId', 'seatId', 'instanceId', 'repositoryId',
  'worktreeId', 'workspacePath', 'cliVersion', 'askPrompt', 'steerPrompt']) {
  check(nonempty(config[key]), `Missing private input: ${key}`);
}

check(['sourceRoot', 'installed', 'workspacePath', 'evidenceDirectory', 'stateRoot']
  .every(key => path.isAbsolute(config[key])) &&
  config.askPrompt === config.askPrompt.trim() &&
  config.steerPrompt === config.steerPrompt.trim(),
  'Private paths must be absolute and prompts must not change under Composer trimming.');
check(config.readyEvidence && nonempty(config.readyEvidence.path) &&
  path.isAbsolute(config.readyEvidence.path) &&
  hex64(config.readyEvidence.sha256) &&
  digest(config.readyEvidence.path) === config.readyEvidence.sha256,
  'Closed candidate nonsecret metadata baseline file/hash is missing or changed.');
check(config.m2SourceConfig && nonempty(config.m2SourceConfig.path) &&
  path.isAbsolute(config.m2SourceConfig.path) &&
  hex64(config.m2SourceConfig.sha256) &&
  digest(config.m2SourceConfig.path) === config.m2SourceConfig.sha256,
  'Original nonsecret M2 binding config file/hash is missing or changed.');
const originalM2 = JSON.parse(fs.readFileSync(config.m2SourceConfig.path, 'utf8').replace(/^\uFEFF/, ''));
for (const key of ['domainId', 'seatId', 'instanceId', 'repositoryId', 'worktreeId']) {
  check(config[key] === originalM2[key], `Existing E/F binding ${key} differs from the original M2 config.`);
}
const actualPath = value => fs.realpathSync.native(value).replace(/^\\\\\?\\/, '').toLowerCase();
const samePath = (left, right) => actualPath(left) === actualPath(right);
check(nonempty(originalM2.worktreeRoot) && samePath(config.workspacePath, originalM2.worktreeRoot),
  'Desktop workspace path must be the actual approved M2 E/F testbed worktree, not vendor cwd.');
const baseline = JSON.parse(fs.readFileSync(config.readyEvidence.path, 'utf8').replace(/^\uFEFF/, ''));
check(baseline.schema === 'gogoke.37.repair-preservation-metadata.v1' &&
  baseline.credentialsRead === false && baseline.rootStat &&
  fs.statSync(config.stateRoot).isDirectory(),
  'The closed candidate baseline or supplied normal-view state root is invalid.');
const baselineRootIdentity = execFileSync(config.python, ['-c',
  'import json,os,sys; s=os.stat(sys.argv[1]); b=json.load(open(sys.argv[2],encoding="utf-8"))["rootStat"]; print("MATCH" if s.st_dev==b["dev"] and s.st_ino==b["ino"] else "DIFFERENT")',
  config.stateRoot, config.readyEvidence.path], { encoding: 'utf8' }).trim();
check(baselineRootIdentity === 'MATCH',
  'Normal-view state root is not the closed candidate preserved physical root.');
check(config.observerTools && typeof config.observerTools === 'object' &&
  ['formal', 'memory', 'ledger'].every(name => config.observerTools[name]),
  'Original formal, memory and ledger observers are required.');
const observerSpecs = [
  { name: 'formal', runtime: config.pwsh,
    args: output => ['-NoProfile', '-NonInteractive', '-File', config.observerTools.formal.script,
      '-OutputFile', output] },
  { name: 'memory', runtime: config.python,
    args: output => [config.observerTools.memory.script,
      path.join(config.stateRoot, 'v37-instances', config.instanceId), output, '0'] },
  { name: 'ledger', runtime: config.python,
    args: output => [config.observerTools.ledger.script, config.stateRoot, output] },
];
for (const { name, runtime, args } of observerSpecs) {
  const observer = config.observerTools[name];
  const original = originalM2.observers.find(row => row.name === name);
  check(original && runtime === original.runtime &&
    JSON.stringify(args('{output}')) === JSON.stringify(original.args) &&
    Array.isArray(observer.equalFields) && observer.equalFields.length > 0 &&
    JSON.stringify(observer.equalFields) === JSON.stringify(original.equalFields) &&
    path.isAbsolute(observer.script) && hex64(observer.sha256) &&
    digest(observer.script) === observer.sha256,
  `Original ${name} observer runtime, fields or bytes differ.`);
}
check(Object.keys(config.observerTools.formal.companions ?? {}).sort().join('|') ===
  ['ordinary-package-identity.py', 'python-path.txt'].sort().join('|'),
  'Both original formal observer companions are required.');
for (const [name, sha256] of Object.entries(config.observerTools.formal.companions ?? {})) {
  const companion = path.join(path.dirname(config.observerTools.formal.script), name);
  check(hex64(sha256) && digest(companion) === sha256,
    `Original formal observer companion ${name} changed.`);
}
check(config.installedSha256 && typeof config.installedSha256 === 'object' &&
  ['gogoke.exe', 'gogoke-native-host.exe', 'resource-index.json']
    .every(name => hex64(config.installedSha256[name])),
  'Exact installed shell/host/resource-index hashes are required.');
const sourceRoot = path.resolve(config.sourceRoot);
const currentHead = execFileSync('git', ['rev-parse', 'HEAD'], { cwd: sourceRoot, encoding: 'utf8' }).trim();
if (currentHead !== SOURCE) {
  execFileSync('git', ['merge-base', '--is-ancestor', SOURCE, currentHead], { cwd: sourceRoot });
  const changed = execFileSync('git', ['diff', '--name-only', SOURCE, currentHead],
    { cwd: sourceRoot, encoding: 'utf8' }).trim().split(/\r?\n/).filter(Boolean);
  check(changed.every(file => file.startsWith('tools/e2e/') || file.startsWith('artifacts/gogoke-37/checkpoints/')),
    'Checkout differs from the frozen source beyond its recorded checkpoint.');
}
check(execFileSync('git', ['status', '--porcelain', '--',
  'tools/e2e/product-cdp.mjs', 'tools/e2e/e2e-webview.mjs',
  'tools/e2e/candidate-custody.ps1', 'tools/e2e/package-lock.json'],
{ cwd: sourceRoot, encoding: 'utf8' }).trim() === '',
  'Actual product-CDP driver dependencies differ from the reviewed source.');
const evidenceDirectory = path.resolve(config.evidenceDirectory);
check(within(evidenceDirectory, privateRoot) && evidenceDirectory !== privateRoot,
  'Evidence directory must be a child of this private config directory.');
check(!fs.existsSync(evidenceDirectory), 'Evidence directory already exists; no run overwrite.');
fs.mkdirSync(evidenceDirectory);
const result = path.join(evidenceDirectory, 'result.json');
const { ActualProduct, id, delay } = await import(pathToFileURL(
  path.join(sourceRoot, 'tools', 'e2e', 'product-cdp.mjs')).href);
const runConfig = { ...config, evidenceDirectory, result, testerArmy: true };
const journal = { schema: 'gogoke.37.native-visible-e2e.v1',
  sourceCommit: SOURCE, state: 'RUNNING', acceptance: false, agentActs: 0,
  baselineMetadata: { sha256: config.readyEvidence.sha256 },
  launches: [], closes: [], operations: [], sessions: [], ui: [], readbacks: [], snapshots: {} };
const product = new ActualProduct(runConfig, journal);
let workspaceId = null;
journal.driverBytes[path.basename(fileURLToPath(import.meta.url))] = digest(fileURLToPath(import.meta.url));
product.save();
const step = (name, value) => { journal.ui.push({ name, value, at: new Date().toISOString() }); product.save(); };
const exactAssociation = value => {
  const keys = ['domainId', 'sessionId', 'seatId', 'incarnation',
    'authorizationGeneration', 'bindingGeneration', 'instanceId'];
  check(value && typeof value === 'object' && !Array.isArray(value) &&
    Object.keys(value).length === keys.length && keys.every(key => nonempty(value[key])),
  'The original native association is not the exact seven-field identity.');
  return value;
};
const canonical = value => Array.isArray(value) ? value.map(canonical) :
  value && typeof value === 'object' ? Object.fromEntries(
    Object.keys(value).sort().map(key => [key, canonical(value[key])])) : value;
const same = (a, b) => JSON.stringify(canonical(a)) === JSON.stringify(canonical(b));
async function snapshot(phase) {
  for (const observer of observerSpecs) {
    const output = path.join(evidenceDirectory, `${observer.name}-${phase}.json`);
    check(!fs.existsSync(output), `Original ${observer.name}-${phase} snapshot already exists.`);
    const args = observer.args(output);
    await new Promise((resolve, reject) => {
      const child = spawn(observer.runtime, args, { windowsHide: true,
        stdio: ['ignore', 'ignore', 'pipe'] });
      let stderr = '';
      child.stderr.on('data', bytes => { stderr = (stderr + bytes).slice(-8192); });
      child.once('error', reject);
      child.once('exit', code => code === 0 ? resolve() :
        reject(Error(`Original read-only ${observer.name} observer exit=${code}: ${stderr}`)));
    });
    journal.snapshots[`${observer.name}-${phase}`] = {
      file: path.basename(output), sha256: digest(output) };
    product.save();
  }
}
function snapshotValue(name, phase) {
  const reference = journal.snapshots[`${name}-${phase}`];
  check(reference && digest(path.join(evidenceDirectory, reference.file)) === reference.sha256,
    `Original ${name}-${phase} snapshot hash differs.`);
  return JSON.parse(fs.readFileSync(path.join(evidenceDirectory, reference.file), 'utf8').replace(/^\uFEFF/, ''));
}
function compareSnapshots() {
  for (const { name } of observerSpecs) {
    const before = snapshotValue(name, 'before');
    const after = snapshotValue(name, 'after');
    for (const field of config.observerTools[name].equalFields) {
      check(Object.hasOwn(before, field) && Object.hasOwn(after, field) &&
        same(before[field], after[field]), `Original ${name}.${field} preservation differs.`);
    }
  }
  for (const phase of ['before', 'after']) {
    const memory = snapshotValue('memory', phase);
    check(memory.memoryDataUnchangedByRead === true && memory.stage1OutputCount === 0 &&
      memory.memoryJobCount === 0,
    `Original memory observer ${phase} is not non-mutating and empty of jobs.`);
  }
}
const invoke = (command, args = {}) => product.evaluate(
  `window.__TAURI_INTERNALS__.invoke(${JSON.stringify(command)},${JSON.stringify(args)})`);
async function userFrame(frame) {
  const record = { frame, rawFrame: JSON.stringify(frame), rawReply: null, reply: null,
    startedAt: new Date().toISOString() };
  journal.operations.push(record); product.save();
  record.rawReply = await invoke('gogoke_design37_user_operation', { frame: record.rawFrame });
  check(typeof record.rawReply === 'string', 'Original USER response is not a raw frame.');
  record.reply = JSON.parse(record.rawReply);
  record.finishedAt = new Date().toISOString(); product.save();
  return record.reply;
}
const visible = (command, fields = {}) => userFrame({
  schema: 'gogoke.37.owner-configuration.v1', command, workspaceId, ...fields,
});
const page = () => {
  check(product.tester && product.tester.page.url() === product.endpoint.url,
    'The exact installed WebView changed or tester connection is absent.');
  return product.tester.page;
};
async function eventually(label, read, predicate, ms = 30000) {
  const deadline = Date.now() + ms;
  let last;
  while (Date.now() < deadline) {
    last = await read();
    if (predicate(last)) return last;
    await delay(150);
  }
  fail(`${label} was not observed before deadline; last=${JSON.stringify(last)}`);
}
async function seatCard() {
  let reply = await product.operation('K-SEAT', 'state-card', config.seatId, {}, '0', ['APPLIED', 'STALE']);
  if (reply.status === 'STALE') reply = await product.operation(
    'K-SEAT', 'state-card', config.seatId, {}, reply.revision);
  return reply;
}
async function hOperation(session, operation, payload = {}, allowed = ['APPLIED']) {
  const reply = await product.operation('K-SESSION', operation, session.id,
    { generation: session.generation, ...payload }, session.revision, allowed);
  session.revision = reply.revision;
  if (reply.result?.newGeneration) session.generation = reply.result.newGeneration;
  product.save();
  return reply;
}
async function hOutput(session) {
  let reply = await hOperation(session, 'output-stream', { afterCursor: session.cursor }, ['APPLIED', 'STALE']);
  if (reply.status === 'STALE') reply = await hOperation(session, 'output-stream',
    { afterCursor: session.cursor }); // New read-only request after a stale revision, never a model replay.
  check(!reply.result?.sourceError && /^\d+$/.test(String(reply.result?.cursor ?? '')) &&
    BigInt(reply.result.cursor) >= BigInt(session.cursor), 'Original H output readback is unresolved.');
  session.cursor = reply.result.cursor;
  journal.readbacks.push({ kind: 'H-output', cursor: session.cursor,
    eventCount: Array.isArray(reply.result.events) ? reply.result.events.length : null });
  product.save(); return reply.result;
}
const nativeHistoryFacts = new Map();
async function nativeHistory(threadId) {
  const value = await invoke('read_thread', { workspaceId, threadId });
  check(value?.result?.thread?.id === threadId && Array.isArray(value.result.thread.turns) &&
    value.result.nativeHistory?.state === 'COMPLETE', 'Original full native thread/read is incomplete.');
  nativeHistoryFacts.set(threadId, structuredClone(value.result.nativeHistory));
  journal.readbacks.push({ kind: 'native-history', threadId,
    history: structuredClone(value.result.nativeHistory) });
  product.save();
  return value.result.thread;
}
async function assertInterruptedPartial(thread, turnId) {
  const history = nativeHistoryFacts.get(thread.id);
  const turns = thread.turns.filter(turn => turn.id === turnId);
  check(turns.length === 1 && turns[0].status === 'interrupted' && turns[0].itemsView === 'notLoaded',
    'Original accepted interrupt lacks its interrupted/notLoaded vendor terminal.');
  check(Array.isArray(history?.partialMessages), 'Original native history has no partial-message sidecar.');
  const partials = history.partialMessages.filter(item => item.turnId === turnId);
  check(partials.length > 0 && partials.every(item => item.kind === 'interruptedAgentMessage' &&
    nonempty(item.itemId) && typeof item.text === 'string' && Array.isArray(item.sourceRefs) &&
    item.sourceRefs.length >= 2 && !turns[0].items.some(final => final.id === item.itemId)),
    'Original interrupted partial output is missing or substituted for a final vendor item.');
  for (const partial of partials) {
    const label = partial.text ? '中断时的部分输出（未收到最终消息）' : '中断前尚未收到文本（未收到最终消息）';
    const expectedId = `gogoke-partial:${JSON.stringify([thread.id, turnId, partial.itemId])}`;
    const expectedText = partial.text ? `${label}\n\n${partial.text}` : label;
    await eventually('source-qualified interrupted partial in actual UI',
      () => product.evaluate(`(() => {
        const expectedId=${JSON.stringify(expectedId)}, expectedText=${JSON.stringify(expectedText)};
        return [...document.querySelectorAll('.message.assistant')].some(element => {
          if (!element.querySelector('.message-bubble')?.textContent?.includes(${JSON.stringify(label)})) return false;
          const key=Object.keys(element).find(key=>key.startsWith('__reactFiber$'));
          for(let fiber=key?element[key]:null;fiber;fiber=fiber.return) {
            const item=fiber.memoizedProps?.item;
            if(item?.id===expectedId) return item.role==='assistant' && item.text===expectedText;
          }
          return false;
        });
      })()`), matched => matched === true);
  }
  return structuredClone(partials);
}
function assertCurrentHistory(thread, originalSend) {
  const turnId = originalSend?.response?.result?.turn?.id;
  check(nonempty(turnId), 'Original USER send has no vendor turn ID; do not infer it.');
  const turns = thread.turns.filter(turn => turn.id === turnId);
  check(turns.length === 1 && Array.isArray(turns[0].items) &&
    turns[0].items.some(item => item.type === 'userMessage' &&
      Array.isArray(item.content) && item.content.some(input =>
        input.type === 'text' && input.text === config.askPrompt)),
    'Original full host history lacks this exact accepted vendor turn and USER input.');
}
const visibleKey = () => `gogoke.native-visible-original.${encodeURIComponent(workspaceId)}`;
const visibleJournal = () => product.evaluate(
  `JSON.parse(localStorage.getItem(${JSON.stringify(visibleKey())})||'null')`);
async function actualUiWrite(command, trigger, expected) {
  const before = await visibleJournal();
  const accepted = new Set([...(before?.acceptedRequestIds ?? []),...(before?.rejectedRequestIds ?? []),...(before?.notDispatchedRequestIds ?? []),before?.lastAccepted?.nativeRequestId,before?.lastRejected?.nativeRequestId,before?.lastNotDispatched?.nativeRequestId,before?.pending?.nativeRequestId].filter(nonempty));
  check(!before?.pending, 'A previous original UI write is still unresolved; no new write.');
  await trigger();
  const after = await eventually(`${command} original UI write`, visibleJournal, value => {
    const last = value?.lastAccepted;
    if (last && !accepted.has(last.nativeRequestId) && last.command === command) return true;
    const pending = value?.pending;
    if (pending && !accepted.has(pending.nativeRequestId) && pending.error) {
      fail(`Original ${command} outcome UNKNOWN: ${pending.error}; no resend.`);
    }
    const rejected = value?.lastRejected;
    if (rejected && !accepted.has(rejected.nativeRequestId) && rejected.command === command) {
      fail(`Original ${command} rejected: ${rejected.reason}; no resend.`);
    }
    const notDispatched = value?.lastNotDispatched;
    if (notDispatched && !accepted.has(notDispatched.nativeRequestId) &&
        notDispatched.command === command) {
      fail(`Original ${command} was not dispatched: ${notDispatched.reason}; no resend.`);
    }
    if (pending && !accepted.has(pending.nativeRequestId)) {
      journal.pendingOriginalUiWrite = pending; product.save();
    }
    return false;
  }, 180000); // Observe the existing native operation deadline; never resend on expiry.
  const original = after.lastAccepted;
  check(original.command === command && original.workspaceId === workspaceId,
    `Original ${command} identity changed.`);
  check(original.payload?.workspaceId === workspaceId &&
    original.payload?.threadId === expected.threadId &&
    (!expected.turnId || original.payload?.turnId === expected.turnId),
    'Accepted write differs from the exact original workspace/thread/turn.');
  check(same(exactAssociation(original.expectedAssociation), expected.association) &&
    same(expected.association,
      exactAssociation((await visible('visible-conversation-route')).association)),
    'Accepted UI write association differs from the original saved USER choice.');
  delete journal.pendingOriginalUiWrite; product.save();
  step(command, { nativeRequestId: original.nativeRequestId, response: original.response });
  return original;
}
async function workspaceCard(workspaceName) {
  const cards = page().locator('.workspace-card');
  const matching = [];
  for (let index = 0; index < await cards.count(); index++) {
    if ((await cards.nth(index).locator('.workspace-name').textContent())?.trim() === workspaceName) {
      matching.push(cards.nth(index));
    }
  }
  check(matching.length === 1, 'Actual sidebar workspace name is not unique.');
  return matching[0];
}
async function clickUniqueRow(workspaceName) {
  const targetThread = journal.sessions[0].threadId;
  const index = await eventually('exact original selected sidebar row', () => product.evaluate(`(() => {
    const rows = [...document.querySelectorAll('.thread-row')];
    const indices = [];
    rows.forEach((element, index) => {
      const key = Object.keys(element).find(k => k.startsWith('__reactFiber$'));
      let fiber = element[key];
      for (let depth = 0; fiber && depth < 25; depth++, fiber = fiber.return) {
        const props = fiber.memoizedProps;
        if (props?.thread?.id === ${JSON.stringify(targetThread)} && props.workspaceId === ${JSON.stringify(workspaceId)}) {
          indices.push(index); break;
        }
      }
    });
    if (indices.length > 1) throw Error('Original selected thread row is duplicated: ' + JSON.stringify(indices));
    return indices.length === 1 ? indices[0] : null;
  })()`), value => Number.isInteger(value));
  await page().locator('.thread-row').nth(index).click();
}
async function connectSelected(association, workspaceName, requireClick, threadId) {
  const route = await visible('visible-conversation-route');
  check(route.schema === 'gogoke.37.visible-conversation.v1' && route.state === 'NATIVE' &&
    route.workspaceId === workspaceId && same(exactAssociation(route.association), association),
  `Saved explicit USER route differs: ${JSON.stringify(route)}`);
  const card = await workspaceCard(workspaceName);
  const connect = card.locator('.connect');
  if (requireClick) check(await connect.count() === 1,
    'Actual workspace Connect UI is absent before the first attachment.');
  if (await connect.count() === 1) await connect.click();
  const transport = await eventually('actual native workspace attachment',
    () => invoke('native_visible_transport', { workspaceId }),
    value => value?.state === 'NATIVE');
  check(transport?.state === 'NATIVE' && same(exactAssociation(transport.association), association),
    `Actual native attachment differs: ${JSON.stringify(transport)}`);
  step('actual-connect', { workspaceId, association });
  const refresh = page().locator('.sidebar-refresh-toggle');
  check(await refresh.count() === 1, 'Actual sidebar refresh control is absent.');
  await eventually('connected sidebar refresh readiness', () => refresh.isEnabled(), enabled => enabled === true);
  await refresh.click();
  const toggle = card.locator('.workspace-toggle');
  if (await toggle.getAttribute('aria-expanded') === 'false') await toggle.click();
  const listing = await invoke('list_threads', { workspaceId,
    cursor: null, limit: 100, sortKey: null });
  check(listing?.result?.nativeHistory?.state === 'COMPLETE' &&
    listing.result.nextCursor === null && Array.isArray(listing.result.data) &&
    listing.result.data.filter(row => row.id === threadId).length === 1,
  `Selected workspace must contain the exact original H thread: ${JSON.stringify(listing)}`);
  await eventually('native sidebar rows', () => card.locator('.thread-row').count(), count => count > 0);
}
let hStopped = false;
try {
  await snapshot('before'); // Product is still normally closed.
  await product.launch();
  check(product.tester && journal.connectionBackend?.agentActs === 0,
    'Actual installed WebView CDP has no zero-agent-action locator.');
  const instances = await product.instances(); // Status only; no credential read or login.
  const instance = instances.instances.filter(row => row.instanceId === config.instanceId &&
    row.driverId === 'codex' && row.version === config.cliVersion);
  check(instance.length === 1 && instance[0].state === 'LOGGED_IN',
    'Existing authenticated M2Ready Codex instance is not available.');
  let workspaces = await invoke('list_workspaces');
  check(Array.isArray(workspaces), 'Actual desktop list_workspaces is not an array.');
  let workspace = workspaces.filter(row => nonempty(row.path) &&
    samePath(row.path, config.workspacePath));
  if (workspace.length === 0) {
    // Real USER command at the approved testbed; no metadata-file injection.
    const registered = await invoke('add_workspace', { path: config.workspacePath });
    check(nonempty(registered?.id) && samePath(registered.path, config.workspacePath) &&
      registered.connected === false, 'Original first registration is not disconnected.');
    step('actual-USER-disconnected-registration', { workspaceId: registered.id });
    // No new H session has been opened by this driver yet. Re-read actual startup UI state.
    await product.closeNormally();
    await product.launch();
    workspaces = await invoke('list_workspaces');
    check(Array.isArray(workspaces), 'Restarted product workspace read is not an array.');
    workspace = workspaces.filter(row => row.id === registered.id && nonempty(row.path) &&
      samePath(row.path, config.workspacePath));
    check(workspace.length === 1 && workspace[0].connected === false,
      'Actual first registration did not survive ordinary restart.');
  }
  check(workspace.length === 1 && nonempty(workspace[0].id),
    'No unique actual desktop workspace row for the approved E/F testbed path. Register it through the real product add-workspace flow before this bounded conversation driver; do not inject a row or guess an identity.');
  workspaceId = workspace[0].id;
  check(!config.workspaceId || config.workspaceId === workspaceId,
    'Optional expected desktop workspace ID differs from the actual producer row.');
  step('actual-workspace-row', { workspaceId, path: workspace[0].path });
  check(workspace[0].connected === false,
    'Target workspace is already connected before the required explicit USER choice.');
  const beforeChoiceTransport = await invoke('native_visible_transport', { workspaceId });
  check(beforeChoiceTransport?.state === 'DISCONNECTED',
    `Target transport is not disconnected before explicit choice: ${JSON.stringify(beforeChoiceTransport)}`);
  const beforeChoiceRoute = await visible('visible-conversation-route');
  if (config.settledRelease?.alreadyReleased) {
    check(beforeChoiceRoute.state === 'NATIVE',
      'The previously released original USER selection must still be identifiable.');
  }
  if (beforeChoiceRoute.state === 'NATIVE') {
    const old = config.settledRelease;
    const prior = exactAssociation(beforeChoiceRoute.association);
    check(old && prior.sessionId === old.sessionId && prior.domainId === config.domainId &&
      prior.seatId === config.seatId && prior.instanceId === config.instanceId &&
      prior.bindingGeneration === old.generation, 'Retained selection differs from original stopped session.');
    if (old.alreadyReleased) {
      const originals = old.evidence;
      for (const reference of [originals?.stop, originals?.release]) {
        check(reference && path.isAbsolute(reference.path) && hex64(reference.sha256) &&
          digest(reference.path) === reference.sha256, 'Original stop/release evidence is absent or changed.');
      }
      const stopped = JSON.parse(fs.readFileSync(originals.stop.path, 'utf8').replace(/^\uFEFF/, ''));
      const released = JSON.parse(fs.readFileSync(originals.release.path, 'utf8').replace(/^\uFEFF/, ''));
      const stoppedSource = old.sourceCommit ?? SOURCE;
      check(typeof stoppedSource === 'string' && /^[a-f0-9]{40}$/.test(stoppedSource),
        'Original stopped candidate source is not an exact Git commit.');
      execFileSync('git', ['merge-base', '--is-ancestor', stoppedSource, SOURCE],
        { cwd: sourceRoot, stdio: 'pipe' });
      const settled = released.state === 'EVIDENCE_READY_REQUIRES_REVIEW' ||
        (released.state === 'SETTLED_WITH_ORIGINAL_HISTORY_RESULT' &&
          released.historyResult === 'FAILED_ORIGINAL_HISTORY_READ' &&
          nonempty(released.originalColdHistoryError));
      const stop = stopped.receipt;
      const release = released.operations.filter(row => row.request?.family === 'K-SESSION' &&
        row.request.operation === 'admission-release' && row.request.targetId === old.sessionId &&
        row.request.payload.generation === old.generation);
      check(stop?.family === 'K-SESSION' && stop.operation === 'stop' &&
        stop.targetId === old.sessionId && stop.status === 'APPLIED' && nonempty(stop.result?.stopFact) &&
        nonempty(stop.requestId) && stop.schema === 'gogoke.37.operations.v1' &&
        stop.requestId === stopped.request?.requestId && stopped.request.domainId === config.domainId &&
        stopped.request.family === 'K-SESSION' && stopped.request.operation === 'stop' &&
        stopped.request.expectedRevision === stop.previousRevision &&
        stopped.request.targetId === old.sessionId && stopped.request.payload.seatId === config.seatId &&
        stopped.request?.payload?.generation === old.generation &&
        stopped.originalStoppedLive?.state === 'APPLIED' && stopped.originalStoppedLive.live?.state === 'STOPPED' &&
        stopped.originalStoppedLive.schema === 'gogoke.37.visible-conversation.v1' &&
        stopped.originalStoppedLive.workspaceId === workspaceId && !stopped.originalStoppedLive.reason &&
        same(exactAssociation(stopped.originalStoppedLive.association), prior) &&
        released.sourceCommit === stoppedSource && settled &&
        release.length === 1 && release[0].receipt?.status === 'APPLIED' &&
        release[0].request.domainId === config.domainId && release[0].request.payload.seatId === config.seatId &&
        nonempty(release[0].receipt.requestId) && release[0].receipt.schema === 'gogoke.37.operations.v1' &&
        release[0].receipt.requestId === release[0].request.requestId &&
        release[0].receipt.targetId === old.sessionId && release[0].receipt.family === 'K-SESSION' &&
        release[0].receipt.operation === 'admission-release' &&
        release[0].request.expectedRevision === release[0].receipt.previousRevision &&
        release[0].receipt.previousRevision === stop.revision && release[0].receipt.revision === old.revision &&
        released.closes.length === 1 && released.closes[0].exitCode === 0 && released.closes[0].forceKill === false,
        'Original physical stop and later admission release do not match the retained selection.');
      step('retained-selection-original-stop-and-release', { sessionId: prior.sessionId,
        stopFact: stop.result.stopFact, releaseRevision: old.revision });
    } else {
      const live = await visible('visible-conversation-read', { expectedAssociation: prior, method: 'live-state' });
      check(live.schema === 'gogoke.37.visible-conversation.v1' && live.workspaceId === workspaceId && !live.reason && live.state === 'APPLIED' && live.live?.state === 'STOPPED' &&
        same(exactAssociation(live.association), prior), 'Retained selection is not original H-proven STOPPED.');
      step('retained-selection-original-H-STOPPED', { sessionId: prior.sessionId });
    }
  }
  if (config.settledRelease && !config.settledRelease.alreadyReleased) {
    const old = config.settledRelease;
    const released = await product.operation('K-SESSION', 'admission-release', old.sessionId,
      { generation: old.generation, seatId: config.seatId }, old.revision, ['APPLIED']);
    check(released.result && released.status === 'APPLIED', 'Original stopped admission release not confirmed.');
    step('original-stopped-admission-release', { requestId: released.requestId, revision: released.revision });
  }
  const card = await seatCard();
  check(card.result?.state === 'IDLE' && card.result.instanceId === config.instanceId &&
    /^\d+$/.test(String(card.result.generation)), 'Existing E USER seat is not idle on its registered instance.');
  const session = { id: id('g37UserSession'), seatId: config.seatId,
    generation: (BigInt(card.result.generation) + 1n).toString(), revision: '0', cursor: '0', threadId: null };
  journal.sessions.push(session); product.save();
  await hOperation(session, 'admission-reserve', { seatId: config.seatId });
  await hOperation(session, 'admission-commit', { seatId: config.seatId });
  if (config.startupObservationMarker) fs.writeFileSync(config.startupObservationMarker, JSON.stringify({ state: 'BEFORE_ORIGINAL_OPEN', sourceCommit: SOURCE, sessionId: session.id, domainId: config.domainId, at: new Date().toISOString() }) + '\n');
  const opened = await hOperation(session, 'open', { seatId: config.seatId,
    repositoryId: config.repositoryId, worktreeId: config.worktreeId });
  check(nonempty(opened.result?.threadId), 'Original K-SESSION open has no vendor thread ID.');
  session.threadId = opened.result.threadId; product.save();
  const choices = await visible('visible-conversation-choices');
  check(choices.schema === 'gogoke.37.visible-conversation.v1' &&
    choices.workspaceId === workspaceId && choices.state === 'APPLIED' &&
    Array.isArray(choices.choices), `No qualified original USER choices: ${JSON.stringify(choices)}`);
  const qualified = choices.choices.filter(row => row?.state === 'NATIVE' &&
    row.repositoryId === config.repositoryId && row.worktreeId === config.worktreeId &&
    row.threadId === session.threadId && row.association?.domainId === config.domainId &&
    row.association?.sessionId === session.id && row.association?.seatId === config.seatId &&
    row.association?.instanceId === config.instanceId &&
    row.association?.bindingGeneration === session.generation);
  check(qualified.length === 1, `Original exact H/E/F choice is not unique: ${JSON.stringify(choices)}`);
  const association = exactAssociation(qualified[0].association);
  const selectId = id('g37Select');
  const selected = await visible('visible-conversation-select', { requestId: selectId,
    route: 'NATIVE', repositoryId: config.repositoryId, threadId: session.threadId, association });
  check(selected.state === 'NATIVE' && selected.requestId === selectId &&
    same(exactAssociation(selected.association), association),
  `Original explicit USER selection failed: ${JSON.stringify(selected)}`);
  // The selected thread identity is established by the original H open, not the DOM.
  await invoke('connect_workspace', { id: workspaceId });
  step('original-USER-explicit-view-reattach', { workspaceId, association });
  await connectSelected(association, workspace[0].name, false, session.threadId);
  const initial = await nativeHistory(session.threadId);
  await clickUniqueRow(workspace[0].name);
  step('host-history-and-sidebar-row-observed', { threadId: session.threadId, turns: initial.turns.length });
  const uiHydration = await eventually('actual React native hydration before input', () => product.evaluate(`(() => {
    const element = document.querySelector('.composer-action.is-send');
    if (!element) return null;
    const key = Object.keys(element).find(k => k.startsWith('__reactFiber$'));
    const nativeKey = ${JSON.stringify(workspaceId + ':' + session.threadId)};
    const expectedAssociation = ${JSON.stringify(JSON.stringify(association))};
    let fiber = element[key], state = null, hydrated = false;
    for (let layer = 0; fiber && layer < 45; layer++, fiber = fiber.return) {
      let hook = fiber.memoizedState;
      for (let index = 0; hook && index < 1200; index++, hook = hook.next) {
        const value = hook.memoizedState;
        if (value && typeof value === 'object' && value.activeThreadIdByWorkspace && value.threadResumeLoadingById) {
          state = { activeThreadId: value.activeThreadIdByWorkspace[${JSON.stringify(workspaceId)}],
            loading: value.threadResumeLoadingById[${JSON.stringify(session.threadId)}] };
        }
        if (value && typeof value === 'object' && value.current &&
          typeof value.current[nativeKey] === 'string') {
          try {
            const actual = JSON.parse(value.current[nativeKey]);
            const expected = JSON.parse(expectedAssociation);
            const fields = Object.keys(expected);
            if (actual && Object.keys(actual).length === fields.length &&
                fields.every(field => actual[field] === expected[field])) hydrated = true;
          } catch { /* Not an association ref; continue the read-only scan. */ }
        }
      }
    }
    return { sourceUrl: location.href, state, hydrated };
  })()`), value => value?.hydrated === true && value.state?.activeThreadId === session.threadId && value.state?.loading === false);
  step('actual-React-native-hydration-before-input', uiHydration);
  const textarea = page().locator('.composer textarea');
  check(await textarea.count() === 1, 'Actual composer textarea is absent.');
  await textarea.fill(config.askPrompt);
  const originalSend = await actualUiWrite('send_user_message', async () => {
    const send = page().locator('.composer-action.is-send');
    check(await send.count() === 1, 'Actual ordinary composer Send control is absent.');
    await send.click();
  }, { threadId: session.threadId, association });
  await eventually('actual USER message projection',
    () => page().locator('.message.user .message-bubble').allTextContents(),
    rows => rows.some(text => text.includes(config.askPrompt)));
  await eventually('original active turn and visible Steer shortcut', async () => ({
    stop: await page().locator('.composer-action.is-stop').count(),
    hint: await product.evaluate('document.querySelector(".composer-followup-hint")?.textContent ?? null'),
  }), value => value.stop === 1 && typeof value.hint === 'string' &&
    value.hint.includes('Shift+Ctrl+Enter'));
  if (config.steerBeforeInterrupt !== false) {
    await textarea.fill(config.steerPrompt);
    await actualUiWrite('turn_steer', () => textarea.press('Control+Shift+Enter'),
      { threadId: session.threadId, turnId: originalSend.response.result.turn.id, association });
  }
  if (config.requireInterruptedPartial === true) {
    await eventually('original agent delta before exercising partial interruption',
      () => hOutput(session), output => Array.isArray(output.events) && output.events.some(event =>
        event.sessionUpdate === 'agent_message_chunk' && event.content?.type === 'text' &&
        nonempty(event.content.text) && event._meta?.codexMethod === 'item/agentMessage/delta' &&
        event._meta?.threadId === session.threadId &&
        event._meta?.turnId === originalSend.response.result.turn.id &&
        nonempty(event._meta?.itemId) && /^[1-9][0-9]*$/.test(event._meta?.rawSourceCursor ?? '')));
    step('original-agent-delta-observed-before-interrupt', { threadId: session.threadId,
      turnId: originalSend.response.result.turn.id });
  }
  await actualUiWrite('turn_interrupt', async () => {
    const stop = page().locator('.composer-action.is-stop');
    check(await stop.count() === 1, 'Original active turn completed before UI interrupt; no substitute stop.');
    await stop.click();
  }, { threadId: session.threadId, turnId: originalSend.response.result.turn.id, association });
  const writesBeforeRefresh = (await visibleJournal())?.acceptedRequestIds?.length;
  await clickUniqueRow(workspace[0].name); // Native setActiveThreadId forces the read-only full history path.
  check((await visibleJournal())?.acceptedRequestIds?.length === writesBeforeRefresh,
    'Read-only UI refresh unexpectedly created a native USER write.');
  const interruptedHistory = await nativeHistory(session.threadId);
  if (config.requireInterruptedPartial === true) {
    await assertInterruptedPartial(interruptedHistory, originalSend.response.result.turn.id);
  }
  await hOutput(session); // Original H read updates its revision after UI writes.
  const stopped = await hOperation(session, 'stop', { seatId: config.seatId }, ['APPLIED', 'STALE']);
  check(stopped.status === 'APPLIED' && nonempty(stopped.result?.stopFact),
    `Original H physical stop is not confirmed: ${JSON.stringify(stopped)}`);
  const live = await visible('visible-conversation-read', { expectedAssociation: association,
    method: 'live-state', params: {} });
  check(live.schema === 'gogoke.37.visible-conversation.v1' &&
    live.workspaceId === workspaceId && live.state === 'APPLIED' &&
    same(exactAssociation(live.association), association) &&
    live.live?.state === 'STOPPED' && !live.reason,
    `Original H physical stop readback is incomplete: ${JSON.stringify(live)}`);
  hStopped = true;
  step('original-H-stop-readback', { stopFact: stopped.result.stopFact, live: live.live });
  await hOutput(session);
  const beforeStop = await nativeHistory(session.threadId);
  const beforeStopMetadata = structuredClone(nativeHistoryFacts.get(session.threadId));
  assertCurrentHistory(beforeStop, originalSend);
  await product.closeNormally();
  await product.launch();
  await product.instances();
  await connectSelected(association, workspace[0].name, false, session.threadId);
  await clickUniqueRow(workspace[0].name);
  const cold = await nativeHistory(session.threadId);
  assertCurrentHistory(cold, originalSend);
  check(same(cold.turns, beforeStop.turns), 'Cold original full-thread history differs from preclose readback.');
  if (config.requireInterruptedPartial === true) {
    check(same(nativeHistoryFacts.get(session.threadId).partialMessages, beforeStopMetadata.partialMessages),
      'Cold source-qualified interrupted partial output differs from its original preclose readback.');
    await assertInterruptedPartial(cold, originalSend.response.result.turn.id);
  }
  await eventually('cold UI original USER message',
    () => page().locator('.message.user .message-bubble').allTextContents(),
    rows => rows.some(text => text.includes(config.askPrompt)));
  step('cold-hydration', { threadId: session.threadId, turns: cold.turns.length });
  const released = await hOperation(session, 'admission-release', { seatId: config.seatId });
  check(released.status === 'APPLIED', `Original H admission release failed: ${JSON.stringify(released)}`);
  await product.closeNormally();
  await snapshot('after'); // Product has normally exited after original H physical stop.
  compareSnapshots();
  journal.state = 'EVIDENCE_READY_REQUIRES_REVIEW'; product.save();
} catch (error) {
  journal.state = 'FAIL';
  journal.error = error instanceof Error ? (error.stack ?? error.message) : String(error);
  journal.hPhysicalStopReadback = hStopped;
  product.save();
  if (product.child && product.child.exitCode === null) await product.preserveFailure();
  throw error;
}
