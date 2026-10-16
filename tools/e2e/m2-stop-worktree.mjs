// Original User/H/F cleanup stop-gate case. Importing starts nothing.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { id, sha256 } from './product-cdp.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const atom = value => typeof value === 'string' && /^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/.test(value);
const check = (value, message) => { if (!value) throw Error(message); };

export async function runStopWorktreeCase(product, config, journal) {
  const c = config.stopWorktree;
  if (!c) return { state: 'NOT_RUN_NOT_CONFIGURED', acceptance: false };
  check(process.platform === 'win32' && config.repositoryId === 'gogokeSeatTestbed' &&
    /^[a-f0-9]{40}$/.test(config.sourceCommit) && c.lifecycleOwnership === 'EXCLUSIVE_M2_STOP_WORKTREE' &&
    atom(c.worktreeId) && atom(c.seatId) && c.seatId !== config.seatId &&
    typeof c.normalCloseReadbackRestart === 'function' && product.tester &&
    product.tester.page.url() === product.endpoint.url &&
    journal.connectionBackend?.agentActs === 0 && journal.connectionBackend?.telemetryDisabled === true &&
    !journal.stopWorktree && !journal.sessions.some(row => row.worktreeId === c.worktreeId),
  'V07 case requires an exclusive actual registered SINGLE worktree, idle seat, installed User ingress and close/readback callback');

  const record = { schema: 'gogoke.37.m2-stop-worktree.v1', state: 'RUNNING', acceptance: false,
    sourceCommit: config.sourceCommit, domainId: config.domainId, repositoryId: config.repositoryId,
    stateRoot: path.resolve(config.stateRoot), evidenceDirectory: path.resolve(config.evidenceDirectory),
    candidateVersion: config.version, candidateInstalledSha256: config.installedSha256,
    worktreeId: c.worktreeId, seatId: c.seatId, instanceId: config.instanceId,
    driverSha256: sha256(path.join(here, 'm2-stop-worktree.mjs')),
    readerSha256: sha256(path.join(here, 'm2-stop-worktree-readback.py')),
    session: { id: id('m2StopSession'), generation: null, revision: '0',
      reserveRequestId: null, commitRequestId: null, openRequestId: null,
      stopRequestId: null, releaseRequestId: null, stopFactId: null },
    preStopCleanupRequestId: null, preStopCleanupReceipt: null,
    cleanupRequestId: null, cleanupReceipt: null, baseline: null,
    notRun: [
      { caseId: 'V07_RESIDUAL_CHILD_PROCESSES', state: 'NOT_RUN', reason: 'No independent descendant-process census was run.' },
      { caseId: 'V07_HOST_RESTART', state: 'NOT_RUN', reason: 'No Owner-host restart was performed.' },
    ] };
  journal.stopWorktree = record; product.save();

  const userOp = async (family, operation, targetId, payload, revision = '0', allowed = ['APPLIED']) =>
    product.operation(family, operation, targetId, payload, String(revision), allowed);
  const graph = async () => {
    let reply = await userOp('K-WORKTREE', 'graph-query', c.worktreeId, {}, '0', ['APPLIED', 'STALE']);
    if (reply.status === 'STALE') reply = await userOp('K-WORKTREE', 'graph-query', c.worktreeId, {}, reply.revision);
    return reply;
  };
  const seatCard = async () => {
    let reply = await userOp('K-SEAT', 'state-card', c.seatId, {}, '0', ['APPLIED', 'STALE']);
    if (reply.status === 'STALE') reply = await userOp('K-SEAT', 'state-card', c.seatId, {}, reply.revision);
    return reply;
  };
  const sessionOp = async (operation, payload = {}) => {
    const reply = await userOp('K-SESSION', operation, record.session.id,
      { generation: record.session.generation, ...payload }, record.session.revision);
    record.session.revision = reply.revision; product.save(); return reply;
  };

  await product.custody(); product.verifyBytes();
  const ui = await product.evaluate('({url:location.href,home:!!document.querySelector(".home-product-entry"),tauri:!!window.__TAURI_INTERNALS__})');
  check(ui.url === product.endpoint.url && ui.home && ui.tauri, 'Actual installed Home/User bridge identity missing');
  const instances = await product.instances();
  const instance = instances.instances.find(row => row.instanceId === config.instanceId);
  check(instance?.state === 'LOGGED_IN', 'V07 requires the original already logged-in test instance');
  const card = await seatCard();
  check(card.result.state === 'IDLE' && card.result.instanceId === config.instanceId,
    'Original User seat must be idle and bound to the configured instance');
  const before = await graph();
  const member = before.result.members?.filter(row => row.worktreeId === c.worktreeId);
  check(before.result.state === 'REGISTERED' && before.result.classification === 'SINGLE' &&
    member?.length === 1 && member[0].domainId === config.domainId &&
    member[0].repositoryId === config.repositoryId && member[0].seatId === c.seatId &&
    member[0].instanceId === config.instanceId,
  'Original F graph must bind exactly one registered test worktree/seat/instance');
  record.graphRevision = before.revision; record.instance = instance;
  record.session.generation = (BigInt(card.result.generation) + 1n).toString(); product.save();

  const reserved = await sessionOp('admission-reserve', { seatId: c.seatId });
  check(reserved.status === 'APPLIED', 'Original H reservation must be applied');
  record.session.reserveRequestId = journal.operations.at(-1).request.requestId;
  const committed = await sessionOp('admission-commit', { seatId: c.seatId });
  check(committed.status === 'APPLIED', 'Original H commit must be applied');
  record.session.commitRequestId = journal.operations.at(-1).request.requestId;
  const opened = await sessionOp('open', { seatId: c.seatId, repositoryId: config.repositoryId,
    worktreeId: c.worktreeId });
  check(opened.status === 'APPLIED' && typeof opened.result.threadId === 'string' && opened.result.threadId,
    'Original H open must bind the exact test worktree without a model send');
  record.session.openRequestId = journal.operations.at(-1).request.requestId;
  record.session.threadId = opened.result.threadId; product.save();

  const denied = await userOp('K-WORKTREE', 'cleanup', c.worktreeId, {}, before.revision, ['DENIED']);
  check(denied.status === 'DENIED' && denied.revision === before.revision,
    'Original User cleanup must be denied while the overlapping H claim is active');
  record.preStopCleanupRequestId = journal.operations.at(-1).request.requestId;
  record.preStopCleanupReceipt = denied; product.save();

  const stopped = await sessionOp('stop', { seatId: c.seatId });
  check(typeof stopped.result.stopFact === 'string' && stopped.result.stopFact,
    'Original H stop must return a real StopFact');
  record.session.stopRequestId = journal.operations.at(-1).request.requestId;
  record.session.stopFactId = stopped.result.stopFact;
  const released = await sessionOp('admission-release', { seatId: c.seatId });
  check(released.status === 'APPLIED', 'Stopped original H claim must be released before cleanup');
  record.session.releaseRequestId = journal.operations.at(-1).request.requestId; product.save();
  const idle = await seatCard();
  check(idle.result.state === 'IDLE' && idle.result.instanceId === config.instanceId,
    'Original H release must leave the same seat idle');
  record.state = 'STOPPED_BASELINE_READBACK_REQUIRED'; product.save();
  await product.custody(); product.verifyBytes();
  const baseline = await c.normalCloseReadbackRestart('stopped');
  const baselineProof = readProof(baseline, 'stopped');
  check(baselineProof.stopOnlyEvidence === true && baselineProof.cleanupApplied === false &&
    baselineProof.worktree.exists === true && baselineProof.worktree.lifecycleState === 'REGISTERED',
  'Stopped baseline must retain the F tree and original H StopFact before cleanup');
  record.baseline = baseline; record.baselineProof = baselineProof; product.save();
  await product.custody(); product.verifyBytes();

  const current = await graph();
  check(current.status === 'APPLIED' && current.result.state === 'REGISTERED' &&
    current.revision === record.graphRevision &&
    JSON.stringify(current.result.members) === JSON.stringify(before.result.members),
  'Same original registered F worktree must remain unchanged after restart and before cleanup');
  const cleaned = await userOp('K-WORKTREE', 'cleanup', c.worktreeId, {}, current.revision);
  check(cleaned.status === 'APPLIED' && cleaned.result.worktreeId === c.worktreeId &&
    cleaned.result.stopFactId === record.session.stopFactId,
  'Original User F cleanup must apply using the actual overlapping H StopFact');
  record.cleanupRequestId = journal.operations.at(-1).request.requestId;
  record.cleanupReceipt = cleaned; record.state = 'CLEANUP_FINAL_READBACK_REQUIRED'; product.save();
  await product.custody(); product.verifyBytes();
  const final = await c.normalCloseReadbackRestart('final');
  const finalProof = readProof(final, 'final');
  check(finalProof.directCaseEvidence === true && finalProof.cleanupApplied === true &&
    finalProof.worktree.exists === false && finalProof.worktree.lifecycleState === 'CLEANED' &&
    finalProof.worktree.stopFactId === record.session.stopFactId &&
    JSON.stringify(finalProof.rootIdentity) === JSON.stringify(baselineProof.rootIdentity) &&
    JSON.stringify(finalProof.candidateIdentity) === JSON.stringify(baselineProof.candidateIdentity) &&
    JSON.stringify(finalProof.candidateInstalledSha256) === JSON.stringify(baselineProof.candidateInstalledSha256),
  'Final F cleanup readback must match the original stop/candidate/root facts');
  record.final = final; record.state = 'FLOW_COMPLETE_REVIEW_REQUIRED'; product.save();
  await product.custody(); product.verifyBytes();
  return { state: record.state, acceptance: false, notRun: record.notRun };

  function readProof(reference, phase) {
    check(reference && path.basename(reference.file) === reference.file && /^[a-f0-9]{64}$/.test(reference.sha256),
      `Normal-close immutable ${phase} readback reference required`);
    const file = path.resolve(config.evidenceDirectory, reference.file);
    check(path.dirname(file) === path.resolve(config.evidenceDirectory) && sha256(file) === reference.sha256,
      `Original ${phase} readback file/hash differs from the callback reference`);
    const proof = JSON.parse(fs.readFileSync(file, 'utf8').replace(/^\uFEFF/, ''));
    check(proof.schema === 'gogoke.37.private-m2-stop-worktree-readback.v1' && proof.phase === phase &&
      proof.caseId === journal.caseId && proof.sourceCommit === config.sourceCommit &&
      proof.domainId === config.domainId && proof.readerSha256 === record.readerSha256 &&
      proof.stateRoot === record.stateRoot && proof.evidenceDirectory === record.evidenceDirectory &&
      proof.worktree?.worktreeId === record.worktreeId && proof.worktree.seatId === record.seatId &&
      proof.worktree.instanceId === record.instanceId && proof.worktree.repositoryId === record.repositoryId &&
      proof.stopFactId === record.session.stopFactId &&
      proof.measurementPreservedDatabaseBytes === true && proof.acceptance === false &&
      proof.normalClose?.exitCode === 0 && proof.normalClose.forceKill === false &&
      proof.launch?.pid === proof.normalClose.pid && proof.candidateIdentity?.sourceCommit === config.sourceCommit &&
      proof.candidateIdentity.version === config.version && proof.candidateIdentity.setId &&
      proof.candidateIdentity.generationId &&
      JSON.stringify(proof.candidateInstalledSha256) === JSON.stringify(config.installedSha256),
    `Original ${phase} readback is not bound to the closed installed candidate`);
    return proof;
  }
}
