// Read-only controls over the retained, actual 0.1.66 V07 success/failure sources.
// Run explicitly with GOGOKE_LIFECYCLE_ACTUAL_D pointing to the retained D archive.
import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { classifyRecordedReadiness, classifyRecordedClose } from './installed-lifecycle-observation.mjs';
import { ActualProduct } from './product-cdp.mjs';

const archive = process.env.GOGOKE_LIFECYCLE_ACTUAL_D;
if (!archive) throw Error('Required actual success/failure archive absent; validation cannot skip');
const digest = p => createHash('sha256').update(fs.readFileSync(p)).digest('hex');
const pinned = (relative, expected) => {
  const p = path.join(archive, relative);
  assert.equal(digest(p), expected, `actual source changed: ${relative}`);
  return p;
};
const read = p => JSON.parse(fs.readFileSync(p, 'utf8').replace(/^\uFEFF/, ''));

test('ActualProduct reports the actual shared module bytes it loads', () => {
  const journal = {};
  new ActualProduct({}, journal);
  const loaded = fileURLToPath(new URL('./installed-lifecycle-observation.mjs', import.meta.url));
  assert.equal(journal.driverBytes['installed-lifecycle-observation.mjs'], digest(loaded));
  assert.match(journal.driverBytes['installed-lifecycle-observation.mjs'], /^[a-f0-9]{64}$/);
});

test('actual V07 successful readiness and caption close retain installed identity and OS exits',
  { skip: !archive && 'actual D archive path required' }, () => {
    const p = pinned('g37-v07-path-corrected-case-prep-20261013/evidence/m2-stop-worktree-result.json',
      '1bcdbeb7d9bd38cf450500893f93fe992fffc9b78e463978ae406504a1f46101');
    const journal = read(p);
    const readyPath = pinned('g37-v07-path-corrected-case-prep-20261013/gogoke-update-c70c93e1bfde49ecad056db3c9bf1bc4.ready',
      '4f1b01f44085927e058b9c56606e768340fd3a3baba5209e1c860d6b78b6dc98');
    assert.equal(journal.currentEndpoint.readyPath, readyPath);
    const ready = classifyRecordedReadiness({ journal, readyReceipt: read(readyPath),
      readyCreatedUtc: fs.statSync(readyPath).birthtime.toISOString() });
    assert.equal(ready.state, 'ACCEPTED_ORIGINAL_LAUNCH');
    const close = classifyRecordedClose(journal);
    assert.equal(close.state, 'OBSERVED_NORMAL_0_0');
    assert.equal(close.shellPid, journal.closes[0].pid);
    assert.equal(close.nativePid, journal.closes[0].nativeIdentity.pid);
    const noNativeIdentity = structuredClone(journal);
    delete noNativeIdentity.nativeCloseObservations[0].ready.native.image.fileId;
    assert.equal(classifyRecordedClose(noNativeIdentity).state, 'UNKNOWN_OR_INCOMPLETE');
    const wrongShell = structuredClone(journal);
    wrongShell.closes[0].pid = 1;
    assert.equal(classifyRecordedClose(wrongShell).state, 'UNKNOWN_OR_INCOMPLETE');
    const wrongImage = structuredClone(journal);
    wrongImage.closes[0].nativeIdentity.image.fileId = '0'.repeat(32);
    assert.equal(classifyRecordedClose(wrongImage).state, 'UNKNOWN_OR_INCOMPLETE');
    const wrongExit = structuredClone(journal);
    wrongExit.closes[0].nativeExit.code = 1;
    assert.equal(classifyRecordedClose(wrongExit).state, 'UNKNOWN_OR_INCOMPLETE');
    assert.equal(journal.driverBytes['product-cdp.mjs'],
      'a6f0ae944aae977fbe38b1606ffc1e662a7eab551557d14b521bc0f5ac8515c2');
  });

test('actual V07 failed readiness keeps the late valid receipt as a failed original launch',
  { skip: !archive && 'actual D archive path required' }, () => {
    const p = pinned('g37-v07-stopped-source-repair-prep-20261013/original-case-forensic-once.json',
      'd67f7861ddc4ecd02afa9dc06c883514ba394fbe24914517da9b997f16a6dbfc');
    const journal = read(p);
    const readyPath = pinned('g37-v07-current66-prep-20261013/gogoke-update-b15d7c62df0c475b9a50122305007182.ready',
      '4f1b01f44085927e058b9c56606e768340fd3a3baba5209e1c860d6b78b6dc98');
    const report = fs.readFileSync(pinned('g37-v07-stopped-source-repair-prep-20261013/V07-READY-MISSING-SPECIALIST.md',
      'f326ffc69c18da7ab56122afea397424f18608b29c73c13ba11bbd7ae3cf41cd'), 'utf8');
    assert.match(report, /原evidence\/m2-stop-worktree-result\.json.*16:06:44\.7075668Z/);
    const failureRecordedUtc = '2026-10-13T16:06:44.7075668Z';
    const stderrPath = journal.currentEndpoint.stderrPath;
    assert.equal(digest(stderrPath), 'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855');
    const state = classifyRecordedReadiness({ journal, readyReceipt: read(readyPath),
      readyCreatedUtc: fs.statSync(readyPath).birthtime.toISOString(), failureRecordedUtc,
      spawnBoundaryUtc: fs.statSync(stderrPath).birthtime.toISOString() });
    assert.equal(state.state, 'LATE_VALID_RECEIPT_ORIGINAL_FAILURE');
    const wrongReceipt = { ...read(readyPath), generationId: 'different' };
    assert.equal(classifyRecordedReadiness({ journal, readyReceipt: wrongReceipt,
      readyCreatedUtc: fs.statSync(readyPath).birthtime.toISOString(), failureRecordedUtc }).state,
      'UNKNOWN_OR_INCOMPLETE');
    assert.ok(state.timing.spawnBoundaryToReadyMs > 180000);
    assert.ok(state.timing.failureToReadyMs > 0);
    assert.equal(journal.startupReadiness.readyReceiptPresent, false);
    assert.equal(journal.operations.length, 0);
    assert.equal(classifyRecordedClose(journal).state, 'UNKNOWN_OR_INCOMPLETE');
  });
