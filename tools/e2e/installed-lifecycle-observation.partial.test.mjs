// Focused evidence-persistence tests for the installed close observer.
import test from 'node:test';
import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import { PassThrough } from 'node:stream';
import { beginInstalledCloseAttempt, observeInstalledClose } from './installed-lifecycle-observation.mjs';

const shellPid = 2201;
const nativePid = 2202;
const createdShell = '2026-10-13T23:00:00.000Z';
const createdNative = '2026-10-13T23:00:01.000Z';
const image = (name, digest) => ({ path: `C:\\candidate\\${name}.exe`, sha256: digest,
  volumeSerial: 'ABCD1234', fileId: `file-${name}` });
const config = { pwsh: 'pinned-pwsh.exe', installed: 'C:\\candidate', installedSha256: {
  'gogoke.exe': 'a'.repeat(64), 'gogoke-native-host.exe': 'b'.repeat(64) } };
const ready = { kind: 'READY', observedUtc: '2026-10-13T23:01:00.000Z',
  shell: { role: 'shell', pid: shellPid, creationUtc: createdShell,
    image: image('gogoke', config.installedSha256['gogoke.exe']) },
  native: { role: 'native', pid: nativePid, creationUtc: createdNative,
    image: image('gogoke-native-host', config.installedSha256['gogoke-native-host.exe']) },
  nativeAncestorChain: [{ pid: shellPid }] };
const exit = (role, pid, creationUtc) => ({ kind: 'EXIT', role, pid, creationUtc,
  observedUtc: '2026-10-13T23:02:00.000Z', code: 0, codeBasis: 'GET_EXIT_CODE_PROCESS' });
const shellExit = exit('shell', shellPid, createdShell);
const nativeExit = exit('native', nativePid, createdNative);
const click = { clickedUtc: '2026-10-13T23:01:30.000Z', marker: 'captionClick_test',
  basis: 'ACTUAL_CAPTION_CLICK_EVENT' };

function fakeSpawn(frames, { exitCode = 0, stderr = '' } = {}) {
  return () => {
    const child = new EventEmitter();
    child.stdout = new PassThrough();
    child.stderr = new PassThrough();
    process.nextTick(() => {
      if (stderr) child.stderr.write(stderr);
      for (const frame of frames) child.stdout.write(`${JSON.stringify(frame)}\n`);
      child.stdout.end(); child.stderr.end();
      process.nextTick(() => child.emit('close', exitCode));
    });
    return child;
  };
}

function recorder() {
  const journal = {};
  let saves = 0;
  const persistence = beginInstalledCloseAttempt(journal, () => { saves++; },
    { path: 'loaded observer source', sha256: 'c'.repeat(64) });
  return { journal, persistence, get saves() { return saves; } };
}

test('verified partial EXIT survives original helper failure with caption, source pin, and error', async () => {
  const record = recorder();
  const observer = observeInstalledClose(config, shellPid, {
    ...record.persistence.hooks,
    spawnImpl: fakeSpawn([ready, shellExit], { exitCode: 1,
      stderr: 'Original candidate close process exit observation deadline' }),
  });
  await observer.ready;
  record.persistence.recordCaptionClick(click);
  await assert.rejects(observer.done, /Original shell\/native close event incomplete/);

  const attempt = record.journal.nativeCloseObservationAttempts[0];
  assert.equal(attempt.state, 'UNKNOWN_OR_INCOMPLETE');
  assert.deepEqual(attempt.events.map(row => row.event), [shellExit]);
  assert.equal(attempt.events[0].observerSource.path, 'loaded observer source');
  assert.match(attempt.events[0].observerSource.sha256, /^[a-f0-9]{64}$/);
  assert.equal(attempt.captionClick.marker, click.marker);
  assert.match(attempt.failure.message, /Original candidate close process exit observation deadline/);
  assert.equal(record.saves, 5, 'start, READY, EXIT, caption, failure are persisted at each boundary');
});

test('malformed EXIT is rejected and never stored among verified events', async () => {
  const record = recorder();
  const malformed = { ...nativeExit, pid: nativePid + 1 };
  const observer = observeInstalledClose(config, shellPid, {
    ...record.persistence.hooks,
    spawnImpl: fakeSpawn([ready, shellExit, malformed], { exitCode: 1 }),
  });
  await observer.ready;
  await assert.rejects(observer.done, /Original candidate close observer frame differs/);

  const attempt = record.journal.nativeCloseObservationAttempts[0];
  assert.equal(attempt.events.length, 1);
  assert.deepEqual(attempt.events[0].event, shellExit);
  assert.equal(attempt.failure.message, 'Original candidate close observer frame differs');
  assert.equal(attempt.partial.verifiedEventCount, 1);
});

test('complete observer stream still requires both process-bound GET_EXIT_CODE_PROCESS events', async () => {
  const record = recorder();
  const observer = observeInstalledClose(config, shellPid, {
    ...record.persistence.hooks,
    spawnImpl: fakeSpawn([ready, shellExit, nativeExit,
      { kind: 'COMPLETE', installedCandidateProcesses: 0 }]),
  });
  await observer.ready;
  const observed = await observer.done;
  assert.equal(observed.completed, true);
  assert.deepEqual(observed.events, [shellExit, nativeExit]);
  const attempt = record.journal.nativeCloseObservationAttempts[0];
  assert.equal(attempt.observerCompleted, true);
  assert.equal(attempt.failure, null);
  assert.equal(attempt.events.length, 2);
  assert.ok(attempt.events.every(row => row.event.codeBasis === 'GET_EXIT_CODE_PROCESS'));
});
