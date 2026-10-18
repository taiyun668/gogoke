// Standalone real installed-provider capture. This is a development driver,
// not a host/model substitute or an M2 acceptance decision.
import fs from 'node:fs';
import path from 'node:path';
import { createHash, randomBytes } from 'node:crypto';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { ActualProduct, readJson, sha256, id, delay } from './product-cdp.mjs';
import { captureProtection, compareProtection as compareSharedProtection, protectionSources } from './installed-protection-observation.mjs';
import { readManagedSource } from './m2-provider-managed-source.mjs';
import { createTemporaryHomeLifecycle } from './installed-temporary-home-lifecycle.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const drivers = ['claude', 'opencode', 'grok'];
const atom = value => typeof value === 'string' && /^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/.test(value);
const inside = (child, parent) => child === parent || child.startsWith(parent + path.sep);
const sameWindowsPath = (left, right) => path.resolve(left).toUpperCase() === path.resolve(right).toUpperCase();
const grokHostMcpQuestionSpecs = [
  { header: 'Format', question: 'Which non-secret format should this marker use?',
    options: [{ label: 'JSON', description: 'Use the JSON format.' },
      { label: 'Plain', description: 'Use plain text.' }], recommendedIndex: 1,
    answerMode: 'RECOMMENDED_OPTION' },
  { header: 'Label', question: 'What exact non-secret label should this marker use?',
    options: [{ label: 'Alpha', description: 'Use the first fixed label.' },
      { label: 'Beta', description: 'Use the second fixed label.' }], recommendedIndex: 0,
    answerMode: 'FREE_TEXT' },
];
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
const grokHostMcpCases = config.cases.filter(row => row.caseMode === 'grokHostMcpQuestion');
const grokContextAfterResumeCases = config.cases.filter(row =>
  row.contextAfterResume === true && row.caseMode === 'sequence');
const claudeSequenceCases = config.cases.filter(row => row.caseMode === 'claudeSequence');
if (config.cases.some(row => row.caseMode !== undefined &&
      !['grokWrite', 'sequence', 'claudeSequence', 'grokHostMcpQuestion'].includes(row.caseMode)) ||
    config.cases.some(row => row.contextAfterResume !== undefined &&
      typeof row.contextAfterResume !== 'boolean') ||
    (grokContextAfterResumeCases.length > 0 && (grokContextAfterResumeCases.length !== 1 ||
      sequenceCases.length !== 1 || grokContextAfterResumeCases[0].driverId !== 'grok')) ||
    (sequenceCases.length > 0 && (sequenceCases.length !== 1 || config.cases.length !== 1 ||
      sequenceCases[0].driverId !== 'grok')) ||
    (claudeSequenceCases.length > 0 && (claudeSequenceCases.length !== 1 || config.cases.length !== 1 ||
      claudeSequenceCases[0].driverId !== 'claude' || claudeSequenceCases[0].contextAfterResume !== true)) ||
    config.cases.some(row => row.contextAfterResume === true &&
      row.caseMode !== 'sequence' && row.caseMode !== 'claudeSequence') ||
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
if ((grokHostMcpCases.length > 0 && (grokHostMcpCases.length !== 1 || config.cases.length !== 1 ||
      grokHostMcpCases[0].driverId !== 'grok' || grokHostMcpCases[0].version !== '1.0.41' ||
      grokHostMcpCases[0].sha256 !== 'ab5d2a424f08281798acbdbb06076166fe000d7995ede94a673417b805210a25' ||
      grokHostMcpCases[0].contextAfterResume === true || grokHostMcpCases[0].writeMarker !== undefined))) {
  throw Error('Grok Host-MCP question case requires the one existing fixed Grok instance and executable pin');
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
const homeLifecycle = createTemporaryHomeLifecycle(product,
  path.join(config.evidenceDirectory, 'temporary-home-attempts.json'));
journal.protectionInstrument = protectionSources();
journal.driverBytes['installed-protection-observation.mjs'] = journal.protectionInstrument.module.sha256;
journal.driverBytes['installed-protection-fields.json'] = journal.protectionInstrument.fieldMap.sha256;
const check = (condition, reason) => {
  if (!condition) throw Error(reason);
  if (!journal.assertions.includes(reason)) { journal.assertions.push(reason); product.save(); }
};
const operation = (...args) => product.operation(...args);
const managedSources = new Map();

function observerValue(name, phase) {
  const reference = journal.snapshots[`${name}-${phase}`];
  const file = path.join(config.evidenceDirectory, reference.file);
  check(sha256(file) === reference.sha256, `Original ${name}-${phase} observer bytes`);
  return readJson(file);
}

async function snapshot(phase) {
  await captureProtection(config.observers, null,
    name => path.join(config.evidenceDirectory, `${name}-provider-${phase}.json`),
    (name, reference) => { journal.snapshots[`${name}-${phase}`] = reference; product.save(); });
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
async function runGrokHostMcpQuestionTurns(row, record, session) {
  const hostCase = { state: 'RUNNING', registration: null, turns: [] };
  record.hostMcpQuestion = hostCase; product.save();
  const safeRpcId = value => (typeof value === 'string' && value.length > 0) ||
    (typeof value === 'number' && Number.isSafeInteger(value));
  let sharedBinding = null;
  for (const [ordinal, spec] of grokHostMcpQuestionSpecs.entries()) {
    const turn = { ordinal: ordinal + 1, question: spec, answerMode: spec.answerMode,
      state: 'PRE_SEND', sendRequestId: null, card: null };
    hostCase.turns.push(turn); product.save();
    let before;
    for (;;) {
      const prior = session.cursor;
      before = await output(session);
      check(/^[0-9]+$/.test(before.rawHighwater ?? '') &&
        /^[0-9]+$/.test(before.ledgerHighwater ?? '') &&
        !before.nativeCardRefs?.some(card => card.state === 'OPEN'),
      'Grok Host-MCP case needs an original H page without a pending card');
      if (BigInt(before.cursor) === BigInt(before.ledgerHighwater)) break;
      check(BigInt(before.cursor) > BigInt(prior),
        'Grok Host-MCP pre-send H page made no ledger progress');
    }
    turn.beforeSendRawHighwater = before.rawHighwater;
    const question = { header: spec.header, question: spec.question,
      options: spec.options, recommendedIndex: spec.recommendedIndex };
    const prompt = 'Use the registered ask_user tool exactly once with this one question, and no other tools: ' +
      JSON.stringify(question) + '. After the User answers, repeat that answer exactly in your final response.';
    turn.prompt = prompt; record.prompt = prompt; product.save();
    const sent = await sessionStep(session, 'send', { body: prompt }, ['APPLIED', 'UNKNOWN']);
    const sendOperation = journal.operations.at(-1);
    turn.sendRequestId = sendOperation.request.requestId;
    turn.sendStatus = sent.status; turn.sendRawFrame = sendOperation.rawFrame;
    record.sendRequestId = turn.sendRequestId;
    record.sendStatus = sent.status; record.state = 'SENT_ORIGINAL_ONCE'; product.save();
    if (sent.status === 'UNKNOWN' && sent.result.reason) {
      throw Error(`grok: original Host-MCP H send UNKNOWN reason=${sent.result.reason}; no resend`);
    }
    const cardDeadline = Date.now() + 600000;
    let asking;
    while (Date.now() < cardDeadline) {
      const page = await output(session);
      check(page.nativeCardRefsIncomplete === false,
        'Grok Host-MCP native card reference page is incomplete');
      const matches = (page.nativeCardRefs ?? []).filter(card => card.state === 'OPEN' &&
        card.turnId === turn.sendRequestId);
      check(matches.length <= 1, 'Grok Host-MCP original H send has an ambiguous OPEN card count');
      if (matches.length === 1) { asking = { page, card: matches[0] }; break; }
      const receipt = page.nativeInputReceipts?.find(value => value.requestId === turn.sendRequestId);
      if (receipt?.receipt?.status && receipt.receipt.status !== 'UNKNOWN') {
        throw Error(`grok: original Host-MCP H send terminated before its card (${receipt.receipt.status}); no replay`);
      }
      await delay(300);
    }
    check(asking !== undefined, 'Grok Host-MCP original H send did not produce one current C card');
    turn.cardId = asking.card.cardId; turn.cardRevision = asking.card.revision;
    turn.recoverRequestIds = [];
    let recovered = await operation('K-QCARD', 'recover', turn.cardId, {}, turn.cardRevision,
      ['APPLIED', 'STALE']);
    turn.recoverRequestIds.push(journal.operations.at(-1).request.requestId);
    if (recovered.status === 'STALE') {
      recovered = await operation('K-QCARD', 'recover', turn.cardId, {}, recovered.revision);
      turn.recoverRequestIds.push(journal.operations.at(-1).request.requestId);
    }
    const native = recovered.result.nativeQuestion;
    const shown = native?.questions?.length === 1 ? native.questions[0] : null;
    const binding = native?.binding;
    const hSendBytes = Buffer.from(turn.sendRawFrame, 'utf8');
    const hSendSha256 = createHash('sha256').update(hSendBytes).digest('hex');
    const actualOptions = shown?.options?.map(({ label, description }) => ({ label, description }));
    check(recovered.result.state === 'OPEN' && recovered.result.availableForAnswer === true &&
      recovered.result.turnId === turn.sendRequestId && recovered.result.seatId === row.seatId &&
      recovered.result.generation === session.generation && native?.provider === 'grok' &&
      native.cardOrigin === 'host-mcp' && native.bindingVersion === 1 &&
      native.allowFreeAnswer === true && native.recommendedIndex === spec.recommendedIndex &&
      shown?.id === 'host0' && shown.header === spec.header && shown.question === spec.question &&
      JSON.stringify(actualOptions) === JSON.stringify(spec.options) &&
      binding?.bindingScope === 'INITIAL_H_PROMPT' &&
      binding.hostSessionId === session.id && binding.openRequestId === record.openRequestId &&
      binding.generation === session.generation && binding.hostPromptRequestId === turn.sendRequestId &&
      binding.hostPromptRequestSha256 === hSendSha256 && binding.registrationStepId === 'thread-start' &&
      atom(binding.serverId) && /^[a-f0-9]{64}$/.test(binding.registrationCommandSha256 ?? '') &&
      /^[a-f0-9]{64}$/.test(binding.sourceFrameSha256 ?? '') &&
      binding.sourceOperationId === binding.processOperationId &&
      typeof binding.sourceEpoch === 'string' && binding.sourceEpoch.length > 0 &&
      /^[0-9]+$/.test(binding.sourceCursor ?? '') &&
      ['promptRpcId', 'outerRpcId', 'innerRpcId'].every(key => safeRpcId(binding[key])) &&
      typeof binding.promptStepId === 'string' && binding.promptStepId.length > 0,
    'Grok Host-MCP C card must bind the exact original H prompt, registered server, raw A source and typed IDs');
    if (sharedBinding === null) {
      sharedBinding = { serverId: binding.serverId, registrationStepId: binding.registrationStepId,
        registrationCommandSha256: binding.registrationCommandSha256,
        hostSessionId: binding.hostSessionId, openRequestId: binding.openRequestId,
        generation: binding.generation, processOperationId: binding.processOperationId };
      hostCase.registration = sharedBinding;
    } else {
      check(binding.serverId === sharedBinding.serverId &&
        binding.registrationStepId === sharedBinding.registrationStepId &&
        binding.registrationCommandSha256 === sharedBinding.registrationCommandSha256 &&
        binding.hostSessionId === sharedBinding.hostSessionId &&
        binding.openRequestId === sharedBinding.openRequestId &&
        binding.generation === sharedBinding.generation &&
        binding.processOperationId === sharedBinding.processOperationId,
      'Grok Host-MCP turns must share the same original registered H process');
    }
    turn.card = { cardId: turn.cardId, revision: turn.cardRevision,
      requestRef: recovered.result.requestRef, turnId: recovered.result.turnId,
      source: { operationId: binding.sourceOperationId, sourceEpoch: binding.sourceEpoch,
        sourceCursor: binding.sourceCursor, frameSha256: binding.sourceFrameSha256 },
      binding: { ...binding }, nativeQuestion: native };
    turn.state = 'OPEN_C_RECOVERED'; product.save();
    const answer = spec.answerMode === 'RECOMMENDED_OPTION'
      ? spec.options[spec.recommendedIndex].label
      : randomBytes(24).toString('hex');
    check(spec.answerMode !== 'FREE_TEXT' ||
      (!JSON.stringify(spec).includes(answer) && !JSON.stringify(record.seatSettings).includes(answer) &&
       !turn.prompt.includes(answer) && !spec.options.some(option => option.label === answer)),
    'Grok Host-MCP free answer is private and absent from prompt, question, options and seat settings');
    turn.answerMode = spec.answerMode; turn.privateAnswer = answer;
    turn.answerGeneratedAfterCardObservation = true; product.save();
    const answered = await operation('K-QCARD', 'answer', turn.cardId,
      { generation: session.generation, answers: { host0: [answer] } }, recovered.revision,
      ['APPLIED', 'UNKNOWN']);
    const answerOperation = journal.operations.at(-1);
    turn.answerRequestId = answerOperation.request.requestId;
    turn.answerWriteRequestId = turn.answerRequestId;
    turn.answerRequestRawFrame = answerOperation.rawFrame;
    turn.answerStatus = answered.status;
    check(answered.status === 'UNKNOWN' || (answered.result.state === 'ANSWERED' &&
      answered.result.deliveryBasis === 'NATIVE_EXACT_WRITE_RECEIPT' &&
      answered.result.vendorConsumptionConfirmed === false),
    'Grok Host-MCP exact C/H answer write is not vendor answer consumption');
    product.save();
    const terminal = await observeReceipt(session, row, turn);
    check(terminal.receipt?.result?.createdTurn === true &&
      terminal.receipt.result.stopReason === 'end_turn' &&
      typeof terminal.receipt.result.sourceEpoch === 'string' &&
      /^[0-9]+$/.test(terminal.receipt.result.sourceCursor ?? '') &&
      BigInt(terminal.receipt.result.sourceCursor) >= BigInt(turn.beforeSendRawHighwater),
    'Grok Host-MCP original H send must receive its same-turn ACP terminal');
    const outputDeadline = Date.now() + 600000;
    let complete = false;
    while (Date.now() < outputDeadline) {
      const page = await output(session);
      check(/^[0-9]+$/.test(page.rawHighwater ?? '') &&
        /^[0-9]+$/.test(page.ledgerHighwater ?? ''),
      'Grok Host-MCP terminal output cursor shape differs');
      if (BigInt(page.rawHighwater) >= BigInt(terminal.receipt.result.sourceCursor) &&
          BigInt(page.cursor) === BigInt(page.ledgerHighwater)) {
        turn.terminalOutputPage = { rawHighwater: page.rawHighwater,
          ledgerCursor: page.cursor, ledgerHighwater: page.ledgerHighwater };
        complete = true; break;
      }
      await delay(300);
    }
    check(complete, 'Grok Host-MCP original H terminal source was not fully observed');
    turn.terminalReceipt = terminal.receipt; turn.state = 'ORIGINAL_H_TERMINAL_OBSERVED';
    record.prompt = prompt; record.sendRequestId = turn.sendRequestId;
    product.save();
  }
  hostCase.state = 'TWO_ORIGINAL_TURNS_ANSWERED_AND_TERMINAL';
  record.hostMcpQuestionObserved = true;
  record.state = 'ORIGINAL_H_RECEIPTED';
  product.save();
}
function claudeTerminalResult(result, beforeSendRawHighwater) {
  return result?.createdTurn === true &&
    result.deliveryBasis === 'CLAUDE_USER_REPLAY_AND_RESULT' &&
    result.resultSubtype === 'success' && /^[a-f0-9]{64}$/.test(result.rawResultSha256 ?? '') &&
    typeof result.vendorSessionId === 'string' && result.vendorSessionId.length > 0 &&
    typeof result.userUuid === 'string' && result.userUuid.length > 0 &&
    typeof result.sourceEpoch === 'string' && result.sourceEpoch.length > 0 &&
    /^\d+$/.test(result.sourceCursor ?? '') &&
    BigInt(result.sourceCursor) > BigInt(beforeSendRawHighwater);
}

const claudeNaturalSequenceTurns = [
  { prompt: 'What is 241 + 537? Please answer with the number only.', expectedOutput: '778',
    answerForContext: '778' },
  { prompt: 'What is 16 multiplied by 27? Please answer with the number only.',
    expectedOutput: '432' }
];

function claudeAnswerForTurn(session, vendorSessionId, turn, terminalCursor) {
  return session.events.slice(turn.beforeSendEventCount).filter(event =>
    event.sessionUpdate === 'agent_message_chunk' &&
    event._meta?.threadId === vendorSessionId &&
    /^\d+$/.test(event._meta?.rawSourceCursor ?? '') &&
    BigInt(event._meta.rawSourceCursor) > BigInt(turn.beforeSendRawHighwater) &&
    BigInt(event._meta.rawSourceCursor) <= BigInt(terminalCursor))
    .map(event => event.content?.type === 'text' ? event.content.text : '').join('');
}

async function observeClaudeAnswer(session, turn, result, expectedOutput, contextLabel) {
  const deadline = Date.now() + 600000;
  while (Date.now() < deadline) {
    const page = await output(session);
    const pageComplete = /^\d+$/.test(page.rawHighwater ?? '') &&
      /^\d+$/.test(page.cursor ?? '') && /^\d+$/.test(page.ledgerHighwater ?? '') &&
      BigInt(page.rawHighwater) >= BigInt(result.sourceCursor) &&
      page.cursor === page.ledgerHighwater;
    if (pageComplete) {
      const text = claudeAnswerForTurn(session, result.vendorSessionId, turn, result.sourceCursor);
      turn.terminalOutputPage = { rawHighwater: page.rawHighwater,
        ledgerCursor: page.cursor, ledgerHighwater: page.ledgerHighwater };
      if (text.trim() !== expectedOutput) {
        throw Error(`${contextLabel}: complete original raw result and consumed ledger lack its expected answer`);
      }
      turn.answerObserved = true;
      turn.observedAnswer = text.trim();
      return;
    }
    await delay(300);
  }
  throw Error(`${contextLabel}: original raw result was not fully observed before timeout`);
}

async function runSequenceTurns(row, record, session) {
  const contextAfterResume = row.contextAfterResume === true;
  const sequence = { state: 'RUNNING', turns: [],
    steer: { state: 'NOT_RUN_UNSUPPORTED',
      originalCapability: record.sequenceCapability.inTurnSteer,
      reason: 'The original H capability reports no ACP in-turn steer; no new send is substituted' },
    runningInterrupt: { state: 'NOT_RUN_RUNNING_STOP_NOT_DISPATCHED',
      originalCapability: record.sequenceCapability.interruptAndResume,
      reason: contextAfterResume
        ? 'Every configured send reaches its own terminal receipt before H stop; this case does not claim an in-flight interruption'
        : 'Both original sends reached their own terminal receipt before H stop; this case does not claim an in-flight interruption' },
    coldResume: null, firstStopFact: null, finalStopFact: null,
    ...(contextAfterResume ? { contextAfterResume: { state: 'PENDING' } } : {}) };
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
    resumedCapability.result.capabilities?.interruptAndResume ===
      record.sequenceCapability.interruptAndResume &&
    resumedCapability.result.capabilities?.resume ===
      'SOURCE_PRESENT_NATIVE_RESUME_BEHAVIOUR_NOT_RUN',
  `${row.driverId}: resumed exact fixed CLI capability differs`);
  sequence.resumedCapability = { requestId: resumedCapability.requestId,
    processOperationId: resumedCapability.result.processOperationId,
    inTurnSteer: resumedCapability.result.capabilities.inTurnSteer,
    interruptAndResume: resumedCapability.result.capabilities.interruptAndResume,
    resume: resumedCapability.result.capabilities.resume };
  if (contextAfterResume) {
    const first = sequence.turns[0];
    const prompt = 'From the conversation before the cold resume, recall the exact private marker from your first reply. ' +
      'Reply with only that marker. Do not use tools, read or write files, open a browser, access credentials, or switch models.';
    check(typeof first.marker === 'string' && first.marker.length > 0 && !prompt.includes(first.marker),
      `${row.driverId}: resumed-context question must not disclose the original marker`);
    let before;
    for (;;) {
      const prior = session.cursor;
      before = await output(session);
      check(/^\d+$/.test(before.rawHighwater ?? '') &&
        /^\d+$/.test(before.ledgerHighwater ?? '') &&
        !before.nativeCardRefs?.some(card => card.state === 'OPEN'),
      'Resumed-context send needs a complete current-generation H page without an open card');
      if (BigInt(before.cursor) === BigInt(before.ledgerHighwater)) break;
      check(BigInt(before.cursor) > BigInt(prior),
        'Resumed-context pre-send H page made no ledger progress');
    }
    const turn = { ordinal: 3, contextAfterResume: true, marker: first.marker, prompt,
      generation: session.generation, beforeSendRawHighwater: before.rawHighwater,
      beforeSendLedgerHighwater: before.ledgerHighwater,
      processOperationId: sequence.resumedCapability.processOperationId,
      sendRequestId: null, hReceipt: null, markerObserved: false };
    sequence.turns.push(turn); sequence.contextAfterResume = {
      state: 'SENDING_ONE_NEW_PROMPT_AFTER_COLD_RESUME', ordinal: 3,
      generation: session.generation, processOperationId: turn.processOperationId,
      beforeSendRawHighwater: turn.beforeSendRawHighwater,
      beforeSendLedgerHighwater: turn.beforeSendLedgerHighwater,
      prompt, expectedPriorMarker: first.marker };
    product.save();
    const sent = await sessionStep(session, 'send', { body: prompt }, ['APPLIED', 'UNKNOWN']);
    turn.sendRequestId = journal.operations.at(-1).request.requestId;
    turn.sendStatus = sent.status; product.save();
    if (sent.status === 'UNKNOWN' && sent.result.reason) {
      throw Error(`${row.driverId}: original resumed-context send UNKNOWN reason=${sent.result.reason}; no resend`);
    }
    const receipt = await observeReceipt(session, row, turn);
    check(receipt.receipt?.result?.createdTurn === true &&
      receipt.receipt.result.stopReason === 'end_turn' &&
      typeof receipt.receipt.result.sourceEpoch === 'string' &&
      receipt.receipt.result.sourceEpoch !== first.terminalSourceEpoch &&
      /^\d+$/.test(receipt.receipt.result.sourceCursor ?? '') &&
      BigInt(receipt.receipt.result.sourceCursor) > BigInt(turn.beforeSendRawHighwater),
    `${row.driverId}: resumed-context H receipt lacks a new-generation terminal source boundary`);
    const deadline = Date.now() + 600000;
    let terminalObserved = false;
    while (Date.now() < deadline) {
      const page = await output(session);
      if (/^\d+$/.test(page.rawHighwater ?? '') &&
          BigInt(page.rawHighwater) >= BigInt(receipt.receipt.result.sourceCursor) &&
          page.cursor === page.ledgerHighwater) {
        terminalObserved = true; break;
      }
      await delay(300);
    }
    check(terminalObserved, `${row.driverId}: resumed-context terminal source was not fully observed`);
    turn.hReceipt = receipt; turn.terminalSourceEpoch = receipt.receipt.result.sourceEpoch;
    turn.terminalSourceCursor = receipt.receipt.result.sourceCursor;
    sequence.contextAfterResume = { ...sequence.contextAfterResume,
      state: 'ORIGINAL_NEW_CONTEXT_REPLY_AWAITS_CLOSED_RAW_READBACK',
      sendRequestId: turn.sendRequestId, sendStatus: turn.sendStatus,
      terminalSourceEpoch: turn.terminalSourceEpoch,
      terminalSourceCursor: turn.terminalSourceCursor };
    product.save();
  }
  const finalStop = await sessionStep(session, 'stop', { seatId: row.seatId });
  check(/^sha256:[a-f0-9]{64}$/.test(finalStop.result.stopFact ?? '') &&
    finalStop.result.stopFact !== sequence.firstStopFact,
  `${row.driverId}: final original H StopFact missing or reused`);
  sequence.finalStopFact = finalStop.result.stopFact;
  await sessionStep(session, 'admission-release', { seatId: row.seatId });
  sequence.state = contextAfterResume
    ? 'ORIGINAL_TWO_TURNS_COLD_RESUME_CONTEXT_ANSWER_STOP_RELEASE_READBACK_REQUIRED'
    : 'ORIGINAL_TWO_TURNS_COLD_RESUME_STOP_RELEASE_READBACK_REQUIRED';
  record.result = 'DIRECT_H_RECEIPT_A_READBACK_REQUIRED'; product.save();
}

async function runClaudeSequenceTurns(row, record, session) {
  const claudeSequence = row.driverId === 'claude';
  const claudeContext = claudeSequence && row.caseMode === 'claudeSequence' &&
    row.contextAfterResume === true;
  const sequence = { state: 'RUNNING', turns: [],
    steer: claudeContext
      ? { state: 'NOT_RUN_CLAUDE_CONTEXT_CASE',
        originalCapability: record.sequenceCapability.inTurnSteer ?? null,
        reason: 'This case tests native cold resume context; it does not claim in-turn steering' }
      : { state: 'NOT_RUN_UNSUPPORTED',
        originalCapability: record.sequenceCapability.inTurnSteer,
        reason: 'The original H capability reports no ACP in-turn steer; no new send is substituted' },
    runningInterrupt: claudeContext
      ? { state: 'NOT_RUN_CAPABILITY_FIELD_ABSENT', originalCapability: null,
        reason: 'The fixed Claude capability does not advertise an interrupt field; this case does not claim in-turn interruption' }
      : { state: 'NOT_RUN_RUNNING_STOP_NOT_DISPATCHED',
        originalCapability: record.sequenceCapability.interruptAndResume,
        reason: 'Both original sends reached their own terminal receipt before H stop; this case does not claim an in-flight interruption' },
    coldResume: null, firstStopFact: null, finalStopFact: null };
  record.sequence = sequence; product.save();
  for (let ordinal = 1; ordinal <= claudeNaturalSequenceTurns.length; ordinal++) {
    const spec = claudeNaturalSequenceTurns[ordinal - 1];
    const prompt = spec.prompt;
    check(!prompt.includes(spec.expectedOutput) &&
      (!spec.answerForContext || !prompt.includes(spec.answerForContext)),
      'Claude natural question must not disclose its expected answer');
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
    const turn = { ordinal, prompt, expectedOutput: spec.expectedOutput,
      ...(spec.answerForContext ? { answerForContext: spec.answerForContext } : {}),
      generation: session.generation, beforeSendRawHighwater: before.rawHighwater,
      beforeSendLedgerHighwater: before.ledgerHighwater,
      beforeSendEventCount: session.events.length, sendRequestId: null, hReceipt: null };
    sequence.turns.push(turn); product.save();
    const sent = await sessionStep(session, 'send', { body: prompt }, ['APPLIED', 'UNKNOWN']);
    turn.sendRequestId = journal.operations.at(-1).request.requestId;
    turn.sendStatus = sent.status; product.save();
    if (sent.status === 'UNKNOWN' && sent.result.reason) {
      throw Error(`${row.driverId}: original sequence send UNKNOWN reason=${sent.result.reason}; no resend`);
    }
    const receipt = await observeReceipt(session, row, turn);
    check(receipt.receipt?.status === 'APPLIED' &&
      claudeTerminalResult(receipt.receipt.result, turn.beforeSendRawHighwater) &&
      (!sequence.vendorSessionId || sequence.vendorSessionId === receipt.receipt.result.vendorSessionId),
    `${row.driverId}: original sequence H terminal receipt is incomplete`);
    sequence.vendorSessionId = receipt.receipt.result.vendorSessionId;
    turn.userUuid = receipt.receipt.result.userUuid;
    turn.rawResultSha256 = receipt.receipt.result.rawResultSha256;
    await observeClaudeAnswer(session, turn, receipt.receipt.result,
      spec.expectedOutput, `Claude natural turn ${ordinal}`);
    turn.terminalSourceEpoch = receipt.receipt.result.sourceEpoch;
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
    (claudeSequence ? Object.hasOwn(resumed.result, 'threadId') &&
      resumed.result.readinessBasis === 'ORIGINAL_CLAUDE_INITIALIZE_ACK' &&
      (resumed.result.threadId === null || resumed.result.threadId === sequence.vendorSessionId) :
      (!resumed.result.threadId || resumed.result.threadId === session.threadId)),
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
    (!claudeSequence || (Object.hasOwn(resumedCapability.result, 'threadId') &&
      (resumedCapability.result.threadId === null ||
        resumedCapability.result.threadId === sequence.vendorSessionId))) &&
    (claudeContext
      ? typeof resumedCapability.result.capabilities?.resume === 'string' &&
        resumedCapability.result.capabilities?.interruptAndResume === undefined
      : resumedCapability.result.capabilities?.inTurnSteer === 'UNSUPPORTED' &&
        resumedCapability.result.capabilities?.resume ===
          'SOURCE_PRESENT_NATIVE_RESUME_BEHAVIOUR_NOT_RUN'),
  `${row.driverId}: resumed exact fixed CLI capability differs`);
  sequence.resumedCapability = claudeContext
    ? { requestId: resumedCapability.requestId,
      processOperationId: resumedCapability.result.processOperationId,
      inTurnSteer: resumedCapability.result.capabilities.inTurnSteer ?? null,
      resume: resumedCapability.result.capabilities.resume,
      interruptAndResume: resumedCapability.result.capabilities.interruptAndResume ?? null,
      evidenceBasis: resumedCapability.result.evidenceBasis }
    : { requestId: resumedCapability.requestId,
      processOperationId: resumedCapability.result.processOperationId,
      inTurnSteer: resumedCapability.result.capabilities.inTurnSteer,
      resume: resumedCapability.result.capabilities.resume };
  if (claudeContext) {
    const priorAnswer = sequence.turns[0].answerForContext;
    let before;
    for (;;) {
      const priorCursor = session.cursor;
      before = await output(session);
      check(/^\d+$/.test(before.rawHighwater ?? '') &&
        /^\d+$/.test(before.ledgerHighwater ?? '') &&
        !before.nativeCardRefs?.some(card => card.state === 'OPEN'),
      'Claude resumed H page must be complete and have no open card');
      if (before.cursor === before.ledgerHighwater) break;
      check(BigInt(before.cursor) > BigInt(priorCursor),
        'Claude resumed H page made no ledger progress');
    }
    const prompt = 'What was the exact answer to my first arithmetic question? Reply with the number only.';
    check(!prompt.includes(priorAnswer),
      'Claude post-resume natural question must not disclose the earlier answer');
    const contextTurn = { ordinal: 3, contextAfterResume: true, priorTurnOrdinal: 1,
      expectedPriorAnswer: priorAnswer, expectedOutput: priorAnswer, prompt,
      generation: session.generation, beforeSendRawHighwater: before.rawHighwater,
      beforeSendLedgerHighwater: before.ledgerHighwater,
      beforeSendEventCount: session.events.length,
      processOperationId: sequence.resumedCapability.processOperationId,
      sendRequestId: null, hReceipt: null, state: 'SENDING_ONCE' };
    sequence.contextAfterResume = contextTurn; product.save();
    const sent = await sessionStep(session, 'send', { body: prompt }, ['APPLIED', 'UNKNOWN']);
    contextTurn.sendRequestId = journal.operations.at(-1).request.requestId;
    contextTurn.sendStatus = sent.status; product.save();
    if (sent.status === 'UNKNOWN' && sent.result.reason) {
      throw Error('claude: original post-resume send UNKNOWN; do not resend');
    }
    const receipt = await observeReceipt(session, row, contextTurn);
    check(receipt.receipt?.status === 'APPLIED' &&
      claudeTerminalResult(receipt.receipt.result, contextTurn.beforeSendRawHighwater) &&
      receipt.receipt.result.vendorSessionId === sequence.vendorSessionId &&
      receipt.receipt.result.sourceEpoch !== sequence.turns[0].terminalSourceEpoch &&
      receipt.receipt.result.sourceEpoch !== sequence.turns[1].terminalSourceEpoch,
    'Claude post-resume original H terminal receipt is incomplete');
    contextTurn.userUuid = receipt.receipt.result.userUuid;
    contextTurn.rawResultSha256 = receipt.receipt.result.rawResultSha256;
    await observeClaudeAnswer(session, contextTurn, receipt.receipt.result,
      priorAnswer, 'Claude post-resume context answer');
    contextTurn.terminalSourceEpoch = receipt.receipt.result.sourceEpoch;
    contextTurn.terminalSourceCursor = receipt.receipt.result.sourceCursor;
    contextTurn.state = 'ORIGINAL_H_RECEIPTED'; product.save();
  }
  const finalStop = await sessionStep(session, 'stop', { seatId: row.seatId });
  check(/^sha256:[a-f0-9]{64}$/.test(finalStop.result.stopFact ?? '') &&
    finalStop.result.stopFact !== sequence.firstStopFact,
  `${row.driverId}: final original H StopFact missing or reused`);
  sequence.finalStopFact = finalStop.result.stopFact;
  await sessionStep(session, 'admission-release', { seatId: row.seatId });
  sequence.state = claudeContext
    ? 'CLAUDE_CONTEXT_AFTER_RESUME_STOP_RELEASE_READBACK_REQUIRED'
    : 'ORIGINAL_TWO_TURNS_COLD_RESUME_STOP_RELEASE_READBACK_REQUIRED';
  record.result = 'DIRECT_H_RECEIPT_A_READBACK_REQUIRED'; product.save();
}

async function runCase(row, instances) {
  const grokWrite = row.caseMode === 'grokWrite';
  const record = { caseId: id('m2ProviderCase'), driverId: row.driverId, instanceId: row.instanceId, seatId: row.seatId,
    worktreeId: row.worktreeId, fixedVersion: row.version, fixedSha256: row.sha256,
    expectedModel: row.model, expectedEffort: row.effort, caseMode: row.caseMode ?? 'singleAnswer',
    ...(row.contextAfterResume === true ? { contextAfterResume: true } : {}),
    state: 'PREFLIGHT', acceptance: false };
  journal.cases.push(record); product.save();
  homeLifecycle.beginCase(record.caseId);
  let businessError = null;
  let teardownJournalError = null;
  try {
  // Sequence branches return from the business body. Keep those returns inside
  // this body so the shared finally and the post-finally refusal still run.
  await (async () => {
  const instance = instances.instances.find(value => value.instanceId === row.instanceId && value.driverId === row.driverId);
  // This one preserved Claude test instance predates managed CLI lifecycle
  // records. Its raw install_state is UNKNOWN, which the UI maps to ERROR.
  // Record that UI fact and use the sealed original registry metadata for
  // this case only. Real H admission still verifies the current instance and
  // actual program source/version/bytes; no installation state is invented.
  let originalRegistry;
  if (row.originalRegistryRead !== undefined) {
    check(row.driverId === 'claude' && row.instanceId === 'claudeTestM2' &&
      row.version === '2.1.196' && row.sha256 ===
      '180d7b279455e8b89d4353a5146447be2f80b80fb0db14bdc6dd9cb98c0aef09',
    'sealed legacy registry entry is restricted to the original fixed Claude case');
    const proof = row.originalRegistryRead;
    check(proof && typeof proof.path === 'string' && path.isAbsolute(proof.path) &&
      /^[a-f0-9]{64}$/.test(proof.sha256 ?? '') && sha256(proof.path) === proof.sha256,
    'original closed registry readback seal differs');
    const raw = readJson(proof.path);
    check(raw.databaseBytesPreserved === true && raw.mutation === false &&
      raw.bindings?.length === 1 && raw.bindings[0].driver === row.driverId,
    'original registry readback is missing or ambiguous');
    originalRegistry = raw.bindings[0].instance;
    check(originalRegistry.instance_id === row.instanceId &&
      originalRegistry.driver_id === row.driverId && originalRegistry.version === row.version &&
      originalRegistry.program_digest === `sha256:${row.sha256}` &&
      originalRegistry.install_state === 'UNKNOWN' && originalRegistry.login_state === 'LOGGED_IN',
    'original legacy instance registry pin or login fact differs');
    record.originalRegistry = { sourceSha256: proof.sha256, fact: originalRegistry,
      uiState: instance?.state ?? 'NOT_REPORTED', actualProgramCheck: 'PENDING_REAL_H_ADMISSION' };
    product.save();
  }
  // The native managed source is independent of the aggregate UI state.
  // Preserve both facts; the original real H admission and pin below still run.
  const managedSource = managedSources.get(row.instanceId);
  if (managedSource) {
    check(instance?.version === row.version && typeof instance.state === 'string',
      'managed OpenCode must still have its original instance in the actual UI read');
    record.managedSourceRead = { ...managedSource, uiState: instance.state,
      actualProgramCheck: 'PENDING_REAL_H_ADMISSION' };
    product.save();
  }
  if ((!instance || instance.state !== 'LOGGED_IN') && !originalRegistry && !managedSource) {
    record.result = 'NOT_RUN_NOT_LOGGED_IN'; product.save();
    throw Error(`${row.driverId}: configured instance not logged in; no login or fallback attempted`);
  }
  record.loggedInInstance = managedSource
    ? { instanceId: row.instanceId, driverId: row.driverId, version: managedSource.fact.version,
      state: managedSource.fact.login_state, basis: 'ORIGINAL_MANAGED_SOURCE_WITH_REAL_H_ADMISSION_REQUIRED' }
    : originalRegistry
    ? { instanceId: row.instanceId, driverId: row.driverId, version: originalRegistry.version,
      state: originalRegistry.login_state, basis: 'ORIGINAL_REGISTRY_WITH_REAL_H_ADMISSION_REQUIRED' }
    : { instanceId: instance.instanceId, driverId: instance.driverId,
      version: instance.version, state: instance.state };
  check(record.loggedInInstance.version === row.version, `${row.driverId}: actual logged-in instance version pin`);
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
  const session = { id: id('m2ProviderSession'), caseId: record.caseId,
    seatId: row.seatId, instanceId: row.instanceId, worktreeId: row.worktreeId,
    generation: (BigInt(seat.result.generation) + 1n).toString(), revision: '0', cursor: '0', events: [], turns: [] };
  record.sessionId = session.id; journal.sessions.push(session); product.save();
  homeLifecycle.registerProducer(session, row.instanceId, record.caseId);
  for (const action of ['admission-reserve', 'admission-commit']) await sessionStep(session, action, { seatId: row.seatId });
  const opened = await sessionStep(session, 'open', { seatId: row.seatId,
    repositoryId: config.repositoryId, worktreeId: row.worktreeId });
  record.openRequestId = journal.operations.at(-1).request.requestId;
  if (row.driverId === 'claude' && row.caseMode === 'claudeSequence') {
    check(Object.hasOwn(opened.result, 'threadId') && opened.result.threadId === null &&
      opened.result.readinessBasis === 'ORIGINAL_CLAUDE_INITIALIZE_ACK',
    'Claude original open must preserve the actual lazy vendor-session identity ACK');
  }
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
  if (row.caseMode === 'sequence' || row.caseMode === 'claudeSequence') {
    const capabilities = capability.result.capabilities;
    if (row.caseMode === 'sequence') {
      check(capabilities?.inTurnSteer === 'UNSUPPORTED' &&
        capabilities?.resume === 'SOURCE_PRESENT_NATIVE_RESUME_BEHAVIOUR_NOT_RUN' &&
        capabilities?.interruptAndResume === 'SOURCE_PRESENT_NATIVE_RESUME_BEHAVIOUR_NOT_RUN',
      `${row.driverId}: original H ACP sequence capability facts differ`);
    } else {
      check(row.driverId === 'claude' && row.contextAfterResume === true &&
        typeof capabilities?.resume === 'string' && capabilities?.interruptAndResume === undefined,
      'Claude case must preserve the original native-resume capability claim');
    }
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
  if (row.caseMode === 'claudeSequence') {
    await runClaudeSequenceTurns(row, record, session);
    return;
  }
  if (row.caseMode === 'grokHostMcpQuestion') {
    await runGrokHostMcpQuestionTurns(row, record, session);
    const stopped = await sessionStep(session, 'stop', { seatId: row.seatId });
    check(typeof stopped.result.stopFact === 'string' && stopped.result.stopFact.length > 0,
      'grok: original Host-MCP process normal stop fact');
    record.stopFact = stopped.result.stopFact;
    await sessionStep(session, 'admission-release', { seatId: row.seatId });
    record.result = 'DIRECT_H_RECEIPT_A_READBACK_REQUIRED'; product.save();
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
  } else if (row.driverId === 'claude' || row.driverId === 'opencode') {
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
  })();
  } catch (error) {
    businessError = error;
  } finally {
    try {
      const teardown = await homeLifecycle.finishCase(record.caseId);
      record.homeTeardown = { state: teardown.state, complete: teardown.complete,
        errors: teardown.errors };
      if (!teardown.complete) record.result = 'CASE_TEARDOWN_INCOMPLETE';
    } catch (teardownError) {
      record.homeTeardown = { state: 'CASE_TEARDOWN_INCOMPLETE', complete: false,
        error: String(teardownError?.stack ?? teardownError) };
      record.result = 'CASE_TEARDOWN_INCOMPLETE';
    }
    try { product.save(); }
    catch (error) { teardownJournalError = error; }
  }
  if (businessError) throw businessError;
  if (teardownJournalError) throw teardownJournalError;
  if (record.homeTeardown?.complete !== true) {
    throw Error(`${row.driverId}: CASE_TEARDOWN_INCOMPLETE; next creating case blocked`);
  }
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
    const providerOutputObserved = record.expectedAnswer === '778'
      ? observed?.answerObserved === true && Boolean(observed?.vendorSessionId)
      : record.caseMode === 'grokHostMcpQuestion'
        ? observed?.hostMcpQuestionObserved === true && observed?.markerObserved === true
        : record.caseMode === 'grokWrite' ? observed?.writeObserved === true
          : observed?.markerObserved === true;
    check(observed?.driverId === record.driverId && observed?.version === record.fixedVersion &&
      observed?.binarySha256 === `sha256:${record.fixedSha256}` && observed?.allEpisodesStopped &&
      observed?.providerEndTurn === true && providerOutputObserved,
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
  return facts;
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
  journal.protectionComparison = compareSharedProtection(config.observers, observerValue);
  check(journal.protectionComparison.allEqual, 'Original thirteen protection fields changed');
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
      'm2-provider-managed-source.mjs', 'm2-provider-capture-readback.py']
      .map(name => [name, sha256(path.join(here, name))])) };
  await snapshot('before');
  await homeLifecycle.captureBaseline();
  // Verify the sealed closure before startup can change the original database.
  for (const row of config.cases) {
    const proof = readManagedSource(row, config.stateRoot);
    if (proof) managedSources.set(row.instanceId, proof);
  }
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
  let homesClosed = false;
  try { homesClosed = await homeLifecycle.verifyClosedReadback(); }
  catch (readbackError) {
    journal.temporaryHomeReadbackError = String(readbackError?.stack ?? readbackError);
  }
  if (!homesClosed) {
    journal.state = 'CASE_TEARDOWN_INCOMPLETE'; journal.acceptance = false; product.save();
    process.exitCode = 1;
  } else {
    journal.state = homeLifecycle.manifest.state === 'NO_HOME_CREATE_DISPATCHED'
      ? 'REVIEW_REQUIRED_NO_HOME_CREATE_DISPATCHED' : 'REVIEW_REQUIRED';
    journal.acceptance = false; product.save();
  }
} catch (error) {
  journal.state = 'FAIL'; journal.error = String(error.stack ?? error);
  journal.currentEndpoint = product.endpoint ?? null; product.save(); process.exitCode = 1;
  if (homeLifecycle.manifest.attempts.length && product.socket?.readyState === WebSocket.OPEN) {
    try { await homeLifecycle.beforeCaptionClose(); }
    catch (teardownError) {
      journal.temporaryHomeLifecycleError = String(teardownError.stack ?? teardownError);
      product.save();
    }
  }
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
  // A sequence already closed its first launch. Only the current child proves
  // that the cold launch is closed; the old close count cannot settle it.
  if (product.child?.exitCode === null && product.child.signalCode === null) {
    journal.custodyHold = 'CURRENT_PRODUCT_AND_ORIGINAL_H_RETAINED_CONTROLLER_SETTLEMENT_REQUIRED';
    product.save();
    try { await product.preserveFailure(); }
    catch (disconnectError) { journal.disconnectError = String(disconnectError); product.save(); }
  }
  }
}
