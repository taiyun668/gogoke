// Read-only evidence input for the one fixed managed OpenCode E2E case.
// This is not a login or H admission result. The real H pin is still required.
import fs from 'node:fs';
import path from 'node:path';
import { createHash } from 'node:crypto';

const digest = 'cf664aa1da32b788f9b2699b84a9bb9be30b7e025693b90f9b85829d5fe4e252';
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const sha = value => typeof value === 'string' && /^[a-f0-9]{64}$/.test(value);
const atom = value => typeof value === 'string' && /^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/.test(value);
const home = value => typeof value === 'string' && /^volume:[a-f0-9]{16}\/file:[a-f0-9]{32}$/.test(value);

export function managedSourceQualifications(row, raw) {
  const instance = raw?.instance, source = raw?.programSource, copy = raw?.managedCopy;
  const qualifications = {
    fixedCase: row.driverId === 'opencode' && row.instanceId === 'opencodeTestM2' &&
      row.version === '1.18.32' && row.sha256 === digest && row.model === 'xai/grok-4.7' &&
      row.caseMode === undefined && row.originalRegistryRead === undefined,
    originalClosedRead: raw?.databaseBytesPreserved === true && sha(raw.databaseSha256) &&
      raw.authContentOpened === false && raw.acceptance === false,
    fixedInstance: instance?.instance_id === row.instanceId && instance.driver_id === row.driverId &&
      instance.version === row.version && instance.program_digest === `sha256:${row.sha256}` &&
      instance.install_state === 'UNKNOWN' && Number.isSafeInteger(instance.revision) && instance.revision > 0,
    originalLogin: instance?.login_state === 'LOGGED_IN',
    fixedHome: home(row.managedSourceRead?.homeIdentity) &&
      instance?.home_identity === row.managedSourceRead.homeIdentity &&
      source?.home_identity === row.managedSourceRead.homeIdentity,
    managedSource: raw?.programSourceKind === 'MANAGED' && source?.source === 'MANAGED' &&
      source.instance_id === row.instanceId && source.version === row.version &&
      source.program_digest === `sha256:${row.sha256}` && atom(source.registration_request_id) &&
      Number.isSafeInteger(source.revision) && source.revision > 0,
    readyCopy: copy?.driver_id === row.driverId && copy.state === 'READY' &&
      copy.version === row.version && copy.image_sha256 === row.sha256,
    sameStage: atom(source?.stage_name) && source.stage_name === copy?.stage_name,
  };
  const failed = Object.entries(qualifications).filter(([, value]) => value !== true).map(([key]) => key);
  if (failed.length) throw Error(`Original managed OpenCode qualification differs: ${failed.join(', ')}`);
  return qualifications;
}

export function readManagedSource(row, stateRoot) {
  if (row.managedSourceRead === undefined) return undefined;
  const proof = row.managedSourceRead;
  if (!proof || typeof proof.path !== 'string' || !path.isAbsolute(proof.path) || !sha(proof.sha256)) {
    throw Error('Original managed OpenCode source requires an absolute path and SHA-256 seal');
  }
  const bytes = fs.readFileSync(proof.path);
  if (hash(bytes) !== proof.sha256) throw Error('Original managed OpenCode source seal differs');
  const raw = JSON.parse(bytes.toString('utf8').replace(/^\uFEFF/, ''));
  const qualifications = managedSourceQualifications(row, raw);
  const database = path.join(stateRoot, 'state.sqlite');
  const closedHash = () => {
    for (const suffix of ['-wal', '-shm', '-journal']) {
      const sidecar = database + suffix;
      if (fs.existsSync(sidecar) && fs.statSync(sidecar).size !== 0) {
        throw Error('Original managed OpenCode source requires a closed checkpointed database');
      }
    }
    return hash(fs.readFileSync(database));
  };
  if (closedHash() !== raw.databaseSha256 || closedHash() !== raw.databaseSha256) {
    throw Error('Original managed OpenCode source does not bind the current closed database bytes');
  }
  return { path: proof.path, sha256: proof.sha256, homeIdentity: proof.homeIdentity,
    databaseSha256: raw.databaseSha256, databaseBytesPreserved: true, qualifications,
    fact: raw.instance, programSource: raw.programSource, managedCopy: raw.managedCopy };
}
