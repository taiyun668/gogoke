// Evidence boundaries only; no product, CLI, runtime substitute, model or actual state DB.
// Set GOGOKE_E2E_TEST_TEMP_ROOT to an explicit private scratch directory.
import assert from 'node:assert/strict';
import { test } from 'node:test';
import fs from 'node:fs';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { managedCodexQualifications, readManagedCodexSource, observeManagedCodexInstance,
  checkManagedCodexSeat, checkManagedCodexCapability } from './m2-extra-managed-source.mjs';

const digest = 'fdda5fa3cf3fb3d000b876720742857676293e4315e4b045fae6f8bd7e866d1d';
const home = 'volume:0000000000000001/file:00000000000000000000000000000001';
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const databaseBytes = Buffer.from('ISOLATED EVIDENCE LOADER TEST; NOT A PRODUCT DATABASE');
function fixture() {
  return {
    config: { instanceId: 'codexTestM2Ready', repositoryId: 'gogokeSeatTestbed',
      managedSourceRead: { homeIdentity: home }, rules: {
        lifecycleOwnership: 'EXCLUSIVE_V08_SUBMITTER_AND_REVIEWER',
        policyOwnership: 'EXCLUSIVE_V08_POLICY_DOMAIN', userPermissionTier: 'NETWORKED_WRITE',
        userMaxConcurrent: 4, projectParallelCap: 4,
        submitter: { instanceId: 'codexTestM2Ready', seatId: 'submitter', worktreeId: 'submitterF' },
        reviewer: { instanceId: 'codexTestM2Ready', seatId: 'reviewer', worktreeId: 'reviewerF' },
      } },
    raw: { instance: { instance_id: 'codexTestM2Ready', driver_id: 'codex', version: '0.160.0',
      program_digest: `sha256:${digest}`, install_state: 'UNKNOWN', login_state: 'LOGGED_IN',
      home_identity: home, revision: 3 }, programSourceKind: 'MANAGED',
      programSource: { instance_id: 'codexTestM2Ready', source: 'MANAGED', stage_name: 'codex-test-stage',
        program_digest: `sha256:${digest}`, version: '0.160.0', home_identity: home,
        registration_request_id: 'original-test-registration', revision: 1 },
      managedCopy: { driver_id: 'codex', state: 'READY', stage_name: 'codex-test-stage',
        version: '0.160.0', image_sha256: digest }, databaseBytesPreserved: true,
      databaseSha256: hash(databaseBytes), authContentOpened: false, acceptance: false },
  };
}

test('fixed source qualifies without changing UNKNOWN install or native login', () => {
  const { config, raw } = fixture();
  assert.ok(Object.values(managedCodexQualifications(config, raw)).every(value => value === true));
  assert.equal(raw.instance.install_state, 'UNKNOWN');
  assert.equal(raw.instance.login_state, 'LOGGED_IN');
});
test('no proof returns no alternative source fact', () => {
  assert.equal(readManagedCodexSource({ managedSourceRead: undefined }), undefined);
});
for (const [name, change] of [
  ['another instance', ({ config }) => { config.instanceId = 'codexOther'; }],
  ['another source seat instance', ({ config }) => { config.rules.reviewer.instanceId = 'codexOther'; }],
  ['different case selection', ({ config }) => { config.rules.selection = 'HOST_BUSY_TO_IDLE_ONLY'; }],
  ['Host case', ({ config }) => { config.rules.host = {}; }],
  ['different capacity', ({ config }) => { config.rules.userMaxConcurrent = 5; }],
  ['other driver', ({ raw }) => { raw.instance.driver_id = 'opencode'; }],
  ['other version', ({ raw }) => { raw.instance.version = '0.161.0'; }],
  ['other digest', ({ raw }) => { raw.instance.program_digest = `sha256:${'0'.repeat(64)}`; }],
  ['not logged in', ({ raw }) => { raw.instance.login_state = 'LOGGED_OUT'; }],
  ['invented install fact', ({ raw }) => { raw.instance.install_state = 'INSTALLED'; }],
  ['missing home pin', ({ config }) => { delete config.managedSourceRead.homeIdentity; }],
  ['instance home mismatch', ({ raw }) => { raw.instance.home_identity = home.replace(/1$/, '2'); }],
  ['source home mismatch', ({ raw }) => { raw.programSource.home_identity = home.replace(/1$/, '2'); }],
  ['both homes changed', ({ raw }) => {
    raw.instance.home_identity = raw.programSource.home_identity = home.replace(/1$/, '2');
  }],
  ['source kind mismatch', ({ raw }) => { raw.programSourceKind = 'EXTERNAL'; }],
  ['source not MANAGED', ({ raw }) => { raw.programSource.source = 'EXTERNAL'; }],
  ['source digest mismatch', ({ raw }) => { raw.programSource.program_digest = `sha256:${'0'.repeat(64)}`; }],
  ['copy not READY despite matching pin', ({ raw }) => { raw.managedCopy.state = 'FAILED'; }],
  ['copy digest mismatch', ({ raw }) => { raw.managedCopy.image_sha256 = '0'.repeat(64); }],
  ['stage mismatch', ({ raw }) => { raw.managedCopy.stage_name = 'other'; }],
  ['unpreserved DB', ({ raw }) => { raw.databaseBytesPreserved = false; }],
  ['credential content read', ({ raw }) => { raw.authContentOpened = true; }],
  ['acceptance claim', ({ raw }) => { raw.acceptance = true; }],
]) {
  test(`reject ${name}`, () => {
    const values = fixture(); change(values);
    assert.throws(() => managedCodexQualifications(values.config, values.raw), /qualification differs/);
  });
}

test('records actual UI ERROR without modifying the instance response', () => {
  const { raw } = fixture();
  const page = { instances: [{ instanceId: 'codexTestM2Ready', driverId: 'codex', version: '0.160.0', state: 'ERROR' }] };
  const original = structuredClone(page);
  const observed = observeManagedCodexInstance({ fact: raw.instance }, page, 'codexTestM2Ready');
  assert.equal(observed.uiState, 'ERROR');
  assert.equal(observed.nativeLoginState, 'LOGGED_IN');
  assert.deepEqual(page, original);
  assert.throws(() => observeManagedCodexInstance({ fact: raw.instance }, { instances: [] }, 'codexTestM2Ready'));
  page.instances[0].version = '0.161.0';
  assert.throws(() => observeManagedCodexInstance({ fact: raw.instance }, page, 'codexTestM2Ready'));
});

test('E setting match does not claim model availability', () => {
  const card = { status: 'APPLIED', result: { state: 'IDLE', layer: 'USER', instanceId: 'codexTestM2Ready',
    settings: { model: 'gpt-6.1-sol', effort: 'high', permissionTier: 'NETWORKED_WRITE' } } };
  const original = structuredClone(card);
  assert.deepEqual(checkManagedCodexSeat(card), { state: 'E_REQUEST_SETTING_MATCH', model: 'gpt-6.1-sol',
    effort: 'high', permissionTier: 'NETWORKED_WRITE', modelAvailability: 'NOT_REPORTED' });
  assert.deepEqual(card, original);
  for (const [key, value] of [['model', 'other-model'], ['effort', 'low'], ['permissionTier', 'READ_ONLY']]) {
    const changed = structuredClone(card); changed.result.settings[key] = value;
    assert.throws(() => checkManagedCodexSeat(changed), /E request settings differ/);
  }
});

test('original H capability must match fixed program and original thread', () => {
  const session = { instanceId: 'codexTestM2Ready', threadId: 'original-thread' };
  const receipt = { status: 'APPLIED', result: { driverId: 'codex', version: '0.160.0',
    binaryDigest: `sha256:${digest}`, threadId: session.threadId } };
  const original = structuredClone(receipt);
  checkManagedCodexCapability(session, receipt);
  assert.deepEqual(receipt, original);
  assert.throws(() => checkManagedCodexCapability(undefined, receipt), /H capability differs/);
  for (const [key, value] of [['driverId', 'opencode'], ['version', '0.161.0'],
    ['binaryDigest', `sha256:${'0'.repeat(64)}`], ['threadId', 'other-thread']]) {
    const changed = structuredClone(receipt); changed.result[key] = value;
    assert.throws(() => checkManagedCodexCapability(session, changed), /H capability differs/);
  }
});

test('sealed loader rejects stale DB, changed seal and uncheckpointed sidecars', t => {
  const root = process.env.GOGOKE_E2E_TEST_TEMP_ROOT;
  assert.ok(root && path.isAbsolute(root), 'Explicit private scratch directory required');
  const directory = fs.mkdtempSync(path.join(root, 'codex-managed-source-boundary-'));
  t.after(() => {
    assert.equal(path.dirname(path.resolve(directory)), path.resolve(root));
    fs.rmSync(directory, { recursive: true, force: true });
  });
  const { config, raw } = fixture();
  const database = path.join(directory, 'state.sqlite');
  const source = path.join(directory, 'source.json');
  const bytes = Buffer.from(JSON.stringify(raw));
  fs.writeFileSync(database, databaseBytes, { flag: 'wx' });
  fs.writeFileSync(source, bytes, { flag: 'wx' });
  config.stateRoot = directory;
  Object.assign(config.managedSourceRead, { path: source, sha256: hash(bytes) });
  assert.equal(readManagedCodexSource(config).fact.login_state, 'LOGGED_IN');
  fs.appendFileSync(source, '\n');
  assert.throws(() => readManagedCodexSource(config), /seal differs/);
  fs.writeFileSync(source, bytes);
  fs.appendFileSync(database, 'changed');
  assert.throws(() => readManagedCodexSource(config), /current closed database bytes/);
  fs.writeFileSync(database, databaseBytes);
  for (const suffix of ['-wal', '-shm', '-journal']) {
    fs.writeFileSync(database + suffix, 'uncheckpointed');
    assert.throws(() => readManagedCodexSource(config), /closed checkpointed database/);
    fs.unlinkSync(database + suffix);
  }
  config.managedSourceRead.path = 'relative.json';
  assert.throws(() => readManagedCodexSource(config), /absolute path/);
});
