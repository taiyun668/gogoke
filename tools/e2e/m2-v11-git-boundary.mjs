// Original H/Codex Git availability and private vendor-worktree boundary.
// Importing this module never launches a product, model, or Git process.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { id, delay } from './product-cdp.mjs';

const atom = value => typeof value === 'string' && /^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/.test(value);
const check = (ok, message) => { if (!ok) throw Error(message); };
const sha = bytes => crypto.createHash('sha256').update(bytes).digest('hex');
const plain = value => typeof value === 'string' && !/["%!^&|<>\r\n]/.test(value);
const same = (a, b) => path.resolve(a).toLowerCase() === path.resolve(b).toLowerCase();

function sourceSnapshot(source) {
  const entries = [];
  const visit = (folder, prefix) => {
    for (const name of fs.readdirSync(folder).sort()) {
      check(/^[\x20-\x7e]+$/.test(name),
        'V11 private source snapshot requires ASCII fixture names');
      const full = path.join(folder, name);
      const stat = fs.lstatSync(full);
      check(!stat.isSymbolicLink() && same(full, fs.realpathSync.native(full)),
        'V11 private source has a link; preserve and stop');
      const relative = prefix ? prefix + '/' + name : name;
      if (stat.isDirectory()) {
        entries.push([relative + '/', null]);
        visit(full, relative);
      } else {
        check(stat.isFile(), 'V11 private source has a nonordinary entry');
        entries.push([relative, sha(fs.readFileSync(full))]);
      }
    }
  };
  visit(source, '');
  return sha(Buffer.from(JSON.stringify(entries)));
}

function setup(config) {
  const c = config.v11GitBoundary;
  check(process.platform === 'win32' && config.repositoryId === 'gogokeSeatTestbed' &&
    c?.ownership === 'EXCLUSIVE_V11_PRIVATE_GIT_BOUNDARY' &&
    ['seatId', 'instanceId', 'worktreeId'].every(key => atom(c[key])) &&
    typeof c.worktreePath === 'string' && path.isAbsolute(c.worktreePath) &&
    typeof c.privateTestbedRoot === 'string' && path.isAbsolute(c.privateTestbedRoot) &&
    typeof config.testbedSource === 'string' && path.isAbsolute(config.testbedSource) &&
    fs.statSync(c.worktreePath).isDirectory() &&
    fs.statSync(config.testbedSource).isDirectory() &&
    fs.statSync(c.privateTestbedRoot).isDirectory() &&
    same(config.testbedSource, fs.realpathSync.native(config.testbedSource)) &&
    same(c.privateTestbedRoot, fs.realpathSync.native(c.privateTestbedRoot)) &&
    same(c.privateTestbedRoot, path.dirname(config.testbedSource)) &&
    !same(c.privateTestbedRoot, path.parse(c.privateTestbedRoot).root) &&
    !same(c.worktreePath, config.testbedSource) &&
    !same(c.privateTestbedRoot, config.stateRoot),
  'V11 requires one exclusive registered F tree and a private source sibling root');
  check(c.fixedGitPath === undefined ||
    (path.isAbsolute(c.fixedGitPath) && plain(c.fixedGitPath) &&
     fs.lstatSync(c.fixedGitPath).isFile() &&
     same(c.fixedGitPath, fs.realpathSync.native(c.fixedGitPath))),
  'V11 optional fixed Git path must identify an existing ordinary absolute program');
  check(plain(c.worktreePath) && plain(config.testbedSource) &&
    plain(c.privateTestbedRoot), 'V11 CMD path has expansion or quoting characters');
  return c;
}

async function originalCommand(product, config, journal, record, phase, command) {
  const c = setup(config);
  const launch = journal.launches?.at(-1);
  check(launch?.sourceCommit === record.sourceCommit &&
    launch.setId === record.installedSha256?.['resource-index.json'] &&
    launch.bootstrap?.version === record.candidateVersion &&
    launch.bootstrap?.setId === launch.setId &&
    launch.bootstrap?.generationId === launch.generationId &&
    journal.launches.length === (journal.closes?.length ?? 0) + 1,
  'V11 original H command must run in the current self-reported installed candidate');
  const read = async (family, operation, target, payload = {}, revision = '0') => {
    let reply = await product.operation(family, operation, target, payload, revision, ['APPLIED', 'STALE']);
    if (reply.status === 'STALE') reply = await product.operation(family, operation, target, payload, reply.revision);
    return reply;
  };
  const seat = await read('K-SEAT', 'state-card', c.seatId);
  const seatCardRequestId = journal.operations.at(-1).request.requestId;
  const graph = await read('K-WORKTREE', 'graph-query', c.worktreeId);
  const graphRequestId = journal.operations.at(-1).request.requestId;
  check(seat.result.state === 'IDLE' && seat.result.instanceId === c.instanceId &&
    seat.result.settings?.permissionTier === 'NETWORKED_WRITE' &&
    graph.result.state === 'REGISTERED' && graph.result.members?.some(member =>
      member.worktreeId === c.worktreeId && member.repositoryId === config.repositoryId &&
      member.domainId === config.domainId && member.seatId === c.seatId &&
      member.instanceId === c.instanceId),
  'V11 original E/F seat, tier or tree differs');
  const session = { id: id('v11GitSession'), seatId: c.seatId, instanceId: c.instanceId,
    worktreeId: c.worktreeId, generation: (BigInt(seat.result.generation) + 1n).toString(),
    revision: '0', cursor: '0', events: [], turns: [] };
  journal.sessions ??= [];
  journal.sessions.push(session);
  const attempt = { phase, sessionId: session.id, seatId: c.seatId,
    instanceId: c.instanceId, worktreeId: c.worktreeId,
    worktreePath: c.worktreePath, command, seatCardRequestId, graphRequestId,
    state: 'H_ADMISSION_PENDING', acceptance: false };
  record.attempts.push(attempt); product.save();
  const step = async (operation, payload = {}) => {
    const receipt = await product.operation('K-SESSION', operation, session.id,
      { generation: session.generation, ...payload }, session.revision);
    session.revision = receipt.revision; product.save(); return receipt;
  };
  await step('admission-reserve', { seatId: c.seatId });
  await step('admission-commit', { seatId: c.seatId });
  const opened = await step('open', { seatId: c.seatId,
    repositoryId: config.repositoryId, worktreeId: c.worktreeId });
  session.threadId = opened.result.threadId;
  attempt.openRequestId = journal.operations.at(-1).request.requestId;
  check(typeof session.threadId === 'string' && session.threadId,
    'V11 original H open lacks Codex thread');
  const body = 'Owner-authorized private V11 ' + phase + ' case. Invoke builtin exec_command ' +
    'exactly once with cmd ' + JSON.stringify(command) + ', workdir ' +
    JSON.stringify(c.worktreePath) + ', shell "cmd.exe", login false. ' +
    'Do not substitute a command or tool, retry, use the network or credentials. ' +
    'Preserve the original tool result and finish the turn.';
  attempt.body = body; attempt.state = 'H_OPEN'; product.save();
  const sent = await step('send', { body });
  attempt.sendRequestId = journal.operations.at(-1).request.requestId;
  attempt.sendReceipt = sent; attempt.turnId = sent.result.turnId; product.save();
  check(sent.result.createdTurn === true && attempt.turnId,
    'V11 original H send lacks created native turn');
  const deadline = Date.now() + 600000;
  while (Date.now() < deadline) {
    const page = await read('K-SESSION', 'output-stream', session.id,
      { generation: session.generation, afterCursor: session.cursor }, session.revision);
    check(page.result.generation === session.generation &&
      BigInt(page.result.cursor) >= BigInt(session.cursor) && !page.result.sourceError,
    'V11 original H output source or cursor differs');
    session.cursor = page.result.cursor; session.revision = page.revision;
    session.events.push(...page.result.events); product.save();
    if (session.events.some(event => event._meta?.codexMethod === 'turn/completed' &&
      event._meta.turnId === attempt.turnId && event._meta.threadId === session.threadId)) break;
    await delay(300);
  }
  const terminal = session.events.find(event => event._meta?.codexMethod === 'turn/completed' &&
    event._meta.turnId === attempt.turnId && event._meta.threadId === session.threadId);
  check(terminal && ['completed', 'failed'].includes(terminal._meta.turnStatus),
    'V11 original Codex turn did not terminate; preserve H process');
  attempt.liveToolItems = session.events.filter(event =>
    event._meta?.codexMethod === 'item/completed' &&
    event._meta.turnId === attempt.turnId &&
    ['fileChange', 'commandExecution', 'mcpToolCall', 'dynamicToolCall'].includes(event._meta.codexItemType))
    .map(event => ({ itemId: event.toolCallId, type: event._meta.codexItemType,
      status: event.status, rawOutput: event.rawOutput }));
  attempt.state = 'ORIGINAL_TURN_TERMINAL_RAW_READBACK_REQUIRED'; product.save();
  const stopped = await step('stop', { seatId: c.seatId });
  check(typeof stopped.result.stopFact === 'string' && stopped.result.stopFact,
    'V11 original H stop lacks StopFact');
  attempt.stopRequestId = journal.operations.at(-1).request.requestId;
  attempt.stopFact = stopped.result.stopFact; product.save();
  await step('admission-release', { seatId: c.seatId });
  attempt.releaseRequestId = journal.operations.at(-1).request.requestId;
  session.turns.push({ turnId: attempt.turnId, sendRequestId: attempt.sendRequestId });
  attempt.state = 'H_STOP_RELEASED_REQUIRES_NORMAL_CLOSE_READBACK'; product.save();
  return attempt;
}

export async function runV11GitProbe(product, config, journal) {
  const c = setup(config);
  check(!journal.v11GitBoundary, 'V11 Git probe uses one fresh original case');
  const command = c.fixedGitPath ? '"' + c.fixedGitPath + '" --version' : 'git --version';
  const record = { state: 'PROBE_RUNNING', acceptance: false,
    sourceCommit: config.sourceCommit, candidateVersion: config.version,
    installedSha256: config.installedSha256, domainId: config.domainId,
    repositoryId: config.repositoryId, testbedSource: config.testbedSource,
    privateTestbedRoot: c.privateTestbedRoot, fixedGitPath: c.fixedGitPath ?? null,
    probeCommand: command, attempts: [] };
  journal.v11GitBoundary = record; product.save();
  await originalCommand(product, config, journal, record, 'GIT_VERSION', command);
  record.state = 'PROBE_REQUIRES_NORMAL_CLOSE_IMMUTABLE_READBACK'; product.save();
  return record;
}

export async function runV11GitVendorAttempt(product, config, journal) {
  const c = setup(config);
  const record = journal.v11GitBoundary;
  check(record?.state === 'PROBE_REQUIRES_NORMAL_CLOSE_IMMUTABLE_READBACK' &&
    record.attempts?.length === 1 && c.fixedGitPath &&
    record.sourceCommit === config.sourceCommit &&
    record.candidateVersion === config.version &&
    JSON.stringify(record.installedSha256) === JSON.stringify(config.installedSha256) &&
    record.domainId === config.domainId && record.repositoryId === config.repositoryId &&
    same(c.fixedGitPath, record.fixedGitPath) &&
    same(config.testbedSource, record.testbedSource) &&
    typeof c.probeReadback === 'string' && path.isAbsolute(c.probeReadback) &&
    !same(c.probeReadback, config.stateRoot) &&
    !path.resolve(c.probeReadback).toLowerCase().startsWith(
      path.resolve(c.privateTestbedRoot).toLowerCase() + path.sep) &&
    fs.statSync(c.probeReadback).isFile(),
  'V11 vendor attempt requires the prior fixed Git probe and fresh immutable reader');
  const bytes = fs.readFileSync(c.probeReadback);
  const proof = JSON.parse(bytes.toString('utf8'));
  check(proof.schema === 'gogoke.37.private-v11-git-readback.v1' &&
    proof.phase === 'probe' && proof.fixedGitReachable === true &&
    proof.acceptance === false && proof.sourceCommit === record.sourceCommit &&
    proof.probeTurnId === record.attempts[0].turnId &&
    proof.probeStopFact === record.attempts[0].stopFact &&
    proof.probeCommand === record.probeCommand &&
    proof.registeredGitDigest === 'sha256:' + sha(fs.readFileSync(c.fixedGitPath)) &&
    journal.closes?.some(row => row.pid === proof.normalClosePid &&
      row.exitCode === 0 && row.forceKill === false) &&
    same(proof.fixedGitPath, record.fixedGitPath),
  'V11 original fixed Git reachability was not proven by the closed reader');
  const target = path.join(c.privateTestbedRoot, id('v11-unregistered-vendor-tree'));
  check(!fs.existsSync(target), 'V11 private unregistered vendor target already exists');
  const command = '"' + c.fixedGitPath + '" -C "' + config.testbedSource +
    '" worktree add --detach "' + target + '" HEAD';
  record.probeReadbackPath = c.probeReadback;
  record.probeReadbackSha256 = sha(bytes);
  record.vendorCommand = command; record.vendorTarget = target;
  record.vendorTargetAbsentBefore = true;
  record.beforeSourceSha256 = sourceSnapshot(config.testbedSource);
  record.state = 'VENDOR_ATTEMPT_RUNNING'; product.save();
  await originalCommand(product, config, journal, record, 'VENDOR_WORKTREE_ADD', command);
  record.afterSourceSha256 = sourceSnapshot(config.testbedSource);
  record.vendorTargetExistsAfter = fs.existsSync(target);
  record.state = 'VENDOR_ATTEMPT_REQUIRES_NORMAL_CLOSE_IMMUTABLE_READBACK';
  product.save();
  return record;
}
