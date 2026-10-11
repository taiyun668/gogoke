// Standalone real installed-provider capture. This is a development driver,
// not a host/model substitute or an M2 acceptance decision.
import fs from 'node:fs';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { ActualProduct, readJson, sha256, id, delay } from './product-cdp.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const drivers = ['claude', 'opencode', 'grok'];
const atom = value => typeof value === 'string' && /^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/.test(value);
const inside = (child, parent) => child === parent || child.startsWith(parent + path.sep);
const sameWindowsPath = (left, right) => path.resolve(left).toUpperCase() === path.resolve(right).toUpperCase();
const config = readJson(process.argv[2]);
const required = ['installed', 'installedSha256', 'version', 'sourceCommit', 'registryKey', 'pwsh',
  'python', 'stateRoot', 'evidenceDirectory', 'result', 'domainId', 'repositoryId', 'observers', 'cases'];
if (process.platform !== 'win32' || !process.argv[2] || required.some(key => config[key] === undefined) ||
    config.repositoryId !== 'gogokeSeatTestbed' || !atom(config.domainId) ||
    !/^[a-f0-9]{40}$/.test(config.sourceCommit) || typeof config.version !== 'string' || !config.version ||
    !Array.isArray(config.cases) || config.cases.length < 1 || config.cases.length > 3 ||
    new Set(config.cases.map(row => row.driverId)).size !== config.cases.length ||
    new Set(config.cases.map(row => row.seatId)).size !== config.cases.length ||
    new Set(config.cases.map(row => row.worktreeId)).size !== config.cases.length ||
    config.cases.some(row => !drivers.includes(row.driverId)) ||
    new Set(config.cases.map(row => row.instanceId)).size !== config.cases.length ||
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
const writeCases = config.cases.filter(row => row.caseMode === 'grokWrite');
const sequenceCases = config.cases.filter(row => row.caseMode === 'sequence');
if (config.cases.some(row => row.caseMode !== undefined &&
      !['grokWrite', 'sequence'].includes(row.caseMode)) ||
    (sequenceCases.length > 0 && (sequenceCases.length !== 1 || config.cases.length !== 1 ||
      sequenceCases[0].driverId !== 'grok')) ||
    (writeCases.length > 0 && (writeCases.length !== 1 || config.cases.length !== 1 ||
      writeCases[0].driverId !== 'grok' ||
      typeof writeCases[0].worktreePath !== 'string' || !path.isAbsolute(writeCases[0].worktreePath) ||
      writeCases[0].worktreePath.startsWith('\\\\?\\') ||
      !/^volume:[a-f0-9]{16}\/file:[a-f0-9]{32}$/.test(writeCases[0].worktreeIdentity ?? '') ||
      !/^grok-provider-write_[a-f0-9]{32}\.json$/.test(writeCases[0].writeMarker ?? '') ||
      !sameWindowsPath(path.dirname(path.resolve(writeCases[0].worktreePath)),
        path.resolve(config.stateRoot, 'v37-worktrees', 'single'))))) {
  throw Error('Grok Write needs one exact registered F root identity and one new private marker');
}
const opencode = config.cases.find(row => row.driverId === 'opencode');
if (opencode && (opencode.model.toLowerCase().includes('gpt') || !/(grok|xai)/i.test(opencode.model))) {
  throw Error('OpenCode case must use its configured xAI/Grok model; GPT is not an eligible substitute');
}

const journal = { schema: 'gogoke.37.m2-provider-win11-e2e.v1', caseId: id('m2ProviderE2E'),
  sourceCommit: config.sourceCommit, domainId: config.domainId, repositoryId: config.repositoryId,
  installedVersion: config.version, installedSha256: config.installedSha256,
  state: 'RUNNING', acceptance: false, authenticationActions: false, observerDatabaseWrites: false,
  hostOperationsWriteCandidateDatabase: true, credentialReads: false, b5: 'NOT_RUN_UNSUPPORTED',
  marker: id('M2_PROVIDER_MARKER'), cases: [], operations: [], sessions: [], launches: [], closes: [],
  snapshots: {}, readbacks: [], goldens: [], assertions: [],
  notRun: drivers.filter(driverId => !config.cases.some(row => row.driverId === driverId))
    .map(driverId => ({ driverId, result: 'NOT_RUN_NOT_SELECTED_FOR_THIS_ORIGINAL_CASE' })) };
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
async function captureProcess(row) {
  const script = String.raw`$ErrorActionPreference = 'Stop'
$inputJson = [Console]::In.ReadToEnd() | ConvertFrom-Json
$processes = @(Get-CimInstance Win32_Process -Property ProcessId,ParentProcessId,ExecutablePath)
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
  $processDetails = Get-CimInstance Win32_Process -Filter ('ProcessId = ' + [string]$item.ProcessId) -Property ProcessId,CommandLine
  $line = [string]$processDetails.CommandLine
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
  const requiresModelArgv = row.driverId !== 'opencode';
  check(matches.length === 1 && matches[0].imageSha256 === row.sha256 &&
    (!requiresModelArgv || (matches[0].model === row.model && matches[0].effort === row.effort &&
    matches[0].modelArgCount === 1 && matches[0].effortArgCount === 1)) &&
    typeof matches[0].commandLineSha256 === 'string' && /^[a-f0-9]{64}$/.test(matches[0].commandLineSha256),
    `${row.driverId}: actual product descendant is the fixed CLI process`);
  const identityText = await new Promise((resolve, reject) => {
    const child = spawn(config.python, [path.join(here, 'm2-provider-capture-readback.py'),
      '--process-identity', String(matches[0].processId), row.sha256],
      { windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
    let stdout = '', stderr = '';
    child.stdout.on('data', bytes => { stdout = (stdout + bytes).slice(-4096); });
    child.stderr.on('data', bytes => { stderr = (stderr + bytes).slice(-8192); });
    child.once('error', reject);
    child.once('exit', code => code === 0 ? resolve(stdout.trim()) :
      reject(Error(`Exact provider process identity exit=${code}: ${stderr}`)));
  });
  const identity = JSON.parse(identityText);
  check(identity.processId === String(matches[0].processId) && identity.imageSha256 === row.sha256 &&
    /^[0-9]+$/.test(identity.creationTime100ns),
    `${row.driverId}: exact process creation time and pinned image`);
  return { basis: 'ACTUAL_PRODUCT_DESCENDANT_PROCESS_COMMAND_LINE_FILTERED',
    productRootPid: String(product.endpoint.pid), ...matches[0],
    creationTime100ns: identity.creationTime100ns };
}
async function captureDirectoryIdentity(root) {
  const identity = await new Promise((resolve, reject) => {
    const child = spawn(config.python, [path.join(here, 'm2-provider-capture-readback.py'),
      '--directory-identity', root], { windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
    let stdout = '', stderr = '';
    child.stdout.on('data', bytes => { stdout = (stdout + bytes).slice(-4096); });
    child.stderr.on('data', bytes => { stderr = (stderr + bytes).slice(-4096); });
    child.once('error', reject);
    child.once('exit', code => code === 0 ? resolve(stdout.trim()) :
      reject(Error(`Original F directory identity exit=${code}: ${stderr}`)));
  });
  return identity;
}
async function observeReceipt(session, row, turn = row) {
  const deadline = Date.now() + 600000;
  while (Date.now() < deadline) {
    const page = await output(session);
    const receipt = page.nativeInputReceipts?.find(value => value.requestId === turn.sendRequestId);
    if (receipt?.receipt?.status === 'APPLIED') {
      turn.hReceipt = receipt; product.save(); return receipt;
    }
    if (receipt?.receipt?.status && receipt.receipt.status !== 'UNKNOWN') {
      throw Error(`${row.driverId}: original H receipt=${receipt.receipt.status}; no replay`);
    }
    await delay(300); // Observe only this request. Never issue a second send.
  }
  throw Error(`${row.driverId}: original H receipt observation expired; preserve product and request`);
}

async function runSequenceTurns(row, record, session) {
  const sequence = { state: 'RUNNING', turns: [],
    steer: { state: 'NOT_RUN_UNSUPPORTED',
      originalCapability: record.sequenceCapability.inTurnSteer,
      reason: 'The original H capability reports no ACP in-turn steer; no new send is substituted' },
    runningInterrupt: { state: 'NOT_RUN_RUNNING_STOP_NOT_DISPATCHED',
      originalCapability: record.sequenceCapability.interruptAndResume,
      reason: 'Both original sends reached their own terminal receipt before H stop; this case does not claim an in-flight interruption' },
    coldResume: null, firstStopFact: null, finalStopFact: null };
  record.sequence = sequence; product.save();
  for (let ordinal = 1; ordinal <= 2; ordinal++) {
    const marker = `${journal.marker}_${row.driverId}_${id(`sequence${ordinal}`)}`;
    const prompt = `Private non-secret provider sequence turn ${ordinal}. Reply with exactly ${marker}. ` +
      'Do not use tools, read or write files, open a browser, access credentials, or switch models.';
    let before;
    for (;;) {
      const prior = session.cursor;
      before = await output(session);
      check(/^\d+$/.test(before.rawHighwater ?? '') &&
        /^\d+$/.test(before.ledgerHighwater ?? '') &&
        !before.nativeCardRefs?.some(card => card.state === 'OPEN'),
      'Provider sequence needs a complete original H page without an open card');
      if (BigInt(before.cursor) === BigInt(before.ledgerHighwater)) break;
      check(BigInt(before.cursor) > BigInt(prior),
        'Provider sequence pre-send H page made no ledger progress');
    }
    const turn = { ordinal, marker, prompt, generation: session.generation,
      beforeSendRawHighwater: before.rawHighwater, sendRequestId: null,
      hReceipt: null };
    sequence.turns.push(turn); product.save();
    const sent = await sessionStep(session, 'send', { body: prompt }, ['APPLIED', 'UNKNOWN']);
    turn.sendRequestId = journal.operations.at(-1).request.requestId;
    turn.sendStatus = sent.status; product.save();
    if (sent.status === 'UNKNOWN' && sent.result.reason) {
      throw Error(`${row.driverId}: original sequence send UNKNOWN reason=${sent.result.reason}; no resend`);
    }
    const receipt = await observeReceipt(session, row, turn);
    check(receipt.receipt?.result?.createdTurn === true &&
      receipt.receipt.result.stopReason === 'end_turn' &&
      typeof receipt.receipt.result.sourceEpoch === 'string' &&
      /^\d+$/.test(receipt.receipt.result.sourceCursor ?? '') &&
      BigInt(receipt.receipt.result.sourceCursor) > BigInt(turn.beforeSendRawHighwater),
    `${row.driverId}: original sequence H terminal receipt is incomplete`);
    const deadline = Date.now() + 600000;
    let observed = false;
    while (Date.now() < deadline) {
      const page = await output(session);
      const text = session.events.filter(event => event.sessionUpdate === 'agent_message_chunk' &&
        event._meta?.threadId === session.threadId &&
        /^\d+$/.test(event._meta?.rawSourceCursor ?? '') &&
        BigInt(event._meta.rawSourceCursor) > BigInt(turn.beforeSendRawHighwater) &&
        BigInt(event._meta.rawSourceCursor) <= BigInt(receipt.receipt.result.sourceCursor))
        .map(event => event.content?.type === 'text' ? event.content.text : '').join('');
      if (text.includes(marker) && /^\d+$/.test(page.rawHighwater ?? '') &&
          BigInt(page.rawHighwater) >= BigInt(receipt.receipt.result.sourceCursor) &&
          page.cursor === page.ledgerHighwater) {
        observed = true; break;
      }
      await delay(300);
    }
    check(observed, `${row.driverId}: original sequence marker and terminal A output absent`);
    turn.markerObserved = true; turn.terminalSourceEpoch = receipt.receipt.result.sourceEpoch;
    turn.terminalSourceCursor = receipt.receipt.result.sourceCursor; product.save();
  }
  const firstStop = await sessionStep(session, 'stop', { seatId: row.seatId });
  check(/^sha256:[a-f0-9]{64}$/.test(firstStop.result.stopFact ?? ''),
    `${row.driverId}: first original H StopFact missing`);
  sequence.firstStopFact = firstStop.result.stopFact; product.save();
  await product.closeNormally();
  await product.launch();
  check(Boolean(product.tester) && journal.connectionBackend?.name === 'tester-army/e2e',
    'Cold provider sequence must use the same installed product USER endpoint');
  const oldGeneration = session.generation;
  let resumed = await operation('K-SESSION', 'resume', session.id,
    { generation: oldGeneration }, session.revision, ['APPLIED', 'UNKNOWN']);
  let original = journal.operations.at(-1);
  const resumeDeadline = Date.now() + 600000;
  while (resumed.status === 'UNKNOWN') {
    if (resumed.result.reason || Date.now() >= resumeDeadline) {
      throw Error(`${row.driverId}: exact original H resume UNKNOWN; preserve without a fresh session`);
    }
    await delay(300);
    original = await product.reconcile(original); resumed = original.receipt;
  }
  check(['APPLIED', 'REPLAYED'].includes(resumed.status) && resumed.result.state === 'RUNNING' &&
    resumed.result.oldGeneration === oldGeneration &&
    /^\d+$/.test(resumed.result.newGeneration ?? '') &&
    BigInt(resumed.result.newGeneration) > BigInt(oldGeneration) &&
    (!resumed.result.threadId || resumed.result.threadId === session.threadId),
  `${row.driverId}: original stopped session did not resume its own H/thread`);
  session.generation = resumed.result.newGeneration;
  session.revision = resumed.revision; sequence.coldResume = {
    requestId: resumed.requestId, oldGeneration, newGeneration: session.generation,
    sessionId: session.id, threadId: session.threadId, receipt: resumed };
  record.resumedProcessIdentity = await captureProcess(row); product.save();
  const resumedCapability = await sessionStep(session, 'capability-probe');
  check(resumedCapability.result.driverId === row.driverId &&
    resumedCapability.result.version === row.version &&
    resumedCapability.result.binaryDigest === `sha256:${row.sha256}` &&
    resumedCapability.result.capabilities?.inTurnSteer === 'UNSUPPORTED' &&
    resumedCapability.result.capabilities?.resume ===
      'SOURCE_PRESENT_NATIVE_RESUME_BEHAVIOUR_NOT_RUN',
  `${row.driverId}: resumed exact fixed CLI capability differs`);
  sequence.resumedCapability = { requestId: resumedCapability.requestId,
    processOperationId: resumedCapability.result.processOperationId,
    inTurnSteer: resumedCapability.result.capabilities.inTurnSteer,
    resume: resumedCapability.result.capabilities.resume };
  const finalStop = await sessionStep(session, 'stop', { seatId: row.seatId });
  check(/^sha256:[a-f0-9]{64}$/.test(finalStop.result.stopFact ?? '') &&
    finalStop.result.stopFact !== sequence.firstStopFact,
  `${row.driverId}: final original H StopFact missing or reused`);
  sequence.finalStopFact = finalStop.result.stopFact;
  await sessionStep(session, 'admission-release', { seatId: row.seatId });
  sequence.state = 'ORIGINAL_TWO_TURNS_COLD_RESUME_STOP_RELEASE_READBACK_REQUIRED';
  record.result = 'DIRECT_H_RECEIPT_A_READBACK_REQUIRED'; product.save();
}

async function runCase(row, instances) {
  const grokWrite = row.caseMode === 'grokWrite';
  const record = { driverId: row.driverId, instanceId: row.instanceId, seatId: row.seatId,
    worktreeId: row.worktreeId, fixedVersion: row.version, fixedSha256: row.sha256,
    expectedModel: row.model, expectedEffort: row.effort, caseMode: row.caseMode ?? 'singleAnswer',
    state: 'PREFLIGHT', acceptance: false };
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
  if (grokWrite) check(seat.result.layer === 'USER' &&
    seat.result.settings?.permissionTier === 'NETWORKED_WRITE',
  'Grok Write needs the actual IDLE USER NETWORKED_WRITE E seat');
  if (row.driverId === 'opencode') check(/(grok|xai)/i.test(seat.result.settings.model) &&
    !/gpt/i.test(seat.result.settings.model), 'OpenCode seat card confirms xAI/Grok and not GPT');
  record.seatSettings = { model: seat.result.settings.model, effort: seat.result.settings.effort };
  product.save();
  record.graph = await graph(row); product.save();
  if (grokWrite) {
    const root = path.resolve(row.worktreePath);
    const stat = fs.lstatSync(root);
    check(stat.isDirectory() && !stat.isSymbolicLink() &&
      sameWindowsPath(fs.realpathSync.native(root), root) &&
      await captureDirectoryIdentity(root) === row.worktreeIdentity,
    'Grok Write F physical root differs from the private registered identity');
    const target = path.join(root, row.writeMarker);
    check(path.dirname(target) === root && !fs.existsSync(target),
      'Grok Write target must be a new file directly in this F root');
    const bytes = JSON.stringify({ case: 'GROK_PROVIDER_WRITE', marker: row.writeMarker }) + '\n';
    record.write = { worktreePath: root, worktreeIdentity: row.worktreeIdentity,
      marker: row.writeMarker, target, expectedContentLength: Buffer.byteLength(bytes),
      expectedContentSha256: createHash('sha256').update(bytes).digest('hex'),
      targetAbsentBeforeSend: true, actualOsAclDisposition: 'NOT_RUN',
      permissionCause: 'UNATTRIBUTED' };
    record.write.expectedContent = bytes;
    product.save();
  }
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
  if (row.caseMode === 'sequence') {
    const capabilities = capability.result.capabilities;
    check(capabilities?.inTurnSteer === 'UNSUPPORTED' &&
      capabilities?.resume === 'SOURCE_PRESENT_NATIVE_RESUME_BEHAVIOUR_NOT_RUN' &&
      capabilities?.interruptAndResume === 'SOURCE_PRESENT_NATIVE_RESUME_BEHAVIOUR_NOT_RUN',
    `${row.driverId}: original H ACP sequence capability facts differ`);
    record.sequenceCapability = { inTurnSteer: capabilities.inTurnSteer,
      resume: capabilities.resume, interruptAndResume: capabilities.interruptAndResume,
      evidenceBasis: capability.result.evidenceBasis,
      requestId: capability.requestId };
  }
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
  record.processIdentity = await captureProcess(row);
  if (row.driverId === 'claude' || row.driverId === 'grok') record.modelEffortEvidence.argv = record.processIdentity;
  product.save();
  if (row.caseMode === 'sequence') {
    await runSequenceTurns(row, record, session);
    return;
  }
  if (grokWrite) {
    record.marker = row.writeMarker;
    record.prompt = `Create one nonsecret JSON file at ${JSON.stringify(record.write.target)} ` +
      `with these exact UTF-8 bytes: ${JSON.stringify(record.write.expectedContent)}. ` +
      'Use the Write tool once for this file. Do not use shell, Git, network tools, ' +
      'browser, agents, or credentials. Do not retry or choose another path. ' +
      'Report the result of this one tool call.';
    let before;
    for (;;) {
      const prior = session.cursor;
      before = await output(session);
      check(/^\d+$/.test(before.rawHighwater ?? '') &&
        /^\d+$/.test(before.ledgerHighwater ?? '') &&
        !before.nativeCardRefs?.some(card => card.state === 'OPEN'),
      'Grok Write needs an original H page without a pending card');
      if (BigInt(before.cursor) === BigInt(before.ledgerHighwater)) break;
      check(BigInt(before.cursor) > BigInt(prior),
        'Grok Write pre-send H page made no ledger progress');
    }
    record.beforeSendRawHighwater = before.rawHighwater;
  } else if (row.driverId === 'claude') {
    record.prompt = 'What is 241 + 537?';
    record.expectedAnswer = '778';
  } else {
    const marker = `${journal.marker}_${row.driverId}_${id('answer')}`;
    record.marker = marker;
    record.prompt = `Private non-secret provider protocol capture. Reply with exactly ${marker}. ` +
      'Do not use tools, read or write files, open a browser, access credentials, or switch models.';
  }
  const sent = await sessionStep(session, 'send', { body: record.prompt }, ['APPLIED', 'UNKNOWN']);
  record.sendRequestId = journal.operations.at(-1).request.requestId;
  record.sendStatus = sent.status; record.state = 'SENT_ORIGINAL_ONCE'; product.save();
  if (sent.status === 'UNKNOWN' && sent.result.reason) {
    throw Error(`${row.driverId}: original send UNKNOWN reason=${sent.result.reason}; no resend`);
  }
  const receipt = await observeReceipt(session, record);
  if (grokWrite) {
    check(receipt.receipt?.result?.createdTurn === true &&
      receipt.receipt.result.stopReason === 'end_turn' &&
      /^\d+$/.test(receipt.receipt.result.sourceCursor ?? '') &&
      typeof receipt.receipt.result.sourceEpoch === 'string' &&
      BigInt(receipt.receipt.result.sourceCursor) >= BigInt(record.beforeSendRawHighwater),
    'Grok Write original ACP terminal receipt is incomplete');
    const deadline = Date.now() + 600000;
    let complete = false;
    while (Date.now() < deadline) {
      const page = await output(session);
      check(/^\d+$/.test(page.rawHighwater ?? '') &&
        /^\d+$/.test(page.ledgerHighwater ?? ''),
      'Grok Write output cursor shape differs');
      if (BigInt(page.rawHighwater) >= BigInt(receipt.receipt.result.sourceCursor) &&
          BigInt(page.cursor) === BigInt(page.ledgerHighwater)) {
        record.terminalOutputPage = { rawHighwater: page.rawHighwater,
          ledgerCursor: page.cursor, ledgerHighwater: page.ledgerHighwater };
        complete = true; break;
      }
      await delay(300);
    }
    check(complete, 'Grok Write original ACP terminal source was not fully observed');
    record.toolCandidates = session.events.filter(event =>
      ['tool_call', 'tool_call_update'].includes(event.sessionUpdate) &&
      event._meta?.provider === 'grok-build' &&
      event._meta?.threadId === session.threadId &&
      /^\d+$/.test(event._meta?.rawSourceCursor ?? '') &&
      BigInt(event._meta.rawSourceCursor) > BigInt(record.beforeSendRawHighwater) &&
      BigInt(event._meta.rawSourceCursor) <= BigInt(receipt.receipt.result.sourceCursor))
      .map(event => ({ sourceCursor: event._meta.rawSourceCursor,
        toolCallId: event.toolCallId, update: event.sessionUpdate, title: event.title,
        kind: event.kind, status: event.status, rawInput: event.rawInput,
        rawOutput: event.rawOutput, content: event.content }));
    record.write.targetExistsAfterTurn = fs.existsSync(record.write.target);
    if (record.write.targetExistsAfterTurn) {
      const bytes = fs.readFileSync(record.write.target);
      record.write.targetContentLengthAfterTurn = bytes.length;
      record.write.targetContentSha256AfterTurn = createHash('sha256').update(bytes).digest('hex');
    }
    product.save();
  }
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
      observed?.providerEndTurn === true && (record.driverId === 'claude'
        ? observed?.answerObserved === true && Boolean(observed?.vendorSessionId)
        : record.caseMode === 'grokWrite' ? observed?.writeObserved === true
          : observed?.markerObserved === true),
      `${record.driverId}: original A end-turn, answer or marker, and stopped pinned H session`);
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

const hEntries = (session, action) => journal.operations.filter(entry =>
  entry.request?.family === 'K-SESSION' && entry.request.targetId === session.id &&
  entry.request.operation === action);
function stoppedReleased(session) {
  const stops = hEntries(session, 'stop'), releases = hEntries(session, 'admission-release');
  return stops.length === 1 && releases.length === 1 && stops[0].receipt?.status === 'APPLIED' &&
    /^sha256:[a-f0-9]{64}$/.test(stops[0].receipt.result?.stopFact ?? '') &&
    releases[0].receipt?.status === 'APPLIED' &&
    releases[0].request.expectedRevision === stops[0].receipt.revision &&
    stops[0].request.payload?.generation === session.generation &&
    releases[0].request.payload?.generation === session.generation;
}
async function settleFailedGrokWrite() {
  const record = journal.cases.find(row => row.caseMode === 'grokWrite');
  const session = journal.sessions.find(row => row.id === record?.sessionId);
  if (!session || hEntries(session, 'open').length !== 1 ||
      hEntries(session, 'open')[0].receipt?.status !== 'APPLIED') return false;
  const stops = hEntries(session, 'stop');
  if (stops.length > 1 || (stops.length === 1 && stops[0].receipt?.status !== 'APPLIED'))
    throw Error('Original H stop already attempted with unresolved result; custody retained');
  if (stops.length === 0) await sessionStep(session, 'stop', { seatId: session.seatId });
  const stop = hEntries(session, 'stop')[0];
  if (!/^sha256:[a-f0-9]{64}$/.test(stop?.receipt?.result?.stopFact ?? ''))
    throw Error('Original H stop has no exact StopFact; custody retained');
  record.stopFact = stop.receipt.result.stopFact; product.save();
  const releases = hEntries(session, 'admission-release');
  if (releases.length > 1 || (releases.length === 1 && releases[0].receipt?.status !== 'APPLIED'))
    throw Error('Original H admission-release already attempted with unresolved result; custody retained');
  if (releases.length === 0) await sessionStep(session, 'admission-release', { seatId: session.seatId });
  if (!stoppedReleased(session)) throw Error('Original H stop/release receipt relation differs; custody retained');
  return true;
}
async function failureReadback() {
  const outputFile = path.join(config.evidenceDirectory, 'm2-provider-failure-settlement-readback.json');
  if (fs.existsSync(outputFile)) throw Error('Original failure settlement readback already exists');
  await new Promise((resolve, reject) => {
    const child = spawn(config.python, [path.join(here, 'm2-provider-capture-readback.py'),
      config.stateRoot, outputFile, config.result, '--failure-settlement'],
      { windowsHide: true, stdio: ['ignore', 'ignore', 'pipe'] });
    let stderr = ''; child.stderr.on('data', bytes => { stderr = (stderr + bytes).slice(-8192); });
    child.once('error', reject);
    child.once('exit', code => code === 0 ? resolve() :
      reject(Error(`Original failure settlement immutable readback exit=${code}: ${stderr}`)));
  });
  const facts = readJson(outputFile);
  check(facts.state === 'FAIL_ORIGINAL_WRITE_SETTLED' && facts.acceptance === false &&
    facts.measurementPreservedDatabaseBytes === true && facts.originalStopRelease === true &&
    facts.directProviderEvidence === false, 'Original Grok Write failure closed DB readback');
  journal.readbacks.push({ file: path.basename(outputFile), sha256: sha256(outputFile) }); product.save();
}

function compareProtection() {
  for (const observer of config.observers) {
    const before = observerValue(observer.name, 'before'), after = observerValue(observer.name, 'after');
    for (const field of observer.equalFields ?? []) check(Object.hasOwn(before, field) &&
      Object.hasOwn(after, field) && JSON.stringify(before[field]) === JSON.stringify(after[field]),
    `${observer.name}: unchanged ${field}`);
  }
  for (const phase of ['before', 'after']) {
    const memory = observerValue('memory', phase);
    check(memory.memoryDataUnchangedByRead === true && memory.stage1OutputCount === 0 &&
      memory.memoryJobCount === 0, `memory ${phase}: existing read-only observer facts`);
  }
  journal.protection = 'PASS_FORMAL_MEMORY_LEDGER_FIELDS_UNCHANGED'; product.save();
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
  await snapshot('after');
  compareProtection();
  await readbackAndGolden();
  journal.state = 'REVIEW_REQUIRED'; journal.acceptance = false; product.save();
} catch (error) {
  journal.state = 'FAIL'; journal.error = String(error.stack ?? error);
  journal.currentEndpoint = product.endpoint ?? null; product.save(); process.exitCode = 1;
  if (writeCases.length === 1) {
    const session = journal.sessions.find(row => row.id === journal.cases[0]?.sessionId);
    let settled = journal.closes.length === 1 && session && stoppedReleased(session);
    if (!settled && journal.closes.length === 0 &&
        product.child?.exitCode === null && product.socket?.readyState === WebSocket.OPEN) {
      try { settled = await settleFailedGrokWrite(); }
      catch (settlementError) {
        journal.settlementError = String(settlementError.stack ?? settlementError); product.save();
      }
    }
    if (settled && journal.closes.length === 0) {
      try { await product.closeNormally(); }
      catch (closeError) { journal.closeError = String(closeError.stack ?? closeError); product.save(); }
    }
    if (settled && journal.closes.length === 1) {
      if (!Object.hasOwn(journal.snapshots, 'formal-after')) {
        try { await snapshot('after'); compareProtection(); }
        catch (observerError) { journal.afterObserverError = String(observerError.stack ?? observerError); product.save(); }
      }
      try { await failureReadback(); }
      catch (readbackError) { journal.failureReadbackError = String(readbackError.stack ?? readbackError); product.save(); }
    }
    if (journal.closes.length === 0) {
      journal.custodyHold = 'PRODUCT_AND_ORIGINAL_H_RETAINED_STOP_RELEASE_OR_NORMAL_CLOSE_NOT_CONFIRMED';
      journal.failedProductHeld = Boolean(product.child && product.child.exitCode === null);
      product.save();
      product.socket?.close();
      if (product.tester) { try { await product.tester.dispose(); }
        catch (disconnectError) { journal.disconnectError = String(disconnectError); product.save(); } }
      product.child?.unref();
    }
    // The original error remains FAIL even when its H and product closed normally.
  } else {
  // After a normal close, keep the already captured after snapshots and the
  // original readback error. Never dispatch a second after observer.
  if (journal.closes.length === 0) {
    try { await product.preserveFailure(); }
    catch (disconnectError) { journal.disconnectError = String(disconnectError); product.save(); }
  }
  }
}
