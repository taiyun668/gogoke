// Evidence-boundary tests only. No product, CLI, login, model or real state DB.
// Set GOGOKE_E2E_TEST_TEMP_ROOT to an explicit private scratch directory.
import assert from 'node:assert/strict';
import { test } from 'node:test';
import fs from 'node:fs';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { managedSourceQualifications, readManagedSource } from './m2-provider-managed-source.mjs';

const digest = 'cf664aa1da32b788f9b2699b84a9bb9be30b7e025693b90f9b85829d5fe4e252';
const home = 'volume:0000000000000001/file:00000000000000000000000000000001';
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const databaseBytes = Buffer.from('ISOLATED EVIDENCE LOADER TEST; NOT A PRODUCT DATABASE');
function fixture() {
  return {
    row: { driverId: 'opencode', instanceId: 'opencodeTestM2', version: '1.18.32', sha256: digest,
      model: 'xai/grok-4.7', managedSourceRead: { homeIdentity: home } },
    raw: { state: 'ORIGINAL72_CLOSED_SELECTED_REGISTRY_SOURCE_NO_CREDENTIAL_CONTENT',
      instance: { instance_id: 'opencodeTestM2', driver_id: 'opencode', version: '1.18.32',
        program_digest: `sha256:${digest}`, install_state: 'UNKNOWN', login_state: 'LOGGED_IN',
        home_identity: home, revision: 3 },
      programSourceKind: 'MANAGED',
      programSource: { instance_id: 'opencodeTestM2', source: 'MANAGED', stage_name: 'opencode-stage-test',
        program_digest: `sha256:${digest}`, version: '1.18.32', home_identity: home,
        registration_request_id: 'e2e-test-original-registration', revision: 1 },
      managedCopy: { driver_id: 'opencode', state: 'READY', stage_name: 'opencode-stage-test',
        version: '1.18.32', image_sha256: digest },
      databaseBytesPreserved: true, databaseSha256: hash(databaseBytes), authContentOpened: false, acceptance: false },
  };
}

test('original schema preserves UNKNOWN install and LOGGED_IN native state', () => {
  const { row, raw } = fixture();
  assert.ok(Object.values(managedSourceQualifications(row, raw)).every(value => value === true));
  assert.equal(raw.instance.install_state, 'UNKNOWN');
  assert.equal(raw.instance.login_state, 'LOGGED_IN');
  raw.state = 'SUBSEQUENT_CLOSED_SELECTED_REGISTRY_SOURCE_NO_CREDENTIAL_CONTENT';
  assert.ok(Object.values(managedSourceQualifications(row, raw)).every(value => value === true));
});
test('absent proof supplies no alternative login fact', () => {
  assert.equal(readManagedSource({ ...fixture().row, managedSourceRead: undefined }, undefined), undefined);
});
for (const [name, change] of [
  ['other instance', ({ row }) => { row.instanceId = 'otherInstance'; }],
  ['other driver', ({ row }) => { row.driverId = 'claude'; }],
  ['other version', ({ row }) => { row.version = '1.18.33'; }],
  ['other digest', ({ row }) => { row.sha256 = '0'.repeat(64); }],
  ['other model', ({ row }) => { row.model = 'xai/grok-other'; }],
  ['combined legacy path', ({ row }) => { row.originalRegistryRead = {}; }],
  ['unpreserved read', ({ raw }) => { raw.databaseBytesPreserved = false; }],
  ['credential read', ({ raw }) => { raw.authContentOpened = true; }],
  ['acceptance claim', ({ raw }) => { raw.acceptance = true; }],
  ['invalid closure hash', ({ raw }) => { raw.databaseSha256 = 'unknown'; }],
  ['native digest mismatch', ({ raw }) => { raw.instance.program_digest = `sha256:${'0'.repeat(64)}`; }],
  ['native not logged in', ({ raw }) => { raw.instance.login_state = 'NOT_LOGGED_IN'; }],
  ['invented install state', ({ raw }) => { raw.instance.install_state = 'INSTALLED'; }],
  ['missing home pin', ({ row }) => { delete row.managedSourceRead.homeIdentity; }],
  ['native home mismatch', ({ raw }) => { raw.instance.home_identity = home.replace(/1$/, '2'); }],
  ['source home mismatch', ({ raw }) => { raw.programSource.home_identity = home.replace(/1$/, '2'); }],
  ['both homes mismatch', ({ raw }) => {
    raw.instance.home_identity = raw.programSource.home_identity = home.replace(/1$/, '2');
  }],
  ['source kind mismatch', ({ raw }) => { raw.programSourceKind = 'EXTERNAL'; }],
  ['source not managed', ({ raw }) => { raw.programSource.source = 'EXTERNAL'; }],
  ['source digest mismatch', ({ raw }) => { raw.programSource.program_digest = `sha256:${'0'.repeat(64)}`; }],
  ['copy not ready', ({ raw }) => { raw.managedCopy.state = 'FAILED'; }],
  ['copy digest mismatch', ({ raw }) => { raw.managedCopy.image_sha256 = '0'.repeat(64); }],
  ['different stage', ({ raw }) => { raw.managedCopy.stage_name = 'other-stage'; }],
]) {
  test(`reject ${name}`, () => {
    const values = fixture(); change(values);
    assert.throws(() => managedSourceQualifications(values.row, values.raw), /qualification differs/);
  });
}

test('sealed loader binds exact closed bytes and rejects changed files or nonempty sidecars', t => {
  const root = process.env.GOGOKE_E2E_TEST_TEMP_ROOT;
  assert.ok(root && path.isAbsolute(root), 'Explicit private test scratch directory required');
  const directory = fs.mkdtempSync(path.join(root, 'managed-source-boundary-'));
  t.after(() => {
    assert.equal(path.dirname(path.resolve(directory)), path.resolve(root));
    fs.rmSync(directory, { recursive: true, force: true });
  });
  const { row, raw } = fixture();
  const database = path.join(directory, 'state.sqlite');
  const source = path.join(directory, 'source.json');
  const sourceBytes = Buffer.from(JSON.stringify(raw));
  fs.writeFileSync(database, databaseBytes, { flag: 'wx' });
  fs.writeFileSync(source, sourceBytes, { flag: 'wx' });
  Object.assign(row.managedSourceRead, { path: source, sha256: hash(sourceBytes) });
  assert.equal(readManagedSource(row, directory).fact.login_state, 'LOGGED_IN');
  fs.appendFileSync(source, '\n');
  assert.throws(() => readManagedSource(row, directory), /seal differs/);
  fs.writeFileSync(source, sourceBytes);
  fs.appendFileSync(database, 'changed');
  assert.throws(() => readManagedSource(row, directory), /current closed database bytes/);
  fs.writeFileSync(database, databaseBytes);
  for (const suffix of ['-wal', '-shm', '-journal']) {
    fs.writeFileSync(database + suffix, 'uncheckpointed');
    assert.throws(() => readManagedSource(row, directory), /closed checkpointed database/);
    fs.unlinkSync(database + suffix);
  }
  row.managedSourceRead.path = 'relative-source.json';
  assert.throws(() => readManagedSource(row, directory), /absolute path/);
});
