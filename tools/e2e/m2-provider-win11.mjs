// Standalone real installed-provider capture. This is a development driver,
// not a host/model substitute or an M2 acceptance decision.
import fs from 'node:fs';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { ActualProduct, readJson, sha256, id, delay } from './product-cdp.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const drivers = ['claude', 'opencode', 'grok'];
const atom = value => typeof value === 'string' && /^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/.test(value);
const inside = (child, parent) => child === parent || child.startsWith(parent + path.sep);
const config = readJson(process.argv[2]);
const required = ['installed', 'installedSha256', 'version', 'sourceCommit', 'registryKey', 'pwsh',
  'python', 'stateRoot', 'evidenceDirectory', 'result', 'domainId', 'repositoryId', 'observers', 'cases'];
if (process.platform !== 'win32' || !process.argv[2] || required.some(key => config[key] === undefined) ||
    config.repositoryId !== 'gogokeSeatTestbed' || !atom(config.domainId) ||
    !/^[a-f0-9]{40}$/.test(config.sourceCommit) || typeof config.version !== 'string' || !config.version ||
    !Array.isArray(config.cases) || config.cases.length !== 3 ||
    new Set(config.cases.map(row => row.driverId)).size !== 3 ||
    new Set(config.cases.map(row => row.seatId)).size !== 3 ||
    new Set(config.cases.map(row => row.worktreeId)).size !== 3 ||
    !drivers.every(driverId => config.cases.some(row => row.driverId === driverId)) ||
    new Set(config.cases.map(row => row.instanceId)).size !== 3 ||
    config.cases.some(row => ![row.instanceId, row.seatId, row.worktreeId].every(atom) ||
      typeof row.version !== 'string' || !/^[a-f0-9]{64}$/.test(row.sha256) ||
      typeof row.model !== 'string' || !row.model.trim() || typeof row.effort !== 'string' || !row.effort.trim()) ||
    !Array.isArray(config.observers) ||
    !['formal', 'memory', 'ledger'].every(name => config.observers.some(row => row.name === name)) ||
    config.observers.some(row => typeof row.runtime !== 'string' || !Array.isArray(row.args) ||
      !Array.isArray(row.equalFields) || row.equalFields.length === 0) ||
    !config.observers.find(row => row.name === 'formal').equalFields?.includes('formal') ||
    !['formal', 'registeredFormal', 'formalData', 'formalRegistry', 'shortcuts']
      .every(field => config.observers.find(row => row.name === 'formal').equalFields.includes(field)) ||
    fs.existsSync(config.result) || !fs.existsSync(config.evidenceDirectory) ||
    ![config.installed, config.stateRoot, config.evidenceDirectory, config.result].every(path.isAbsolute) ||
    !inside(path.resolve(config.result), path.resolve(config.evidenceDirectory)) ||
    [config.installed, config.stateRoot, path.resolve(here, '..', '..')]
      .some(root => inside(path.resolve(config.evidenceDirectory), path.resolve(root)))) {
  throw Error('Fresh private provider E2E config, exact installed product, three fixed provider cases and observers required');
}
if (config.cases.find(row => row.driverId === 'opencode')?.model.toLowerCase().includes('gpt') ||
    !/(grok|xai)/i.test(config.cases.find(row => row.driverId === 'opencode')?.model ?? '')) {
  throw Error('OpenCode case must use its configured xAI/Grok model; GPT is not an eligible substitute');
}

const journal = { schema: 'gogoke.37.m2-provider-win11-e2e.v1', caseId: id('m2ProviderE2E'),
  sourceCommit: config.sourceCommit, domainId: config.domainId, repositoryId: config.repositoryId,
  installedVersion: config.version, installedSha256: config.installedSha256,
  state: 'RUNNING', acceptance: false, authenticationActions: false, observerDatabaseWrites: false,
  hostOperationsWriteCandidateDatabase: true, credentialReads: false, b5: 'NOT_RUN_UNSUPPORTED',
  marker: id('M2_PROVIDER_MARKER'), cases: [], operations: [], sessions: [],
  snapshots: {}, readbacks: [], goldens: [], assertions: [] };
const product = new ActualProduct(config, journal);
const check = (condition, reason) => {
  if (!condition) throw Error(reason);
  if (!journal.assertions.includes(reason)) { journal.assertions.push(reason); product.save(); }
};
const operation = (...args) => product.operation(...args);

function observerValue(name, phase) {
  const reference = journal.snapshots[`${name}-${phase}`];
  const file = path.join(config.evidenceDirectory, reference.file);
  check(sha256(file) === reference.sha256, `Original ${name}-${phase} observer bytes`);
  return readJson(file);
}

async function snapshot(phase) {
  for (const observer of config.observers) {
    const output = path.join(config.evidenceDirectory, `${observer.name}-provider-${phase}.json`);
    if (fs.existsSync(output)) throw Error(`Original ${observer.name} ${phase} observer exists`);
    const args = observer.args.map(value => value === '{output}' ? output : value);
    await new Promise((resolve, reject) => {
      const child = spawn(observer.runtime, args, { windowsHide: true, stdio: ['ignore', 'ignore', 'pipe'] });
      let stderr = ''; child.stderr.on('data', bytes => { stderr = (stderr + bytes).slice(-8192); });
      child.once('error', reject);
      child.once('exit', code => code === 0 ? resolve() : reject(Error(`Readonly ${observer.name} ${phase} exit=${code}: ${stderr}`)));
    });
    journal.snapshots[`${observer.name}-${phase}`] = { file: path.basename(output), sha256: sha256(output) };
    product.save();
  }
}

async function seatCard(seatId) {
  let reply = await operation('K-SEAT', 'state-card', seatId, {}, '0', ['APPLIED', 'STALE']);
  if (reply.status === 'STALE') reply = await operation('K-SEAT', 'state-card', seatId, {}, reply.revision);
  return reply;
}
async function graph(row) {
  let reply = await operation('K-WORKTREE', 'graph-query', row.worktreeId, {}, '0', ['APPLIED', 'STALE']);
  if (reply.status === 'STALE') reply = await operation('K-WORKTREE', 'graph-query', row.worktreeId, {}, reply.revision);
  check(reply.result.state === 'REGISTERED' && reply.result.members?.some(member =>
    member.worktreeId === row.worktreeId && member.domainId === config.domainId &&
    member.repositoryId === config.repositoryId && member.seatId === row.seatId &&
    member.instanceId === row.instanceId), `${row.driverId}: original registered F/E binding`);
  return reply;
}
async function sessionStep(session, action, payload = {}, allowed = ['APPLIED']) {
  const reply = await operation('K-SESSION', action, session.id,
    { generation: session.generation, ...payload }, session.revision, allowed);
  session.revision = reply.revision;
  product.save();
  return reply;
}
async function output(session) {
  let reply = await operation('K-SESSION', 'output-stream', session.id,
    { generation: session.generation, afterCursor: session.cursor }, session.revision, ['APPLIED', 'STALE']);
  if (reply.status === 'STALE') {
    session.revision = reply.revision; product.save();
    reply = await operation('K-SESSION', 'output-stream', session.id,
      { generation: session.generation, afterCursor: session.cursor }, session.revision);
  }
  check(BigInt(reply.result.cursor) >= BigInt(session.cursor), `${session.id}: original H cursor monotonic`);
  session.revision = reply.revision; session.cursor = reply.result.cursor;
  session.events.push(...reply.result.events); product.save();
  if (reply.result.sourceError) throw Error(`Original ${session.id} A source error: ${JSON.stringify(reply.result.sourceError)}`);
  return reply.result;
}
async function captureArgv(row) {
  const script = String.raw`$ErrorActionPreference = 'Stop'
$inputJson = [Console]::In.ReadToEnd() | ConvertFrom-Json
$processes = @(Get-CimInstance Win32_Process)
$byId = @{}
foreach ($item in $processes) { $byId[[string]$item.ProcessId] = $item }
function Test-ChildOfProduct($item, $rootPid, $map) {
  $parent = [string]$item.ParentProcessId
  for ($depth = 0; $depth -lt 32 -and $parent; $depth++) {
    if ($parent -eq [string]$rootPid) { return $true }
    if (-not $map.ContainsKey($parent)) { return $false }
    $parent = [string]$map[$parent].ParentProcessId
  }
  return $false
}
function ArgValue($line, $name) {
  $pattern = '(?:^|\s)' + [regex]::Escape($name) + '(?:=|\s+)(?:"([^"]*)"|''([^'']*)''|([^\s"'']+))'
  $match = [regex]::Match($line, $pattern)
  if (-not $match.Success) { return $null }
  foreach ($index in 1..3) { if ($match.Groups[$index].Success) { return $match.Groups[$index].Value } }
  return $null
}
$matchingProcesses = @()
foreach ($item in $processes) {
  if (-not $item.ExecutablePath -or -not (Test-ChildOfProduct $item $inputJson.productPid $byId)) { continue }
  $hash = (Get-FileHash -LiteralPath $item.ExecutablePath -Algorithm SHA256).Hash.ToLowerInvariant()
  if ($hash -ne $inputJson.sha256) { continue }
  $line = [string]$item.CommandLine
  $effortFlag = if ($inputJson.driverId -eq 'grok') { '--reasoning-effort' } else { '--effort' }
  $matchingProcesses += [pscustomobject]@{
    processId = [string]$item.ProcessId
    parentProcessId = [string]$item.ParentProcessId
    imageSha256 = $hash
    model = ArgValue $line '--model'
    effort = ArgValue $line $effortFlag
    modelArgCount = [regex]::Matches($line, '(?:^|\s)--model(?:=|\s+)').Count
    effortArgCount = [regex]::Matches($line, '(?:^|\s)' + [regex]::Escape($effortFlag) + '(?:=|\s+)').Count
    modelFlag = '--model'
    effortFlag = $effortFlag
    commandLineSha256 = -join ([Security.Cryptography.SHA256]::Create().ComputeHash([Text.Encoding]::Unicode.GetBytes($line)) | ForEach-Object { $_.ToString('x2') })
  }
}
ConvertTo-Json -InputObject @($matchingProcesses) -Compress`;
  const payload = JSON.stringify({ productPid: product.endpoint.pid, driverId: row.driverId,
    sha256: row.sha256, model: row.model, effort: row.effort });
  const captured = await new Promise((resolve, reject) => {
    const child = spawn(config.pwsh, ['-NoProfile', '-NonInteractive', '-Command', script],
      { windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] });
    let stdout = '', stderr = '';
    child.stdout.on('data', bytes => { stdout = (stdout + bytes).slice(-8192); });
    child.stderr.on('data', bytes => { stderr = (stderr + bytes).slice(-8192); });
    child.once('error', reject);
    child.once('exit', code => code === 0 ? resolve(stdout.trim()) : reject(Error(`Readonly process argv observer exit=${code}: ${stderr}`)));
    child.stdin.end(payload);
  });
  const rows = JSON.parse(captured);
  const matches = Array.isArray(rows) ? rows : [rows];
  check(matches.length === 1 && matches[0].imageSha256 === row.sha256 &&
    matches[0].model === row.model && matches[0].effort === row.effort &&
    matches[0].modelArgCount === 1 && matches[0].effortArgCount === 1 &&
    typeof matches[0].commandLineSha256 === 'string' && /^[a-f0-9]{64}$/.test(matches[0].commandLineSha256),
    `${row.driverId}: actual pinned child argv binds configured model and effort`);
  return { basis: 'ACTUAL_PRODUCT_DESCENDANT_PROCESS_COMMAND_LINE_FILTERED',
    productRootPid: String(product.endpoint.pid), ...matches[0] };
}
async function observeReceipt(session, row) {
  const deadline = Date.now() + 600000;
  while (Date.now() < deadline) {
    const page = await output(session);
    const receipt = page.nativeInputReceipts?.find(value => value.requestId === row.sendRequestId);
    if (receipt?.receipt?.status === 'APPLIED') {
      row.hReceipt = receipt; product.save(); return;
    }
    if (receipt?.receipt?.status && receipt.receipt.status !== 'UNKNOWN') {
      throw Error(`${row.driverId}: original H receipt=${receipt.receipt.status}; no replay`);
    }
    await delay(300); // Observe only this request. Never issue a second send.
  }
  throw Error(`${row.driverId}: original H receipt observation expired; preserve product and request`);
}

async function runCase(row, instances) {
  const record = { driverId: row.driverId, instanceId: row.instanceId, seatId: row.seatId,
    worktreeId: row.worktreeId, fixedVersion: row.version, fixedSha256: row.sha256,
    expectedModel: row.model, expectedEffort: row.effort, state: 'PREFLIGHT', acceptance: false };
  journal.cases.push(record); product.save();
  const instance = instances.instances.find(value => value.instanceId === row.instanceId && value.driverId === row.driverId);
  if (!instance || instance.state !== 'LOGGED_IN') {
    record.result = 'NOT_RUN_NOT_LOGGED_IN'; product.save();
    throw Error(`${row.driverId}: configured instance not logged in; no login or fallback attempted`);
  }
  record.loggedInInstance = { instanceId: instance.instanceId, driverId: instance.driverId,
    version: instance.version, state: instance.state };
  check(instance.version === row.version, `${row.driverId}: actual logged-in instance version pin`);
  const seat = await seatCard(row.seatId);
  check(seat.result.state === 'IDLE' && seat.result.instanceId === row.instanceId,
    `${row.driverId}: actual seat is Idle on configured instance`);
  check(seat.result.settings?.model === row.model && seat.result.settings?.effort === row.effort,
    `${row.driverId}: configured seat model and effort match private fixture`);
  if (row.driverId === 'opencode') check(/(grok|xai)/i.test(seat.result.settings.model) &&
    !/gpt/i.test(seat.result.settings.model), 'OpenCode seat card confirms xAI/Grok and not GPT');
  record.seatSettings = { model: seat.result.settings.model, effort: seat.result.settings.effort };
  product.save();
  record.graph = await graph(row); product.save();
  const session = { id: id('m2ProviderSession'), caseId: journal.caseId,
    seatId: row.seatId, instanceId: row.instanceId, worktreeId: row.worktreeId,
    generation: (BigInt(seat.result.generation) + 1n).toString(), revision: '0', cursor: '0', events: [], turns: [] };
  record.sessionId = session.id; journal.sessions.push(session); product.save();
  for (const action of ['admission-reserve', 'admission-commit']) await sessionStep(session, action, { seatId: row.seatId });
  const opened = await sessionStep(session, 'open', { seatId: row.seatId,
    repositoryId: config.repositoryId, worktreeId: row.worktreeId });
  session.threadId = opened.result.threadId; product.save();
  const capability = await sessionStep(session, 'capability-probe');
  check(capability.result.driverId === row.driverId && capability.result.version === row.version &&
    capability.result.binaryDigest === `sha256:${row.sha256}` && capability.result.evidenceBasis !== undefined,
    `${row.driverId}: original H/F fixed executable pin`);
  check(capability.result.evidenceBasis === (row.driverId === 'claude'
    ? 'ORIGINAL_CLAUDE_INITIALIZE_ACK' : 'NATIVE_ACP_INITIALIZE_DECLARATION'),
    `${row.driverId}: original provider initialization evidence basis`);
  record.capability = { driverId: capability.result.driverId, version: capability.result.version,
    binaryDigest: capability.result.binaryDigest, evidenceBasis: capability.result.evidenceBasis };
  if (row.driverId === 'claude') {
    check(capability.result.requestedModel === row.model && capability.result.requestedEffort === row.effort,
      'Claude H capability records model and effort from the bound seat');
    record.modelEffortEvidence = { basis: 'ORIGINAL_H_CLAUDE_CAPABILITY',
      model: capability.result.requestedModel, effort: capability.result.requestedEffort };
  } else if (row.driverId === 'opencode') {
    record.modelEffortEvidence = { basis: 'REQUIRES_ORIGINAL_ACP_MODEL_EFFORT_ACK_READBACK',
      model: seat.result.settings.model, effort: seat.result.settings.effort };
  } else {
    record.modelEffortEvidence = { basis: 'REQUIRES_CAPTURED_ACTUAL_PROCESS_ARGV',
      model: seat.result.settings.model, effort: seat.result.settings.effort };
  }
  if (row.driverId === 'claude' || row.driverId === 'grok') {
    record.modelEffortEvidence.argv = await captureArgv(row);
  }
  product.save();
  const marker = `${journal.marker}_${row.driverId}_${id('answer')}`;
  record.marker = marker;
  record.prompt = `Private non-secret provider protocol capture. Reply with exactly ${marker}. ` +
    'Do not use tools, read or write files, open a browser, access credentials, or switch models.';
  const sent = await sessionStep(session, 'send', { body: record.prompt }, ['APPLIED', 'UNKNOWN']);
  record.sendRequestId = journal.operations.at(-1).request.requestId;
  record.sendStatus = sent.status; record.state = 'SENT_ORIGINAL_ONCE'; product.save();
  if (sent.status === 'UNKNOWN' && sent.result.reason) {
    throw Error(`${row.driverId}: original send UNKNOWN reason=${sent.result.reason}; no resend`);
  }
  await observeReceipt(session, record);
  record.state = 'ORIGINAL_H_RECEIPTED'; product.save();
  const stopped = await sessionStep(session, 'stop', { seatId: row.seatId });
  check(typeof stopped.result.stopFact === 'string' && stopped.result.stopFact.length > 0,
    `${row.driverId}: original process normal stop fact`);
  record.stopFact = stopped.result.stopFact;
  await sessionStep(session, 'admission-release', { seatId: row.seatId });
  record.result = 'DIRECT_H_RECEIPT_A_READBACK_REQUIRED'; product.save();
}

async function readbackAndGolden() {
  const outputFile = path.join(config.evidenceDirectory, 'm2-provider-readback-final.json');
  if (fs.existsSync(outputFile)) throw Error('Provider private readback already exists');
  await new Promise((resolve, reject) => {
    const child = spawn(config.python, [path.join(here, 'm2-provider-capture-readback.py'),
      config.stateRoot, outputFile, config.result],
      { windowsHide: true, stdio: ['ignore', 'ignore', 'pipe'] });
    let stderr = ''; child.stderr.on('data', bytes => { stderr = (stderr + bytes).slice(-8192); });
    child.once('error', reject);
    child.once('exit', code => code === 0 ? resolve() : reject(Error(`Original provider immutable readback exit=${code}: ${stderr}`)));
  });
  const facts = readJson(outputFile);
  check(facts.measurementPreservedDatabaseBytes && facts.directProviderEvidence &&
    facts.acceptance === false && facts.databaseWrites === false && facts.credentialReads === false,
    'Provider normal-close immutable direct readback');
  const reference = { file: path.basename(outputFile), sha256: sha256(outputFile) };
  journal.readbacks.push(reference); product.save();
  for (const record of journal.cases) {
    if (record.result === 'NOT_RUN_NOT_LOGGED_IN') continue;
    const observed = facts.sessions.find(value => value.sessionId === record.sessionId);
    check(observed?.driverId === record.driverId && observed?.version === record.fixedVersion &&
      observed?.binarySha256 === `sha256:${record.fixedSha256}` && observed?.allEpisodesStopped &&
      observed?.providerEndTurn === true && observed?.markerObserved === true,
      `${record.driverId}: original A end-turn, marker, and stopped pinned H session`);
    const bundle = path.join(config.evidenceDirectory, `m2-${record.driverId}-protocol-golden.json`);
    if (fs.existsSync(bundle)) throw Error(`${record.driverId}: private golden output already exists`);
    await new Promise((resolve, reject) => {
      const child = spawn(process.execPath, [path.join(here, 'cli-protocol-golden.mjs'), 'import',
        '--frames', outputFile, '--normalized', outputFile, '--out', bundle,
        '--session-id', record.sessionId, '--cli-version', record.fixedVersion,
        '--binary-sha256', record.fixedSha256, '--capture-id', `${journal.caseId}-${record.driverId}`,
        '--outcome', 'success'], { windowsHide: true, stdio: ['ignore', 'ignore', 'pipe'] });
      let stderr = ''; child.stderr.on('data', bytes => { stderr = (stderr + bytes).slice(-8192); });
      child.once('error', reject);
      child.once('exit', code => code === 0 ? resolve() : reject(Error(`${record.driverId} golden importer exit=${code}: ${stderr}`)));
    });
    const golden = readJson(bundle);
    check(golden.manifest.cliDriver === record.driverId && golden.manifest.cliVersion === record.fixedVersion &&
      golden.manifest.cliBinarySha256 === record.fixedSha256 && golden.manifest.productSourceCommit === config.sourceCommit &&
      golden.manifest.inputSha256.privateFrames === reference.sha256 &&
      golden.manifest.inputSha256.normalizedOutput === reference.sha256 &&
      golden.manifest.baselineStatus === 'REVIEW_REQUIRED' && golden.manifest.acceptance === 'NOT_ASSESSED',
      `${record.driverId}: existing importer preserves fixed identity and review-required state`);
    journal.goldens.push({ driverId: record.driverId, file: path.basename(bundle), sha256: sha256(bundle),
      state: 'REVIEW_REQUIRED_ACCEPTANCE_NOT_ASSESSED' }); product.save();
  }
}

try {
  journal.driverBytes = { ...journal.driverBytes,
    ...Object.fromEntries(['cli-protocol-golden.mjs', 'm2-provider-win11.mjs',
      'm2-provider-capture-readback.py'].map(name => [name, sha256(path.join(here, name))])) };
  await snapshot('before');
  await product.launch();
  check(Boolean(product.tester) && journal.connectionBackend?.name === 'tester-army/e2e' &&
    journal.connectionBackend.telemetryDisabled === true && journal.connectionBackend.agentActs === 0,
    'Actual installed product uses hard-located tester-army/e2e with telemetry disabled and agent.act zero');
  const instances = await product.instances();
  for (const row of config.cases) await runCase(row, instances);
  journal.state = 'DIRECT_PROVIDER_H_RECEIPTS_A_READBACK_REQUIRED'; product.save();
  await product.closeNormally();
  await readbackAndGolden();
  await snapshot('after');
  for (const observer of config.observers) {
    const before = observerValue(observer.name, 'before'), after = observerValue(observer.name, 'after');
    for (const field of observer.equalFields ?? []) check(Object.hasOwn(before, field) && Object.hasOwn(after, field) &&
      JSON.stringify(before[field]) === JSON.stringify(after[field]), `${observer.name}: unchanged ${field}`);
  }
  for (const phase of ['before', 'after']) {
    const memory = observerValue('memory', phase);
    check(memory.memoryDataUnchangedByRead === true && memory.stage1OutputCount === 0 &&
      memory.memoryJobCount === 0, `memory ${phase}: existing read-only observer facts`);
  }
  journal.state = 'REVIEW_REQUIRED'; journal.acceptance = false; product.save();
} catch (error) {
  journal.state = 'FAIL'; journal.error = String(error.stack ?? error);
  journal.currentEndpoint = product.endpoint ?? null; product.save(); process.exitCode = 1;
  try { await product.preserveFailure(); }
  catch (disconnectError) { journal.disconnectError = String(disconnectError); product.save(); }
}
