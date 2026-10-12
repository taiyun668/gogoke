// Evidence boundaries only; no product, CLI, runtime substitute, model or actual state DB.
// Set GOGOKE_E2E_TEST_TEMP_ROOT to an explicit private scratch directory.
import assert from 'node:assert/strict';
import { test } from 'node:test';
import fs from 'node:fs';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { managedCodexQualifications, managedCodexV06Qualifications,
  readManagedCodexSource, observeManagedCodexInstance, checkManagedCodexSeat,
  checkManagedCodexCapability } from './m2-extra-managed-source.mjs';

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

function v06Fixture() {
  const values = fixture();
  const domainId = 'm2V06Domain';
  delete values.config.rules;
  values.config.domainId = domainId;
  values.config.cliVersion = '0.160.0';
  values.config.cliSha256 = digest;
  values.config.v06Remaining = {
    lifecycleOwnership: 'EXCLUSIVE_M2_V06_REMAINING', domainId,
    instanceId: 'codexTestM2Ready', templateId: 'm2V06Template',
    parentSeatId: 'm2V06Parent', childSeatId: 'm2V06Child',
    parentTreeId: 'm2V06ParentF', childTreeId: 'm2V06ChildF',
    model: 'gpt-6.1-sol', effort: 'high', permissionTier: 'NETWORKED_WRITE',
    maxConcurrent: 1,
    takeover: { questionId: 'm2PrivateScope',
      prompt: 'Which scope is authorized for this M2 product test?', option: 'Private testbed only' },
  };
  return values;
}

test('V06 managed predicate requires its exact fixture without V08 rules or cap', () => {
  const { config, raw } = v06Fixture();
  const original = structuredClone({ config, raw });
  assert.ok(Object.values(managedCodexV06Qualifications(config, raw)).every(value => value === true));
  assert.deepEqual({ config, raw }, original);
});
for (const [name, change] of [
  ['V08 rules present', ({ config }) => { config.rules = { userMaxConcurrent: 4, projectParallelCap: 4 }; }],
  ['wrong V06 ownership', ({ config }) => { config.v06Remaining.lifecycleOwnership = 'EXCLUSIVE_V08_SUBMITTER_AND_REVIEWER'; }],
  ['domain mismatch', ({ config }) => { config.v06Remaining.domainId = 'otherDomain'; }],
  ['instance mismatch', ({ config }) => { config.v06Remaining.instanceId = 'codexOther'; }],
  ['CLI version mismatch', ({ config }) => { config.cliVersion = '0.161.0'; }],
  ['CLI digest mismatch', ({ config }) => { config.cliSha256 = '0'.repeat(64); }],
  ['template model mismatch', ({ config }) => { config.v06Remaining.model = 'other-model'; }],
  ['capacity four', ({ config }) => { config.v06Remaining.maxConcurrent = 4; }],
  ['duplicate fresh identity', ({ config }) => { config.v06Remaining.childTreeId = config.v06Remaining.parentTreeId; }],
  ['takeover question mismatch', ({ config }) => { config.v06Remaining.takeover.questionId = 'otherQuestion'; }],
  ['not logged in', ({ raw }) => { raw.instance.login_state = 'LOGGED_OUT'; }],
  ['invented install state', ({ raw }) => { raw.instance.install_state = 'INSTALLED'; }],
  ['source not managed', ({ raw }) => { raw.programSource.source = 'EXTERNAL'; }],
  ['copy not ready', ({ raw }) => { raw.managedCopy.state = 'UNKNOWN'; }],
  ['source digest mismatch', ({ raw }) => { raw.programSource.program_digest = `sha256:${'0'.repeat(64)}`; }],
  ['home mismatch', ({ raw }) => { raw.programSource.home_identity = home.replace(/1$/, '2'); }],
  ['unpreserved DB', ({ raw }) => { raw.databaseBytesPreserved = false; }],
]) {
  test(`V06 rejects ${name}`, () => {
    const values = v06Fixture(); change(values);
    assert.throws(() => managedCodexV06Qualifications(values.config, values.raw), /qualification differs/);
  });
}

function hostFixture() {
  const values = fixture();
  values.config.rules.selection = 'HOST_BUSY_TO_IDLE_ONLY';
  values.config.rules.host = { lifecycleOwnership: 'EXCLUSIVE_V08_HOST_RECIPIENTS', busyToIdleOnly: true,
    destination: { instanceId: 'codexTestM2Ready', seatId: 'host', worktreeId: 'hostF' },
    alternateDestination: { instanceId: 'codexTestM2Ready', seatId: 'alternate', worktreeId: 'alternateF' } };
  values.config.rules.priorCore13 = { journal: path.resolve('sealed-core-journal.json'), journalSha256: 'a'.repeat(64),
    finalReadback: path.resolve('sealed-core-readback.json'), finalSha256: 'b'.repeat(64) };
  return values;
}

test('existing Host busy-to-idle selection receives the same fixed source qualification', () => {
  const { config, raw } = hostFixture();
  const original = structuredClone({ config, raw });
  assert.ok(Object.values(managedCodexQualifications(config, raw)).every(value => value === true));
  assert.deepEqual({ config, raw }, original);
});
for (const [name, change] of [
  ['other selection', ({ config }) => { config.rules.selection = 'CROSS_PROJECT_ONLY'; }],
  ['unsliced Host', ({ config }) => { delete config.rules.selection; }],
  ['all Host cases', ({ config }) => { config.rules.host.busyToIdleOnly = false; }],
  ['Host ownership missing', ({ config }) => { delete config.rules.host.lifecycleOwnership; }],
  ['foreign project mixed in', ({ config }) => { config.rules.foreignProject = {}; }],
  ['other recipient instance', ({ config }) => { config.rules.host.destination.instanceId = 'codexOther'; }],
  ['duplicate recipient seat', ({ config }) => { config.rules.host.destination.seatId = 'submitter'; }],
  ['duplicate recipient worktree', ({ config }) => { config.rules.host.alternateDestination.worktreeId = 'hostF'; }],
  ['missing original core refs', ({ config }) => { delete config.rules.priorCore13; }],
  ['unsealed original core refs', ({ config }) => { config.rules.priorCore13.finalSha256 = null; }],
  ['relative original core ref', ({ config }) => { config.rules.priorCore13.journal = 'relative.json'; }],
  ['Host login absent', ({ raw }) => { raw.instance.login_state = 'LOGGED_OUT'; }],
  ['Host source not managed', ({ raw }) => { raw.programSource.source = 'EXTERNAL'; }],
  ['Host copy not ready', ({ raw }) => { raw.managedCopy.state = 'UNKNOWN'; }],
  ['Host current DB unpreserved', ({ raw }) => { raw.databaseBytesPreserved = false; }],
  ['Host digest differs', ({ raw }) => { raw.instance.program_digest = `sha256:${'0'.repeat(64)}`; }],
  ['Host home differs', ({ raw }) => { raw.programSource.home_identity = home.replace(/1$/, '2'); }],
]) {
  test(`reject ${name}`, () => {
    const values = hostFixture(); change(values);
    assert.throws(() => managedCodexQualifications(values.config, values.raw), /qualification differs/);
  });
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
  const v06 = v06Fixture();
  v06.config.stateRoot = directory;
  v06.config.managedSourceRead = { ...config.managedSourceRead };
  assert.equal(readManagedCodexSource(v06.config).qualifications.fixedCase, true);
  const hostConfig = { ...hostFixture().config, stateRoot: directory,
    managedSourceRead: { ...config.managedSourceRead } };
  assert.equal(readManagedCodexSource(hostConfig).fact.login_state, 'LOGGED_IN');
  fs.appendFileSync(source, '\n');
  assert.throws(() => readManagedCodexSource(config), /seal differs/);
  assert.throws(() => readManagedCodexSource(hostConfig), /seal differs/);
  fs.writeFileSync(source, bytes);
  fs.appendFileSync(database, 'changed');
  assert.throws(() => readManagedCodexSource(config), /current closed database bytes/);
  assert.throws(() => readManagedCodexSource(hostConfig), /current closed database bytes/);
  fs.writeFileSync(database, databaseBytes);
  for (const suffix of ['-wal', '-shm', '-journal']) {
    fs.writeFileSync(database + suffix, 'uncheckpointed');
    assert.throws(() => readManagedCodexSource(config), /closed checkpointed database/);
    assert.throws(() => readManagedCodexSource(hostConfig), /closed checkpointed database/);
    fs.unlinkSync(database + suffix);
  }
  config.managedSourceRead.path = 'relative.json';
  assert.throws(() => readManagedCodexSource(config), /absolute path/);
});
