import { causalCloseSources } from '../causal-close/causal-close.mjs';
import path from 'node:path';

// Offline transport fixture; never a process observation or product result.
export function closeFixture(pid = 10, mode = 'ATTACHED_RETAINED_OS_HANDLES') {
  const binding = { installed: path.resolve('fixture-installed'), installedSha256: {
    'gogoke.exe': 'a'.repeat(64), 'gogoke-native-host.exe': 'b'.repeat(64) } };
  const image = (name, digit) => ({ path: path.join(binding.installed, name),
    sha256: binding.installedSha256[name], volumeSerial: '1234abcd', fileId: `0x${digit.repeat(32)}` });
  const ready = { kind: 'READY', shell: { role: 'shell', pid, creationUtc: 'shell-created',
    cimCreationUtc: 'shell-cim-created', image: image('gogoke.exe', '1') },
    native: { role: 'native', pid: pid + 1, parentPid: pid, creationUtc: 'native-created',
      image: image('gogoke-native-host.exe', '2') },
    nativeAncestorChain: [{ pid, creationUtc: 'shell-cim-created', parentPid: pid - 1 }] };
  const token = `caption_${'a'.repeat(32)}`;
  const arm = { token, waitResult: 'WAIT_TIMEOUT', waitMilliseconds: 0,
    shellPid: pid, shellCreationUtc: ready.shell.creationUtc,
    nativePid: pid + 1, nativeCreationUtc: ready.native.creationUtc };
  const proof = { token, command: 'plugin:window|close', label: 'main', arm,
    dispatchFunction: `g37CaptionDispatch_${'b'.repeat(32)}`,
    breakpoint: 'CONDITIONAL_ORIGINAL_INVOKE_IN_CAPTION_DISPATCH_STACK',
    click: 'ONE_ACTUAL_CAPTION_DOM_EVENT_IN_SYNCHRONOUS_DISPATCH', resumedOriginalCall: true };
  const events = ['shell', 'native'].map(role => ({ role, ...ready[role],
    code: 0, codeBasis: 'GET_EXIT_CODE_PROCESS' }));
  const attached = mode === 'ATTACHED_RETAINED_OS_HANDLES';
  const attempt = { state: attached ? 'ATTACHED_CAUSAL_PROOF_REVIEW_REQUIRED' : 'CAUSAL_PROOF_REVIEW_REQUIRED',
    source: causalCloseSources(), ready, pausedCall: proof, events,
    observerCompleted: true, helperExitCode: 0, complete: { installedCandidateProcesses: 0 },
    ...(attached ? {} : { childExit: { code: 0, signal: null } }) };
  const close = { pid, exitCode: attached ? null : 0, signal: null, forceKill: false,
    observationMode: mode, causalProof: proof, shellExit: events[0], nativeExit: events[1],
    nativeIdentity: ready.native,
    ...(attached ? { parentChildObservation: 'NOT_AVAILABLE_TO_CURRENT_OBSERVER' } : {}) };
  return { causalCloseAttempts: [attempt], closes: [close], boundInstalledIdentity: binding };
}
