// One original caption click, paused at the existing Tauri invoke call, and
// one retained-object zero-wait barrier. This module has no launch side effect.
import fs from 'node:fs';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { createInterface } from 'node:readline';
import { createHash, randomUUID } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { isDeepStrictEqual } from 'node:util';

const loadedPath = fileURLToPath(import.meta.url);
const helperPath = path.join(path.dirname(loadedPath), 'candidate-close-events.ps1');
const hash = file => createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const loadedHash = hash(loadedPath);
const helperHash = hash(helperPath);
const command = 'plugin:window|close';
const waitMs = 15000;
const tokenPattern = /^[A-Za-z0-9_-]{16,128}$/;
export function causalCloseSources() {
  if (hash(loadedPath) !== loadedHash || hash(helperPath) !== helperHash)
    throw Error('Causal close source changed since module load');
  return { module: { path: loadedPath, sha256: loadedHash }, helper: { path: helperPath, sha256: helperHash } };
}
const deferred = () => {
  let resolve, reject;
  const promise = new Promise((a, b) => { resolve = a; reject = b; });
  promise.catch(() => {});
  return { promise, resolve, reject };
};
const exactIdentity = (actual, original) => actual.shellPid === original.shell.pid &&
  actual.shellCreationUtc === original.shell.creationUtc &&
  actual.nativePid === original.native.pid &&
  actual.nativeCreationUtc === original.native.creationUtc;

export function qualifyAttachedCausalExit(ready, observed) {
  const shell = observed?.events?.find(item => item.role === 'shell');
  const native = observed?.events?.find(item => item.role === 'native');
  if (!shell || !native || shell.code !== 0 || native.code !== 0 ||
      shell.pid !== ready?.shell?.pid || native.pid !== ready?.native?.pid ||
      shell.creationUtc !== ready?.shell?.creationUtc || native.creationUtc !== ready?.native?.creationUtc ||
      shell.codeBasis !== 'GET_EXIT_CODE_PROCESS' || native.codeBasis !== 'GET_EXIT_CODE_PROCESS' ||
      ready !== observed.ready || observed.helperExitCode !== 0 ||
      observed.complete !== true || observed.events.length !== 2 ||
      observed.armed?.waitResult !== 'WAIT_TIMEOUT' ||
      observed.armed?.waitMilliseconds !== 0 || !exactIdentity(observed.armed, ready)) {
    throw Error('Attached retained process exit, identity, or ARM proof differs');
  }
  return { shell, native };
}

// One normal-close receipt contract for both driver modes. Spawned mode has
// already checked its real NodeChild in closeCausally; attached mode records
// the absence explicitly and relies only on the two real OS process events.
export function requireRecordedNormalClose(journal, endpoint, expectedSource = causalCloseSources(), binding) {
  const attempt = journal.causalCloseAttempts?.at(-1);
  const close = journal.closes?.at(-1);
  if (!attempt || !close || close.pid !== endpoint?.pid || close.forceKill !== false) {
    throw Error('Recorded normal close is absent or belongs to another endpoint');
  }
  const ready = attempt.ready;
  const proof = attempt.pausedCall;
  const pins = binding?.installedSha256;
  const chain = ready?.nativeAncestorChain;
  const imageMatches = (role, name) => {
    const process = ready?.[role], image = process?.image;
    return process?.role === role && Number.isSafeInteger(process.pid) && process.pid > 0 &&
      typeof image?.path === 'string' && path.isAbsolute(image.path) &&
      typeof binding?.installed === 'string' && path.isAbsolute(binding.installed) &&
      path.resolve(image.path) === path.resolve(binding.installed, name) &&
      /^[a-f0-9]{64}$/.test(pins?.[name] ?? '') && image.sha256 === pins[name] &&
      /^[a-f0-9]{8}$/i.test(image.volumeSerial ?? '') &&
      /^0x[a-f0-9]{32}$/i.test(image.fileId ?? '');
  };
  if (ready?.kind !== 'READY' || !imageMatches('shell', 'gogoke.exe') ||
      !imageMatches('native', 'gogoke-native-host.exe') || !Array.isArray(chain) || chain.length === 0 ||
      chain[0].pid !== ready.native.parentPid || chain.at(-1).pid !== ready.shell.pid ||
      !ready.shell.cimCreationUtc || chain.at(-1).creationUtc !== ready.shell.cimCreationUtc ||
      chain.some((row, index) => !Number.isSafeInteger(row.pid) || row.pid <= 0 ||
        typeof row.creationUtc !== 'string' || !row.creationUtc ||
        (index < chain.length - 1 && row.parentPid !== chain[index + 1].pid))) {
    throw Error('Recorded normal close READY image, physical identity or ancestry differs');
  }
  if (!isDeepStrictEqual(attempt.source, expectedSource) ||
      !isDeepStrictEqual(proof, close.causalProof) ||
      !proof || proof.command !== command || !tokenPattern.test(proof.token ?? '') ||
      typeof proof.label !== 'string' || !proof.label ||
      !/^g37CaptionDispatch_[a-f0-9]{32}$/.test(proof.dispatchFunction ?? '') ||
      proof.breakpoint !== 'CONDITIONAL_ORIGINAL_INVOKE_IN_CAPTION_DISPATCH_STACK' ||
      proof.click !== 'ONE_ACTUAL_CAPTION_DOM_EVENT_IN_SYNCHRONOUS_DISPATCH' ||
      proof.resumedOriginalCall !== true || proof.arm?.token !== proof.token ||
      ready?.shell?.pid !== endpoint.pid || ready?.native?.pid === endpoint.pid ||
      !ready?.shell?.creationUtc || !ready?.native?.creationUtc ||
      attempt.observerCompleted !== true || attempt.complete?.installedCandidateProcesses !== 0) {
    throw Error('Recorded normal close source, caption, identity or completion differs');
  }
  for (const pin of Object.values(expectedSource)) {
    if (!pin?.path || !/^[a-f0-9]{64}$/.test(pin.sha256 ?? '') || hash(pin.path) !== pin.sha256)
      throw Error('Recorded normal close source bytes differ');
  }
  qualifyAttachedCausalExit(ready, { ready, armed: proof.arm, events: attempt.events,
    complete: attempt.observerCompleted, helperExitCode: attempt.helperExitCode });
  const shell = attempt.events.find(row => row.role === 'shell');
  const native = attempt.events.find(row => row.role === 'native');
  if (!isDeepStrictEqual(shell, close.shellExit) || !isDeepStrictEqual(native, close.nativeExit) ||
      !isDeepStrictEqual(ready.native, close.nativeIdentity))
    throw Error('Recorded normal close event identities differ');
  if (close.observationMode === 'ATTACHED_RETAINED_OS_HANDLES') {
    const ready = attempt.ready;
    const armed = close.causalProof?.arm;
    const sameExit = (actual, recorded) => actual?.role === recorded?.role &&
      actual?.pid === recorded?.pid && actual?.creationUtc === recorded?.creationUtc &&
      actual?.code === recorded?.code && actual?.codeBasis === recorded?.codeBasis;
    if (attempt.state !== 'ATTACHED_CAUSAL_PROOF_REVIEW_REQUIRED' ||
        attempt.observerCompleted !== true || attempt.helperExitCode !== 0 ||
        attempt.complete?.installedCandidateProcesses !== 0 || attempt.events?.length !== 2 ||
        !ready?.shell || !ready?.native || !armed ||
        armed.waitResult !== 'WAIT_TIMEOUT' || armed.waitMilliseconds !== 0 ||
        !exactIdentity(armed, ready) ||
        close.exitCode !== null || close.signal !== null ||
        close.parentChildObservation !== 'NOT_AVAILABLE_TO_CURRENT_OBSERVER' ||
        !sameExit(close.shellExit, attempt.events.find(row => row.role === 'shell')) ||
        !sameExit(close.nativeExit, attempt.events.find(row => row.role === 'native')) ||
        close.nativeIdentity?.pid !== ready.native.pid ||
        close.nativeIdentity?.creationUtc !== ready.native.creationUtc) {
      throw Error('Attached normal close receipt lacks the mode-specific OS proof');
    }
    return { mode: 'ATTACHED_RETAINED_OS_HANDLES', attempt, close };
  }
  const currentSpawned = close.observationMode === 'SPAWNED_NODE_CHILD' && close.signal === null;
  const retainedOriginalSpawned = !close.observationMode && close.causalProof &&
    close.shellExit?.code === 0 && close.nativeExit?.code === 0 &&
    journal.productExits?.some(row => row.pid === endpoint.pid && row.code === 0 && row.signal === null);
  if (attempt.state !== 'CAUSAL_PROOF_REVIEW_REQUIRED' || close.exitCode !== 0 ||
      attempt.childExit?.code !== 0 || attempt.childExit?.signal !== null ||
      (!currentSpawned && !retainedOriginalSpawned)) {
    throw Error('Spawned normal close receipt lacks its NodeChild proof');
  }
  return { mode: 'SPAWNED_NODE_CHILD', attempt, close };
}

export function observeCausalClose(config, shellPid, hooks = {}) {
  causalCloseSources();
  const child = (hooks.spawnImpl ?? spawn)(config.pwsh, ['-NoProfile', '-NonInteractive', '-File',
    helperPath, '-Installed', config.installed, '-ShellPid', String(shellPid),
    '-ShellSha256', config.installedSha256['gogoke.exe'],
    '-NativeSha256', config.installedSha256['gogoke-native-host.exe']],
  { windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] });
  const ready = deferred(), armed = deferred(), done = deferred();
  let original, requestedToken, armedFrame, complete = false, failure, closeCode, readyAt;
  let stderr = '';
  const events = [];
  const snapshot = () => ({ ready: original ?? null, armed: armedFrame ?? null, events: [...events], complete });
  const fail = error => {
    if (failure) return;
    failure = error;
    ready.reject(error); armed.reject(error); done.reject(error);
    hooks.onFailure?.(error, snapshot());
  };
  const timer = setTimeout(() => fail(Error('Retained original handle readiness deadline')), waitMs);
  child.stderr.on('data', bytes => { stderr = (stderr + bytes).slice(-8192); });
  child.stdin.on('error', fail);
  createInterface({ input: child.stdout }).on('line', line => {
    if (failure) return;
    try {
      const item = JSON.parse(line);
      if (item.kind === 'READY' && !original &&
          item.shell?.pid === shellPid && item.native?.pid > 0 && item.native.pid !== shellPid &&
          item.shell.image?.sha256 === config.installedSha256['gogoke.exe'] &&
          item.native.image?.sha256 === config.installedSha256['gogoke-native-host.exe'] &&
          item.shell.image.fileId && item.native.image.fileId &&
          item.shell.creationUtc && item.native.creationUtc &&
          Array.isArray(item.nativeAncestorChain) &&
          item.nativeAncestorChain.at(-1)?.pid === shellPid) {
        original = item; readyAt = performance.now(); clearTimeout(timer); hooks.onReady?.(item); ready.resolve(item);
      } else if (item.kind === 'ARMED' && original && requestedToken && !armedFrame &&
          events.length === 0 && item.token === requestedToken && exactIdentity(item, original) &&
          item.waitResult === 'WAIT_TIMEOUT' && item.waitMilliseconds === 0) {
        armedFrame = item; hooks.onArmed?.(item); armed.resolve(item);
      } else if (item.kind === 'EXIT' && armedFrame && !complete &&
          ['shell', 'native'].includes(item.role) && !events.some(e => e.role === item.role) &&
          item.pid === original[item.role].pid &&
          item.creationUtc === original[item.role].creationUtc &&
          item.codeBasis === 'GET_EXIT_CODE_PROCESS' &&
          Number.isSafeInteger(item.code) && item.code >= 0) {
        events.push(item); hooks.onEvent?.(item);
      } else if (item.kind === 'COMPLETE' && armedFrame && events.length === 2 &&
          item.installedCandidateProcesses === 0 && !complete) {
        complete = true; hooks.onComplete?.(item);
      } else throw Error('Retained-object causal observer frame differs');
    } catch (error) { fail(error); }
  });
  child.once('error', fail);
  child.once('close', code => {
    clearTimeout(timer); closeCode = code;
    if (code !== 0 || failure || !complete || events.length !== 2 || !armedFrame)
      fail(failure ?? Error(`Causal close helper incomplete: exit=${code}; ${stderr}`));
    else done.resolve({ ready: original, armed: armedFrame, events, complete, helperExitCode: code });
  });
  return {
    ready: ready.promise, armed: armed.promise, done: done.promise,
    remainingMs() { return readyAt === undefined ? 0 : Math.max(0, 30000 - (performance.now() - readyAt)); },
    get snapshot() { return { ...snapshot(), helperExitCode: closeCode }; },
    arm(token) {
      if (!original || requestedToken || failure || !tokenPattern.test(token))
        throw Error('Causal ARM unavailable or already sent');
      requestedToken = token;
      child.stdin.write(JSON.stringify({ kind: 'ARM', token,
        shellPid: original.shell.pid, shellCreationUtc: original.shell.creationUtc,
        nativePid: original.native.pid, nativeCreationUtc: original.native.creationUtc }) + '\n');
    },
  };
}

// CDP request only; never inspects or records the Tauri IPC envelope.
export function createCdp(socket, nextId, timeoutMs = waitMs) {
  return (method, params = {}) => new Promise((resolve, reject) => {
    if (socket.readyState !== WebSocket.OPEN) return reject(Error('Original product CDP socket absent'));
    const id = nextId();
    const finish = (error, value) => {
      clearTimeout(timer); socket.removeEventListener('message', received);
      socket.removeEventListener('close', closed);
      error ? reject(error) : resolve(value);
    };
    const received = event => {
      let message;
      try { message = JSON.parse(event.data); } catch (error) { return finish(error); }
      if (message.id !== id) return;
      if (message.error || message.result?.exceptionDetails)
        finish(Error(`${method} rejected or evaluation failed`));
      else finish(null, message.result);
    };
    const closed = () => finish(Error(`${method} connection closed`));
    const timer = setTimeout(() => finish(Error(`${method} response deadline`)), timeoutMs);
    socket.addEventListener('message', received); socket.addEventListener('close', closed);
    try { socket.send(JSON.stringify({ id, method, params })); }
    catch (error) { finish(error); }
  });
}

export function captionBreakpointCondition(markerKey, token, currentLabel) {
  // V8 evaluates this in the original invoke call frame. No IPC envelope is read.
  return `arguments[0]===${JSON.stringify(command)}&&arguments[1]?.label===${JSON.stringify(currentLabel)}&&` +
    `globalThis[${JSON.stringify(markerKey)}]?.token===${JSON.stringify(token)}&&` +
    `globalThis[${JSON.stringify(markerKey)}]?.activeDispatch===true&&` +
    `globalThis[${JSON.stringify(markerKey)}]?.eventSeen===true`;
}

export function exactPausedCall(params, breakpointId, dispatchName) {
  return Boolean(params?.hitBreakpoints?.includes(breakpointId) &&
    params.callFrames?.[0]?.callFrameId &&
    params.callFrames.some(frame => frame.functionName === dispatchName));
}

export async function pauseOriginalCaptionInvoke(product, observer, hooks = {}) {
  const socket = product.socket;
  if (!socket || socket.readyState !== WebSocket.OPEN) throw Error('Original caption socket absent');
  const request = hooks.request ?? createCdp(socket, () => ++product.sequence);
  const token = `caption_${randomUUID().replaceAll('-', '')}`;
  const markerKey = `__gogokeCausalCaption_${randomUUID().replaceAll('-', '')}`;
  const dispatchName = `g37CaptionDispatch_${randomUUID().replaceAll('-', '')}`;
  let breakpointId, pausedId, pauseFailure, sawPause = false;
  let resumeAttempted = false, resumeConfirmed = false, clickScheduled = false;
  const pause = deferred();
  const remaining = () => Math.min(waitMs, observer.remainingMs?.() ?? 30000);
  const bounded = async (work, step) => {
    const ms = remaining();
    if (ms <= 0) throw Error(`${step} exceeded original 30 s observer ceiling`);
    let timer;
    return Promise.race([Promise.resolve().then(work),
      new Promise((_, reject) => { timer = setTimeout(() =>
        reject(Error(`${step} exceeded original 30 s observer ceiling`)), ms); }),
    ]).finally(() => clearTimeout(timer));
  };
  const ask = (method, params) => bounded(() => request(method, params), method);
  const onMessage = event => {
    try {
      const frame = JSON.parse(event.data);
      if (frame.method !== 'Debugger.paused') return;
      if (sawPause || pauseFailure) {
        pauseFailure = Error('Duplicate or unexpected Debugger pause');
        pause.reject(pauseFailure);
        return;
      }
      sawPause = true;
      pausedId = frame.params?.callFrames?.[0]?.callFrameId;
      if (!breakpointId || !exactPausedCall(frame.params, breakpointId, dispatchName)) {
        pauseFailure = Error('Debugger paused outside the exact caption dispatch stack');
        pause.reject(pauseFailure);
      } else pause.resolve(pausedId);
    } catch (error) { pauseFailure = error; pause.reject(error); }
  };
  socket.addEventListener('message', onMessage);
  const cleanupRequest = async (method, params) => {
    if (remaining() > 0) await ask(method, params).catch(() => {});
    else Promise.resolve(request(method, params)).catch(() => {});
  };
  const cleanup = async () => {
    if (breakpointId) await cleanupRequest('Debugger.removeBreakpoint', { breakpointId });
    if (sawPause && !resumeAttempted) {
      resumeAttempted = true; // Release only this original call; proof remains failed.
      await cleanupRequest('Debugger.resume');
    }
    socket.removeEventListener('message', onMessage);
    if (!clickScheduled) {
      await ask('Runtime.evaluate', { expression: `(()=>{const s=window[${JSON.stringify(markerKey)}];
        if(s){s.button.removeEventListener('click',s.listener,true);delete window[${JSON.stringify(markerKey)}]}})()`,
        returnByValue: true, silent: true }).catch(() => {});
    }
    if (pauseFailure) throw pauseFailure;
  };
  try {
    await ask('Debugger.enable');
    const fn = await ask('Runtime.evaluate', {
      expression: 'window.__TAURI_INTERNALS__?.invoke', returnByValue: false, silent: true });
    const objectId = fn.result?.objectId;
    if (fn.result?.type !== 'function' || !objectId) throw Error('Original Tauri invoke object absent');
    const labelResult = await ask('Runtime.evaluate', {
      expression: 'window.__TAURI_INTERNALS__?.metadata?.currentWindow?.label',
      returnByValue: true, silent: true });
    const currentLabel = labelResult.result?.value;
    if (typeof currentLabel !== 'string' || !currentLabel)
      throw Error('Actual Tauri window label unavailable');
    const prepared = await bounded(() => product.evaluate(`(()=>{const b=document.querySelector('.window-caption-control-close');
      if(!b||window[${JSON.stringify(markerKey)}]!==undefined)throw Error('Caption token setup differs');
      const s={token:${JSON.stringify(token)},activeDispatch:false,eventSeen:false,button:b};
      s.listener=()=>{if(s.activeDispatch)s.eventSeen=true};
      b.addEventListener('click',s.listener,{capture:true,once:true});
      window[${JSON.stringify(markerKey)}]=s;return 'CAPTION_ARMED'})()`), 'Caption marker setup');
    if (prepared !== 'CAPTION_ARMED') throw Error('Caption marker installation differs');
    const condition = captionBreakpointCondition(markerKey, token, currentLabel);
    const installed = await ask('Debugger.setBreakpointOnFunctionCall', { objectId, condition });
    breakpointId = installed.breakpointId;
    if (!breakpointId) throw Error('Conditional function-call breakpoint unsupported');
    const dispatch = product.evaluate(`(()=>{const s=window[${JSON.stringify(markerKey)}];
      if(!s||s.token!==${JSON.stringify(token)}||s.activeDispatch||s.eventSeen)throw Error('Caption scope differs');
      function ${dispatchName}(){s.activeDispatch=true;try{s.button.click()}finally{
        s.activeDispatch=false;s.eventSeen=false;s.button.removeEventListener('click',s.listener,true);
        delete window[${JSON.stringify(markerKey)}]}}
      setTimeout(${dispatchName},0);return 'ONE_CLICK_SCHEDULED'})()`);
    dispatch.catch(() => {});
    const clickReply = await bounded(() => dispatch, 'Original caption scheduling');
    if (clickReply !== 'ONE_CLICK_SCHEDULED') throw Error('Original caption scheduling differs');
    clickScheduled = true;
    const callFrameId = await bounded(() => pause.promise, 'Original caption invoke breakpoint');
    if (pauseFailure) throw pauseFailure;
    const args = await ask('Debugger.evaluateOnCallFrame', { callFrameId,
      expression: `({command:arguments[0],label:arguments[1]?.label,
        marker:globalThis[${JSON.stringify(markerKey)}]?.token,
        activeDispatch:globalThis[${JSON.stringify(markerKey)}]?.activeDispatch,
        eventSeen:globalThis[${JSON.stringify(markerKey)}]?.eventSeen})`,
      returnByValue: true, silent: true });
    const actual = args.result?.value;
    if (pauseFailure || actual?.command !== command || actual.label !== currentLabel ||
        actual.marker !== token || actual.activeDispatch !== true || actual.eventSeen !== true)
      throw Error('Paused call command, label, token, or exact event scope differs');
    observer.arm(token);
    const arm = await bounded(() => observer.armed, 'Retained-object ARM');
    if (pauseFailure || arm.token !== token) throw Error('ARMED token or pause sequence differs');
    await ask('Debugger.removeBreakpoint', { breakpointId }); breakpointId = null;
    if (pauseFailure) throw pauseFailure;
    resumeAttempted = true;
    await ask('Debugger.resume');
    resumeConfirmed = true;
    if (pauseFailure) throw pauseFailure;
    return { token, command, label: currentLabel,
      breakpoint: 'CONDITIONAL_ORIGINAL_INVOKE_IN_CAPTION_DISPATCH_STACK',
      click: 'ONE_ACTUAL_CAPTION_DOM_EVENT_IN_SYNCHRONOUS_DISPATCH',
      dispatchFunction: dispatchName,
      arm: { token: arm.token, waitResult: arm.waitResult, waitMilliseconds: arm.waitMilliseconds,
        shellPid: arm.shellPid, shellCreationUtc: arm.shellCreationUtc,
        nativePid: arm.nativePid, nativeCreationUtc: arm.nativeCreationUtc },
      resumedOriginalCall: resumeConfirmed };
  } finally { await cleanup(); }
}

export function qualifyCausalExit(ready, observed, childExit) {
  const shell = observed.events.find(item => item.role === 'shell');
  const native = observed.events.find(item => item.role === 'native');
  if (!shell || !native || shell.code !== 0 || native.code !== 0 ||
      childExit.code !== 0 || childExit.signal !== null || shell.code !== childExit.code ||
      ready !== observed.ready || observed.helperExitCode !== 0 ||
      observed.armed?.waitResult !== 'WAIT_TIMEOUT' || observed.armed?.waitMilliseconds !== 0 ||
      !exactIdentity(observed.armed, ready))
    throw Error('Original process exit code, retained identity, or ARM proof differs');
  return { shell, native };
}

export function observeOriginalChildExit(child, remainingMs) {
  const result = deferred();
  let timer;
  const dispose = () => {
    clearTimeout(timer);
    child.removeListener('exit', onExit);
    child.removeListener('error', onError);
  };
  const onExit = (code, signal) => { dispose(); result.resolve({ code, signal }); };
  const onError = error => { dispose(); result.reject(error); };
  child.once('exit', onExit);
  child.once('error', onError);
  timer = setTimeout(() => onError(Error('Original child exit missing within original observer ceiling')),
    Math.max(0, remainingMs()));
  return { promise: result.promise, dispose };
}

export async function closeCausally(product, hooks = {}) {
  causalCloseSources();
  await product.custody();
  const attempt = { schema: 'gogoke.37.causal-close-attempt.v1',
    state: 'OBSERVING', source: causalCloseSources(), ready: null,
    pausedCall: null, events: [], observerCompleted: false, failure: null };
  product.journal.causalCloseAttempts ??= [];
  product.journal.causalCloseAttempts.push(attempt); product.save();
  const observer = observeCausalClose(product.config, product.endpoint.pid, {
    ...hooks, onReady: value => { attempt.ready = value; product.save(); },
    onEvent: value => { attempt.events.push(value); product.save(); },
    onComplete: value => { attempt.observerCompleted = true; attempt.complete = value; product.save(); },
  });
  let childExitObservation;
  try {
    const ready = await observer.ready;
    if (product.child.exitCode !== null || product.child.signalCode !== null)
      throw Error('Original product exited before caption breakpoint');
    childExitObservation = observeOriginalChildExit(product.child, observer.remainingMs);
    attempt.pausedCall = await pauseOriginalCaptionInvoke(product, observer, hooks); product.save();
    const [childExit, observed] = await Promise.all([childExitObservation.promise, observer.done]);
    const { shell, native } = qualifyCausalExit(ready, observed, childExit);
    causalCloseSources();
    attempt.helperExitCode = observed.helperExitCode;
    attempt.childExit = childExit;
    attempt.state = 'CAUSAL_PROOF_REVIEW_REQUIRED'; product.save();
    product.journal.closes.push({ pid: product.endpoint.pid, exitCode: childExit.code,
      signal: childExit.signal, forceKill: false, observationMode: 'SPAWNED_NODE_CHILD',
      causalProof: attempt.pausedCall,
      shellExit: shell, nativeExit: native, nativeIdentity: ready.native }); product.save();
    product.socket.close();
    if (product.tester) { await product.tester.dispose(); product.tester = null; }
    return attempt;
  } catch (error) {
    attempt.state = 'UNKNOWN_OR_INCOMPLETE';
    attempt.failure = String(error?.stack ?? error);
    attempt.partial = observer.snapshot; product.save();
    throw error;
  } finally {
    childExitObservation?.dispose();
  }
}

export async function closeAttachedCausally(product, hooks = {}) {
  causalCloseSources();
  await product.custody();
  const attempt = { schema: 'gogoke.37.causal-close-attempt.v1',
    state: 'OBSERVING', observationMode: 'ATTACHED_RETAINED_OS_HANDLES',
    source: causalCloseSources(), ready: null, pausedCall: null, events: [],
    observerCompleted: false, parentChildObservation: 'NOT_AVAILABLE_TO_CURRENT_OBSERVER',
    failure: null };
  product.journal.causalCloseAttempts ??= [];
  product.journal.causalCloseAttempts.push(attempt); product.save();
  const observer = observeCausalClose(product.config, product.endpoint.pid, {
    ...hooks, onReady: value => { attempt.ready = value; product.save(); },
    onEvent: value => { attempt.events.push(value); product.save(); },
    onComplete: value => { attempt.observerCompleted = true; attempt.complete = value; product.save(); },
  });
  try {
    const ready = await observer.ready;
    attempt.ready = ready; product.save();
    attempt.pausedCall = await pauseOriginalCaptionInvoke(product, observer, hooks); product.save();
    const observed = await observer.done;
    const { shell, native } = qualifyAttachedCausalExit(ready, observed);
    causalCloseSources();
    attempt.helperExitCode = observed.helperExitCode;
    attempt.state = 'ATTACHED_CAUSAL_PROOF_REVIEW_REQUIRED';
    product.journal.closes.push({ pid: product.endpoint.pid, exitCode: null, signal: null,
      forceKill: false, observationMode: 'ATTACHED_RETAINED_OS_HANDLES',
      parentChildObservation: 'NOT_AVAILABLE_TO_CURRENT_OBSERVER',
      causalProof: attempt.pausedCall, shellExit: shell, nativeExit: native,
      nativeIdentity: ready.native });
    product.save();
    requireRecordedNormalClose(product.journal, product.endpoint, undefined, product.config);
    product.socket.close();
    if (product.tester) { await product.tester.dispose(); product.tester = null; }
    return attempt;
  } catch (error) {
    attempt.state = 'UNKNOWN_OR_INCOMPLETE';
    attempt.failure = String(error?.stack ?? error);
    attempt.partial = observer.snapshot; product.save();
    throw error;
  }
}
