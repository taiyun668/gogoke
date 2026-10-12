// Fixed Codex evidence input for the existing V08 core and Host busy-to-idle case.
// Follows the separately reviewed fixed OpenCode source reader; no provider-wide fallback.
import fs from 'node:fs';
import path from 'node:path';
import { createHash } from 'node:crypto';

const instanceId = 'codexTestM2Ready';
const version = '0.160.0';
const digest = 'fdda5fa3cf3fb3d000b876720742857676293e4315e4b045fae6f8bd7e866d1d';
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const sha = value => typeof value === 'string' && /^[a-f0-9]{64}$/.test(value);
const atom = value => typeof value === 'string' && /^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/.test(value);
const home = value => typeof value === 'string' && /^volume:[a-f0-9]{16}\/file:[a-f0-9]{32}$/.test(value);

function sourceQualifications(config, raw, fixedCase, label = 'Original managed Codex') {
  const instance = raw?.instance, source = raw?.programSource, copy = raw?.managedCopy;
  const qualifications = {
    fixedCase,
    originalClosedRead: raw?.databaseBytesPreserved === true && sha(raw.databaseSha256) &&
      raw.authContentOpened === false && raw.acceptance === false,
    fixedInstance: instance?.instance_id === instanceId && instance.driver_id === 'codex' &&
      instance.version === version && instance.program_digest === `sha256:${digest}` &&
      instance.install_state === 'UNKNOWN' && Number.isSafeInteger(instance.revision) && instance.revision > 0,
    originalLogin: instance?.login_state === 'LOGGED_IN',
    fixedHome: home(config.managedSourceRead?.homeIdentity) &&
      instance?.home_identity === config.managedSourceRead.homeIdentity &&
      source?.home_identity === config.managedSourceRead.homeIdentity,
    managedSource: raw?.programSourceKind === 'MANAGED' && source?.source === 'MANAGED' &&
      source.instance_id === instanceId && source.version === version &&
      source.program_digest === `sha256:${digest}` && atom(source.registration_request_id) &&
      Number.isSafeInteger(source.revision) && source.revision > 0,
    readyCopy: copy?.driver_id === 'codex' && copy.state === 'READY' &&
      copy.version === version && copy.image_sha256 === digest,
    sameStage: atom(source?.stage_name) && source.stage_name === copy?.stage_name,
  };
  const failed = Object.entries(qualifications).filter(([, value]) => value !== true).map(([key]) => key);
  if (failed.length) throw Error(`${label} qualification differs: ${failed.join(', ')}`);
  return qualifications;
}

export function managedCodexQualifications(config, raw) {
  const rules = config.rules;
  const core = rules?.selection === undefined && rules?.host === undefined;
  // The original driver validates both sealed core13 files, the same source seats,
  // their real releases and its final direct readback before this source reader.
  const recipients = [rules?.submitter, rules?.reviewer,
    rules?.host?.destination, rules?.host?.alternateDestination];
  const prior = rules?.priorCore13;
  const host = rules?.selection === 'HOST_BUSY_TO_IDLE_ONLY' &&
    rules.host?.busyToIdleOnly === true && rules.foreignProject === undefined &&
    rules.host.lifecycleOwnership === 'EXCLUSIVE_V08_HOST_RECIPIENTS' &&
    recipients.every(row => row?.instanceId === instanceId && atom(row.seatId) && atom(row.worktreeId)) &&
    new Set(recipients.map(row => row.seatId)).size === 4 &&
    new Set(recipients.map(row => row.worktreeId)).size === 4 &&
    typeof prior?.journal === 'string' && path.isAbsolute(prior.journal) && sha(prior.journalSha256) &&
    typeof prior?.finalReadback === 'string' && path.isAbsolute(prior.finalReadback) && sha(prior.finalSha256);
  return sourceQualifications(config, raw,
    config.instanceId === instanceId && config.repositoryId === 'gogokeSeatTestbed' &&
      (core || host) &&
      rules?.lifecycleOwnership === 'EXCLUSIVE_V08_SUBMITTER_AND_REVIEWER' &&
      rules.policyOwnership === 'EXCLUSIVE_V08_POLICY_DOMAIN' &&
      rules.userPermissionTier === 'NETWORKED_WRITE' && rules.userMaxConcurrent === 4 &&
      rules.projectParallelCap === 4 && rules.submitter?.instanceId === instanceId &&
      rules.reviewer?.instanceId === instanceId);
}

export function managedCodexV06Qualifications(config, raw) {
  const f = config?.v06Remaining;
  const identities = [f?.parentSeatId, f?.childSeatId, f?.parentTreeId, f?.childTreeId];
  const fixedCase = config?.rules === undefined &&
    config?.repositoryId === 'gogokeSeatTestbed' &&
    config?.instanceId === instanceId && atom(config?.domainId) && config?.domainId === f?.domainId &&
    f?.lifecycleOwnership === 'EXCLUSIVE_M2_V06_REMAINING' &&
    f?.instanceId === instanceId &&
    [f?.templateId, ...identities].every(atom) && new Set(identities).size === 4 &&
    f?.model === 'gpt-6.1-sol' && f?.effort === 'high' &&
    f?.permissionTier === 'NETWORKED_WRITE' && f?.maxConcurrent === 1 &&
    f?.takeover?.questionId === 'm2PrivateScope' &&
    f?.takeover?.prompt === 'Which scope is authorized for this M2 product test?' &&
    f?.takeover?.option === 'Private testbed only' &&
    config?.cliVersion === version && config?.cliSha256 === digest;
  return sourceQualifications(config, raw, fixedCase, 'Original V06 managed Codex');
}

export function managedCodexM2Qualifications(config, raw) {
  const selected = [config?.instanceId, config?.childInstanceId,
    config?.sideChat?.sourceInstanceId, config?.sideChat?.sideInstanceId,
    config?.rules?.submitter?.instanceId, config?.rules?.reviewer?.instanceId,
    config?.rules?.host?.destination?.instanceId,
    config?.rules?.host?.alternateDestination?.instanceId].filter(value => value !== undefined);
  return sourceQualifications(config, raw,
    config?.repositoryId === 'gogokeSeatTestbed' &&
      config?.v06Remaining === undefined &&
      config?.retainedPrelaunchFailure === undefined && selected.length >= 2 &&
      selected.every(value => value === instanceId), 'Original M2 managed Codex');
}

export function readManagedCodexSource(config) {
  if (config.managedSourceRead === undefined) return undefined;
  const proof = config.managedSourceRead;
  if (!proof || typeof proof.path !== 'string' || !path.isAbsolute(proof.path) || !sha(proof.sha256)) {
    throw Error('Original managed Codex source requires an absolute path and SHA-256 seal');
  }
  const bytes = fs.readFileSync(proof.path);
  if (hash(bytes) !== proof.sha256) throw Error('Original managed Codex source seal differs');
  const raw = JSON.parse(bytes.toString('utf8').replace(/^\uFEFF/, ''));
  const qualifications = config.v06Remaining !== undefined
    ? managedCodexV06Qualifications(config, raw)
    : config.childInstanceId !== undefined
    ? managedCodexM2Qualifications(config, raw)
    : managedCodexQualifications(config, raw);
  const database = path.join(config.stateRoot, 'state.sqlite');
  const closedHash = () => {
    for (const suffix of ['-wal', '-shm', '-journal']) {
      const sidecar = database + suffix;
      if (fs.existsSync(sidecar) && fs.statSync(sidecar).size !== 0) {
        throw Error('Original managed Codex source requires a closed checkpointed database');
      }
    }
    return hash(fs.readFileSync(database));
  };
  if (closedHash() !== raw.databaseSha256 || closedHash() !== raw.databaseSha256) {
    throw Error('Original managed Codex source does not bind the current closed database bytes');
  }
  return { path: proof.path, sha256: proof.sha256, homeIdentity: proof.homeIdentity,
    databaseSha256: raw.databaseSha256, databaseBytesPreserved: true, qualifications,
    fact: raw.instance, programSource: raw.programSource, managedCopy: raw.managedCopy };
}

export function observeManagedCodexInstance(proof, page, selectedInstanceId) {
  const matches = page.instances.filter(row => row.instanceId === selectedInstanceId && row.driverId === 'codex');
  if (selectedInstanceId !== instanceId || proof.fact.instance_id !== selectedInstanceId ||
      matches.length !== 1 || matches[0].version !== version ||
      typeof matches[0].state !== 'string' || !matches[0].state) {
    throw Error('Managed Codex requires its exact original instance in the actual UI read');
  }
  return { instanceId: selectedInstanceId, driverId: 'codex', version: matches[0].version,
    uiState: matches[0].state, nativeLoginState: proof.fact.login_state,
    basis: 'ORIGINAL_MANAGED_SOURCE_WITH_REAL_H_ADMISSION_REQUIRED' };
}

export function checkManagedCodexSeat(card) {
  // This proves the current E request settings only. Existing F/H determines model availability.
  if (card.status !== 'APPLIED' || card.result?.state !== 'IDLE' || card.result.layer !== 'USER' ||
      card.result.instanceId !== instanceId || card.result.settings?.model !== 'gpt-6.1-sol' ||
      card.result.settings.effort !== 'high' || card.result.settings.permissionTier !== 'NETWORKED_WRITE') {
    throw Error('Managed Codex E request settings differ from the fixed model, effort or permission tier');
  }
  return { state: 'E_REQUEST_SETTING_MATCH', model: card.result.settings.model,
    effort: card.result.settings.effort, permissionTier: card.result.settings.permissionTier,
    modelAvailability: 'NOT_REPORTED' };
}

export function checkManagedCodexCapability(session, receipt) {
  // Observe the original probe receipt. Never send a substitute or second probe.
  if (session?.instanceId !== instanceId || receipt.status !== 'APPLIED' ||
      receipt.result?.driverId !== 'codex' || receipt.result.version !== version ||
      receipt.result.binaryDigest !== `sha256:${digest}` ||
      typeof session.threadId !== 'string' || !session.threadId || receipt.result.threadId !== session.threadId) {
    throw Error('Original managed Codex H capability differs from the fixed pin or thread');
  }
}
