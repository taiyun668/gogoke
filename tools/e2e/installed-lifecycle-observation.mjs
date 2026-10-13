// Shared installed-candidate readiness and OS-handle close observation.
// No product launch, model call, credential access or native build on import.
import fs from 'node:fs';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';
import { randomUUID, createHash } from 'node:crypto';
const here = path.dirname(fileURLToPath(import.meta.url));
const loadedPath = fileURLToPath(import.meta.url);
const sourceHash = file => createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const loadedHash = sourceHash(loadedPath);
export function lifecycleSource() {
  if (sourceHash(loadedPath) !== loadedHash) throw Error('Loaded lifecycle instrument source changed');
  return { path: loadedPath, sha256: loadedHash };
}
const readJson = file => JSON.parse(fs.readFileSync(file, 'utf8').replace(/^\uFEFF/, ''));
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const id = prefix => `${prefix}_${randomUUID().replaceAll('-', '')}`;

function observeInstalledClose(config, shellPid) {
  const helper = spawn(config.pwsh, ['-NoProfile', '-NonInteractive', '-File',
    path.join(here, 'candidate-close-events.ps1'), '-Installed', config.installed,
    '-ShellPid', String(shellPid), '-ShellSha256', config.installedSha256['gogoke.exe'],
    '-NativeSha256', config.installedSha256['gogoke-native-host.exe']],
  { windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
  let resolveReady, rejectReady, resolveDone, rejectDone;
  const ready = new Promise((resolve, reject) => { resolveReady = resolve; rejectReady = reject; });
  const done = new Promise((resolve, reject) => { resolveDone = resolve; rejectDone = reject; });
  done.catch(() => {}); // A READY failure can precede the caller's completion wait.
  const events = []; let original = null, completed = false, error = null, stderr = '';
  const timer = setTimeout(() => {
    error = Error('Original shell/native handle readiness deadline; no caption dispatched');
    rejectReady(error);
  }, 15000);
  helper.stderr.on('data', bytes => { stderr = (stderr + bytes).slice(-8192); });
  const lines = createInterface({ input: helper.stdout });
  lines.on('line', line => {
    if (error) return;
    try {
      const item = JSON.parse(line);
      if (item.kind === 'READY' && !original &&
          item.shell?.pid === shellPid && item.native?.pid > 0 && item.native.pid !== shellPid &&
          item.shell?.image?.sha256 === config.installedSha256['gogoke.exe'] &&
          item.native?.image?.sha256 === config.installedSha256['gogoke-native-host.exe'] &&
          item.shell?.image?.fileId && item.native?.image?.fileId &&
          item.shell?.creationUtc && item.native?.creationUtc &&
          Array.isArray(item.nativeAncestorChain) &&
          item.nativeAncestorChain.at(-1)?.pid === shellPid) {
        original = item; clearTimeout(timer); resolveReady(item);
      } else if (item.kind === 'EXIT' && original && !completed &&
          ['shell', 'native'].includes(item.role) && events.every(event => event.role !== item.role) &&
          item.pid === original[item.role].pid && item.creationUtc === original[item.role].creationUtc &&
          typeof item.observedUtc === 'string' && Number.isSafeInteger(item.code) && item.code >= 0 &&
          item.codeBasis === 'GET_EXIT_CODE_PROCESS') {
        events.push(item);
      } else if (item.kind === 'COMPLETE' && original && events.length === 2 &&
                 item.installedCandidateProcesses === 0) {
        completed = true;
      } else {
        throw Error('Original candidate close observer frame differs');
      }
    } catch (cause) {
      error = cause; clearTimeout(timer); rejectReady(cause);
    }
  });
  helper.once('error', cause => { error = cause; clearTimeout(timer); rejectReady(cause); rejectDone(cause); });
  helper.once('close', code => {
    clearTimeout(timer);
    if (!original) rejectReady(error ?? Error(`Original candidate close observer exited before handles: ${code}; ${stderr}`));
    if (code !== 0 || error || !completed || events.length !== 2) {
      rejectDone(error ?? Error(`Original shell/native close event incomplete: helper=${code}; ${stderr}`));
    } else resolveDone({ ready: original, events, completed: true });
  });
  return { ready, done, events };
}

export async function observeInstalledReadiness(product, ready, port, setId, index) {
  lifecycleSource();
  return (async function () {
    const measuredDeadlineStartUtc = new Date().toISOString();
    const deadline = Date.now() + 180000;
    const timing = { spawnAttemptUtc: this.journal.productSpawnAttemptUtc ?? null,
      spawnReturnedUtc: this.journal.productSpawnReturnedUtc ?? null, measuredDeadlineStartUtc,
      deadlineAtUtc: new Date(deadline).toISOString(), readyPath: ready,
      decisionAtUtc: null, readyPresentAtDecision: null, readyAcceptedBeforeDeadline: null };
    this.journal.readinessObservations ??= [];
    this.journal.readinessObservations.push(timing);
    this.journal.readinessTiming = timing; this.save();
    let target;
    let readyObservedBeforeDeadline = false;
    let diagnosticConnected = false;
    let startupConsoleFailure = null;
    const failOriginalStartup = () => {
      if (!startupConsoleFailure) return;
      const original = startupConsoleFailure.args.slice(1)
        .map(arg => arg.value ?? arg.description ?? arg.type).join('\n');
      throw Error(`Actual product startup failed: ${original}`);
    };
    while (Date.now() < deadline) {
      failOriginalStartup();
      if (this.childError || this.child.exitCode !== null) throw Error(`Installed product launch failed: ${this.childError ?? this.child.exitCode}; ${this.stderr}`);
      // Read-only readiness sampling. A launch or mutation is never retried.
      try {
        const response = await fetch(`http://127.0.0.1:${port}/json/list`, { signal: AbortSignal.timeout(2000) });
        if (response.ok) {
          this.lastReadinessError = null;
          target = (await response.json()).find(page => {
          if (page.type !== 'page') return false;
          const url = new URL(page.url);
          return ['http:', 'https:', 'gogoke-resource:'].includes(url.protocol) &&
            ['gogoke-resource.localhost', 'localhost'].includes(url.hostname) &&
            url.pathname === `/${setId}/index.html`;
          });
        }
      } catch (error) {
        this.lastReadinessError = JSON.stringify({ name: error.name, message: error.message,
          cause: error.cause && { name: error.cause.name, message: error.cause.message,
            code: error.cause.code, address: error.cause.address, port: error.cause.port } });
      }
      if (target && !diagnosticConnected) {
        this.endpoint.url = target.url;
        await this.custody();
        await this.connectDiagnostic(target);
        diagnosticConnected = true;
        this.socket.addEventListener('message', event => {
          const message = JSON.parse(event.data);
          if (message.method !== 'Runtime.consoleAPICalled' ||
              message.params?.args?.[0]?.value !== 'Failed to signal gogoke update readiness') return;
          // Capture the producer's buffered startup error. Never replay readiness.
          startupConsoleFailure = message.params;
          this.journal.startupConsoleFailure = { pid: this.child.pid, ...message.params };
          this.save();
        });
        this.socket.send(JSON.stringify({ id: ++this.sequence, method: 'Runtime.enable' }));
      }
      failOriginalStartup();
      if (target && fs.existsSync(ready)) {
        readyObservedBeforeDeadline = Date.now() < deadline;
        break;
      }
      await delay(200);
    }
    const readyPresentAtDecision = fs.existsSync(ready);
    if (!target || !readyObservedBeforeDeadline) {
      this.journal.readinessTiming.decisionAtUtc = new Date().toISOString();
      this.journal.readinessTiming.readyPresentAtDecision = readyPresentAtDecision; this.save();
      this.journal.readinessTiming.readyAcceptedBeforeDeadline = false; this.save();
      this.journal.startupReadiness = {
        targetUrl: target?.url ?? null,
        diagnosticConnected,
        readyReceiptPresent: readyPresentAtDecision,
        readyAcceptedBeforeDeadline: false,
        lastTransportError: this.lastReadinessError ?? null,
      };
      if (diagnosticConnected) {
        try {
          this.journal.startupUi = await this.evaluate('({url:location.href,home:!!document.querySelector(".home-product-entry"),conversation:document.querySelectorAll(".composer").length===1,tauri:!!window.__TAURI_INTERNALS__})');
        } catch (error) {
          this.journal.startupUiError = String(error?.stack ?? error);
        }
      }
      this.journal.startupStderr = this.stderr; this.save();
      throw Error(`Actual product readiness missing: ${JSON.stringify(this.journal.startupReadiness)}; original stderr=${this.stderr}`);
    }
    this.journal.readinessTiming.readyDetectedAtUtc = new Date().toISOString(); this.save();
    const receipt = readJson(ready);
    if (receipt.version !== this.config.version || receipt.setId !== setId || receipt.generationId !== index.generationId) {
      throw Error('Installed bootstrap byte identity differs');
    }
    this.endpoint.url = target.url;
    await this.custody();
    const ui = await this.evaluate('({url:location.href,home:!!document.querySelector(".home-product-entry"),conversation:document.querySelectorAll(".composer").length===1,tauri:!!window.__TAURI_INTERNALS__})');
    failOriginalStartup();
    if (ui.url !== target.url || (!ui.home && !ui.conversation) || !ui.tauri) throw Error('Actual Home/conversation User bridge is absent');
    this.journal.readinessTiming.decisionAtUtc = new Date().toISOString();
    this.journal.readinessTiming.readyPresentAtDecision = true; this.save();
    this.journal.readinessTiming.readyAcceptedBeforeDeadline = true; this.save();
    return { target, receipt, ui };
  }).call(product);
}

async function closeNormallyImpl() {
    if (!this.socket || this.socket.readyState !== WebSocket.OPEN) throw Error('Original caption requires the proven live product socket; no close dispatched');
    await this.custody();
    const observer = observeInstalledClose(this.config, this.endpoint.pid);
    const original = await observer.ready;
    if (this.child.exitCode !== null || this.child.signalCode !== null || observer.events.length) {
      throw Error('Original shell/native exited before the caption close');
    }
    this.journal.nativeCloseReadiness ??= [];
    this.journal.nativeCloseReadiness.push(original); this.save();
    const exited = new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(Error('Original caption close did not exit; no force kill')), 30000);
      this.child.once('exit', code => { clearTimeout(timer); resolve(code); });
    });
    // A dispatch error still leaves the original bounded exit observer handled.
    exited.catch(() => {});
    const marker = id('captionClick');
    let resolveClick, rejectClick;
    const clickObserved = new Promise((resolve, reject) => { resolveClick = resolve; rejectClick = reject; });
    clickObserved.catch(() => {});
    const clickTimer = setTimeout(() => rejectClick(Error('Actual caption click event was not observed')), 15000);
    const onClickMessage = event => {
      try {
        const message = JSON.parse(event.data);
        if (message.method !== 'Runtime.consoleAPICalled' || message.params?.args?.[0]?.value !== marker) return;
        const clickedAt = message.params.args[1]?.value;
        if (!Number.isSafeInteger(clickedAt) || clickedAt <= 0) throw Error('Actual caption click timestamp absent');
        resolveClick({ clickedUtc: new Date(clickedAt).toISOString(), observedUtc: new Date().toISOString(),
          marker, basis: 'ACTUAL_CAPTION_CLICK_EVENT' });
      } catch (error) { rejectClick(error); }
    };
    this.socket.addEventListener('message', onClickMessage);
    let clicked;
    try {
      const dispatch = this.evaluate(`(()=>{const b=document.querySelector('.window-caption-control-close');if(!b)throw Error('Actual caption close absent');const marker=${JSON.stringify(marker)};b.addEventListener('click',event=>{if(!event.isTrusted)console.info(marker,Date.now())},{capture:true,once:true});setTimeout(()=>b.click(),100);return 'CAPTION_CLOSE_SCHEDULED'})()`);
      dispatch.catch(() => {}); // The actual click may close CDP before its evaluate reply.
      const first = await Promise.race([dispatch, clickObserved]);
      if (typeof first === 'string' && first !== 'CAPTION_CLOSE_SCHEDULED') {
        throw Error('Actual caption scheduling differed');
      }
      clicked = await clickObserved;
    } finally {
      clearTimeout(clickTimer);
      this.socket.removeEventListener('message', onClickMessage);
    }
    this.journal.captionClickEvidence ??= [];
    this.journal.captionClickEvidence.push(clicked); this.save();
    const [code, observed] = await Promise.all([exited, observer.done]);
    if (observed.events.some(event => Date.parse(event.observedUtc) < Date.parse(clicked.clickedUtc)) ||
        observed.events.some(event => !Number.isFinite(Date.parse(event.observedUtc)))) {
      throw Error('Original shell/native exit preceded the caption close');
    }
    const shell = observed.events.find(event => event.role === 'shell');
    const native = observed.events.find(event => event.role === 'native');
    if (!shell || !native || shell.code !== code) {
      throw Error('Original shell/native exit code or process binding differs');
    }
    this.journal.nativeCloseObservations ??= [];
    this.journal.nativeCloseObservations.push({ click: clicked, ready: observed.ready, events: observed.events }); this.save();
    if (native.code !== 0) throw Error(`Original native-host exit=${native.code}; no normal-close credit`);
    this.socket.close();
    if (this.tester) {
      await this.tester.dispose(); this.tester = null;
    }
    this.journal.closes.push({ pid: this.endpoint.pid, exitCode: code, forceKill: false,
      captionClick: clicked, nativeExit: native, shellExit: shell,
      nativeIdentity: observed.ready.native }); this.save();
    if (code !== 0) throw Error(`Actual product exit=${code}; ${this.stderr}`);
  }

export const closeInstalledNormally = product => { lifecycleSource(); return closeNormallyImpl.call(product); };

export function recordLateReadyAfterFailure(product) {
  const timing = product.journal.readinessTiming;
  if (timing?.readyAcceptedBeforeDeadline !== false) return;
  const readyPath = product.endpoint?.readyPath;
  const observation = { inspectedAtUtc: new Date().toISOString(), readyPath: readyPath ?? null,
    filePresent: !!readyPath && fs.existsSync(readyPath), state: 'UNKNOWN_OR_INCOMPLETE' };
  if (observation.filePresent) {
    try {
      const stat = fs.statSync(readyPath);
      const receipt = readJson(readyPath);
      observation.fileBirthUtc = stat.birthtime.toISOString();
      observation.fileWriteUtc = stat.mtime.toISOString();
      observation.exactReceipt = receipt.version === product.config.version &&
        receipt.setId === product.endpoint.setId &&
        receipt.generationId === product.endpoint.generationId;
      if (observation.exactReceipt && Number.isFinite(Date.parse(timing.decisionAtUtc)) &&
          stat.birthtimeMs > Date.parse(timing.decisionAtUtc)) {
        observation.state = 'LATE_VALID_RECEIPT_ORIGINAL_FAILURE';
      }
    } catch (error) { observation.readError = String(error); }
  }
  product.journal.readinessLateObservations ??= [];
  product.journal.readinessLateObservations.push(observation);
  product.save();
}

// Read-only classification of retained original evidence. A later receipt cannot
// turn an already-failed readiness decision into an accepted launch.
export function classifyRecordedReadiness({ journal, readyReceipt, readyCreatedUtc, failureRecordedUtc,
  spawnBoundaryUtc }) {
  lifecycleSource();
  const endpoint = journal?.currentEndpoint;
  const exactReceipt = !!endpoint && readyReceipt?.setId === endpoint.setId &&
    readyReceipt?.generationId === endpoint.generationId &&
    readyReceipt?.version === journal.candidateVersion;
  const accepted = journal?.launches?.some(row => row?.pid === endpoint?.pid &&
    row?.readyPath === endpoint?.readyPath && row?.bootstrap?.setId === endpoint?.setId &&
    row?.bootstrap?.generationId === endpoint?.generationId &&
    row?.bootstrap?.version === journal.candidateVersion) &&
    journal.startupReadiness?.readyReceiptPresent !== false &&
    journal.readinessTiming?.readyAcceptedBeforeDeadline !== false;
  const failedMissing = journal?.state === 'FAIL_ORIGINAL_REQUESTS_RETAINED' &&
    (journal.startupReadiness?.readyReceiptPresent === false ||
     journal.startupReadiness?.readyAcceptedBeforeDeadline === false) &&
    journal.launches?.length === 0 && journal.operations?.length === 0;
  const time = value => typeof value === 'string' && Number.isFinite(Date.parse(value)) ? Date.parse(value) : null;
  const spawn = time(spawnBoundaryUtc), failure = time(failureRecordedUtc), created = time(readyCreatedUtc);
  const timing = { spawnBoundaryUtc: spawnBoundaryUtc ?? null, failureRecordedUtc: failureRecordedUtc ?? null,
    readyCreatedUtc: readyCreatedUtc ?? null, spawnBoundaryToReadyMs: spawn !== null && created !== null ? created - spawn : null,
    failureToReadyMs: failure !== null && created !== null ? created - failure : null };
  if (exactReceipt && accepted) return { state: 'ACCEPTED_ORIGINAL_LAUNCH', timing };
  if (exactReceipt && failedMissing && failure !== null && created !== null && created > failure) {
    return { state: 'LATE_VALID_RECEIPT_ORIGINAL_FAILURE', timing };
  }
  return { state: 'UNKNOWN_OR_INCOMPLETE', timing };
}

export function classifyRecordedClose(journal, index = 0) {
  lifecycleSource();
  const close = journal?.closes?.[index], observed = journal?.nativeCloseObservations?.[index];
  const click = journal?.captionClickEvidence?.[index];
  const shell = observed?.events?.find(row => row.role === 'shell');
  const native = observed?.events?.find(row => row.role === 'native');
  const identical = (actual, expected) => actual?.pid === expected?.pid &&
    actual?.creationUtc === expected?.creationUtc && actual?.image?.sha256 && actual?.image?.fileId;
  const sameImage = (actual, expected) => ['path', 'sha256', 'volumeSerial', 'fileId'].every(key =>
    typeof actual?.[key] === 'string' && actual[key] && actual[key] === expected?.[key]);
  const sameExit = (actual, expected) => ['kind', 'role', 'pid', 'creationUtc', 'observedUtc', 'code', 'codeBasis']
    .every(key => actual?.[key] !== undefined && actual[key] === expected?.[key]);
  const afterClick = row => Number.isFinite(Date.parse(row?.observedUtc)) &&
    Number.isFinite(Date.parse(click?.clickedUtc)) && Date.parse(row.observedUtc) >= Date.parse(click.clickedUtc);
  if (close?.exitCode === 0 && close.forceKill === false && click?.basis === 'ACTUAL_CAPTION_CLICK_EVENT' &&
      close.captionClick?.marker === click.marker &&
      close.pid === observed?.ready?.shell?.pid && close.pid === shell?.pid &&
      ['clickedUtc', 'observedUtc', 'marker', 'basis'].every(key => close.captionClick[key] === click[key]) &&
      identical(observed?.ready?.shell, close?.shellExit) &&
      identical(observed?.ready?.native, close?.nativeIdentity) &&
      sameImage(observed?.ready?.native?.image, close?.nativeIdentity?.image) &&
      observed?.ready?.shell?.image?.sha256 === journal?.candidateInstalledSha256?.['gogoke.exe'] &&
      observed?.ready?.native?.image?.sha256 === journal?.candidateInstalledSha256?.['gogoke-native-host.exe'] &&
      sameExit(shell, close.shellExit) && sameExit(native, close.nativeExit) &&
      shell?.pid === close?.shellExit?.pid && shell?.creationUtc === close?.shellExit?.creationUtc &&
      native?.pid === close?.nativeExit?.pid && native?.creationUtc === close?.nativeExit?.creationUtc &&
      shell?.code === 0 && native?.code === 0 &&
      shell?.codeBasis === 'GET_EXIT_CODE_PROCESS' && native?.codeBasis === 'GET_EXIT_CODE_PROCESS' &&
      afterClick(shell) && afterClick(native)) {
    return { state: 'OBSERVED_NORMAL_0_0', shellPid: shell.pid, nativePid: native.pid,
      captionMarker: click.marker };
  }
  return { state: 'UNKNOWN_OR_INCOMPLETE' };
}
