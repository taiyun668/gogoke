// Compose existing attachment, normal-close and protection instruments only.
// Close failed status-only observations, including original late-ready launches; never settle H/HOME.
import fs from 'node:fs';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { isDeepStrictEqual } from 'node:util';
import { pathToFileURL, fileURLToPath } from 'node:url';
const hash = p => createHash('sha256').update(fs.readFileSync(p)).digest('hex');
const read = p => JSON.parse(fs.readFileSync(p, 'utf8').replace(/^\uFEFF/, ''));
const need = (ok, why) => { if (!ok) throw Error(why); };
const c = read(process.argv[2]);
need(c.executionEnabled === true && c.acceptance === false && c.actor.path === fileURLToPath(import.meta.url), 'Unbound retained observer');
for (const r of [c.actor, c.config, c.failure, c.manifest, c.startup, ...Object.values(c.modules), ...c.refs])
  need(path.isAbsolute(r.path) && /^[a-f0-9]{64}$/.test(r.sha256) && hash(r.path) === r.sha256, 'Pinned original/source differs');
const originalConfig = read(c.config.path);
const lateReady = c.originalReadinessMode === 'LATE_VALID_RECEIPT_ORIGINAL_FAILURE';
const config = { ...originalConfig, python: originalConfig.runtimePins.python.path };
need(c.refs.some(r => r.path === config.python && r.sha256 === config.runtimePins.python.sha256), 'Original Python runtime pin missing');
const failed = read(c.failure.path), manifest = read(c.manifest.path), startup = read(c.startup.path).startup;
need(failed.schema === 'gogoke.37.common-managed-cli-status.v1' && failed.state === 'FAIL_OR_UNKNOWN_ORIGINAL_PRESERVED' &&
  failed.failedProductHeld === true && failed.homeTeardown?.state === 'NO_HOME_CREATE_DISPATCHED' && failed.homeTeardown.complete === true &&
  ['operations','sessions','managedCliActions','statusReads','closes'].every(k => Array.isArray(failed[k]) && failed[k].length === 0) &&
  (failed.launches.length === 1 || (lateReady && failed.launches.length === 0 &&
    failed.readinessTiming?.readyAcceptedBeforeDeadline === false &&
    failed.startupReadiness?.readyReceiptPresent === false && failed.startupReadiness?.diagnosticConnected === false)) &&
  failed.sourceCommit === config.sourceCommit && failed.stateRoot === config.stateRoot &&
  failed.candidateVersion === config.version && JSON.stringify(failed.candidateInstalledSha256) === JSON.stringify(config.installedSha256) &&
  manifest.caseId === failed.caseId && ['attempts','startupAttempts','authorizedRepairs'].every(k => Array.isArray(manifest[k]) && manifest[k].length === 0) &&
  startup.state === 'CLOSED_PREDECESSOR' && !fs.existsSync(c.result), 'Original is not a zero-action held status observation');
const { ActualProduct } = await import(pathToFileURL(c.modules.product.path));
const { closeAttachedCausally, requireRecordedNormalClose, causalCloseSources } = await import(pathToFileURL(c.modules.causal.path));
const protection = await import(pathToFileURL(c.modules.protection.path));
const journal = { schema: 'gogoke.37.retained-no-action-close.v1', state: 'RUNNING', acceptance: false,
  caseId: c.caseId, sourceCommit: config.sourceCommit, candidateVersion: config.version,
  originalFailure: c.failure, originalCaseRemainsFailed: true, noStopFactInvented: true,
  noHOrHomeMutation: true, operations: [], sessions: [], launches: [], closes: [], snapshots: {}, sourcePins: c };
const product = new ActualProduct({ ...config, evidenceDirectory: c.evidenceDirectory, result: c.result, testerArmy: false }, journal);
product.save();
try {
  await product.attachRetained({ endpoint: c.retainedEndpoint ?? failed.currentEndpoint, readySha256: c.readySha256,
    provenance: { failure: c.failure }, sourcePins: c.modules });
  if (lateReady) {
    const lifecycle = await import(pathToFileURL(c.modules.lifecycle.path));
    journal.readinessTiming = structuredClone(failed.readinessTiming);
    lifecycle.recordLateReadyAfterFailure(product);
    need(journal.readinessLateObservations?.at(-1)?.state === 'LATE_VALID_RECEIPT_ORIGINAL_FAILURE',
      'Original late-ready receipt is not qualified');
  }
  const frame = JSON.stringify({ schema: 'gogoke.37.owner-configuration.v1', command: 'secretary-configuration-read' });
  const rawReply = await product.evaluate(`window.__TAURI_INTERNALS__.invoke('gogoke_design37_user_operation',{frame:${JSON.stringify(frame)}})`);
  const reply = JSON.parse(rawReply), h = reply.conversation;
  journal.originalNativeRead = { rawFrame: frame, rawReply }; product.save();
  need(reply.state === 'DESIGNATED' && reply.seatId === startup.seatId && reply.incarnation === startup.seatIncarnation && reply.instanceId === startup.instanceId &&
    h?.state === 'FOUND' && h.sessionId === startup.sessionId && String(h.generation) === startup.oldGeneration && String(h.revision) === startup.oldRevision &&
    h.claimState === 'STOPPED' && h.runtimeAvailable === false && h.stoppedFact === true &&
    (lateReady ? reply.startupReason === 'Original Secretary bootstrap failed: Invalid("failed resume typed OBSERVED response absent")' :
      failed.startupHomeReads.some(row => row.reply?.startupReason === reply.startupReason && row.reply?.conversation?.sessionId === h.sessionId)),
    'Original native stopped predecessor changed; retain without close');
  journal.preCloseBasis = 'ORIGINAL_NATIVE_STOPPED_FACT_AND_ZERO_ACTION_CREATION_RECORDS'; product.save();
  await closeAttachedCausally(product);
  requireRecordedNormalClose(journal, product.endpoint, causalCloseSources(), config);
  // Repeat the original baseline instrument without changing its input or claiming
  // a successful original teardown. Equal inventory is only a raw no-change fact.
  const baselineAfter = path.join(c.evidenceDirectory, 'original-home-inventory-after-close.json');
  const observed = spawnSync(config.python, ['-E', '-B', c.homeReader.path, '--input', c.baselineInput.path, '--output', baselineAfter], { encoding: 'utf8', windowsHide: true });
  journal.homeInventoryReader = { exitCode: observed.status, stdout: observed.stdout, stderr: observed.stderr, output: baselineAfter }; product.save();
  need(observed.status === 0, 'Original baseline inventory reader failed');
  const before = read(c.baselineBefore.path), after = read(baselineAfter);
  need(before.caseId === failed.caseId && after.caseId === failed.caseId && before.state === 'BASELINE_MEASURED' && after.state === 'BASELINE_MEASURED' &&
    isDeepStrictEqual(before.homes, after.homes), 'Original HOME inventory changed; no teardown pass');
  journal.originalHomeInventoryUnchanged = true; product.save();
  await protection.captureProtection(config.observers, null,
    name => path.join(c.evidenceDirectory, `${name}-after-retained-close.json`),
    (name, ref) => { journal.snapshots[`${name}-after`] = ref; product.save(); });
  journal.protectionComparison = protection.compareProtection(config.observers, (name, phase) =>
    protection.readProtectionSnapshot(phase === 'before' ? config.evidenceDirectory : c.evidenceDirectory,
      phase === 'before' ? failed.snapshots[`${name}-before`] : journal.snapshots[`${name}-after`]));
  need(journal.protectionComparison.allEqual === true && journal.protectionComparison.measuredFieldCount === 13, 'Original protected fields changed');
  journal.state = 'NORMAL_CLOSE_OBSERVED_ORIGINAL_STATUS_CASE_STILL_FAILED'; product.save();
} catch (error) {
  journal.state = 'FAIL_PRESERVE_NO_REPLAY'; journal.error = String(error?.stack ?? error); product.save(); throw error;
} finally { product.socket?.close(); }
