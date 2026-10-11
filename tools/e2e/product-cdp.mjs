// Actual installed WebView and the same User ingress as the product UI.
// Authentication pages are never controlled. No fake bridge or vendor is used.
import fs from 'node:fs';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { createServer } from 'node:net';
import { createHash, randomUUID } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { createInterface } from 'node:readline';

export const readJson = file => JSON.parse(fs.readFileSync(file, 'utf8').replace(/^\uFEFF/, ''));
export const sha256 = file => createHash('sha256').update(fs.readFileSync(file)).digest('hex');
export const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
export const id = prefix => `${prefix}_${randomUUID().replaceAll('-', '')}`;
const here = path.dirname(fileURLToPath(import.meta.url));

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

export class ActualProduct {
  constructor(config, journal) {
    this.config = config; this.journal = journal; this.sequence = 0;
    journal.driverBytes = Object.fromEntries([
      'product-cdp.mjs', 'candidate-close-events.ps1', 'e2e-webview.mjs', 'm1-win11.mjs', 'candidate-custody.ps1',
      'm1-readback.py', 'package-lock.json',
    ].map(relative => [relative, sha256(path.join(here, relative))]));
    journal.nodeVersion = process.version;
  }
  save() {
    const temporary = `${this.config.result}.tmp`;
    fs.writeFileSync(temporary, JSON.stringify(this.journal, null, 2) + '\n');
    fs.renameSync(temporary, this.config.result);
    // Observe a live run through its append-only stdout log. Windows can
    // refuse replacement of an open journal target even with DELETE sharing.
    const progress = JSON.stringify({ state: this.journal.state,
      operation: this.journal.operations?.at(-1)?.request?.operation ?? null,
      sessions: this.journal.sessions?.map(({ generation, turns }) => ({
        generation, turns: turns?.length ?? 0,
      })) ?? [], closes: this.journal.closes?.length ?? 0 });
    if (progress !== this.lastProgress) { console.log(progress); this.lastProgress = progress; }
  }
  get stderr() {
    if (!this.stderrFile) return '';
    const fd = fs.openSync(this.stderrFile, 'r');
    try {
      const length = fs.fstatSync(fd).size;
      const tail = Buffer.alloc(Math.min(length, 8192));
      fs.readSync(fd, tail, 0, tail.length, length - tail.length);
      return tail.toString('utf8');
    } finally { fs.closeSync(fd); }
  }
  async custody(before = false) {
    const c = this.config;
    const args = ['-NoProfile', '-NonInteractive', '-File', path.join(here, 'candidate-custody.ps1'),
      '-Installed', c.installed, '-Version', c.version, '-RegistryKey', c.registryKey];
    if (before) args.push('-BeforeLaunch');
    else args.push('-ExpectedPid', String(this.endpoint.pid), '-Port', String(this.endpoint.port));
    await new Promise((resolve, reject) => {
      const child = spawn(c.pwsh, args, { windowsHide: true, stdio: ['ignore', 'ignore', 'pipe'] });
      let tail = '';
      child.stderr.on('data', bytes => { tail = (tail + bytes).slice(-8192); });
      child.once('error', reject);
      child.once('exit', code => code === 0 ? resolve() : reject(Error(`Candidate custody exit=${code}: ${tail}`)));
    });
  }
  verifyBytes() {
    for (const [relative, expected] of Object.entries(this.config.installedSha256)) {
      const absolute = path.resolve(this.config.installed, relative);
      if (!absolute.startsWith(path.resolve(this.config.installed) + path.sep) ||
          !/^[a-f0-9]{64}$/.test(expected) || sha256(absolute) !== expected) {
        throw Error(`Actual installed bytes differ: ${relative}`);
      }
    }
    for (const required of ['gogoke.exe', 'gogoke-native-host.exe', 'resource-index.json']) {
      if (!Object.hasOwn(this.config.installedSha256, required)) throw Error(`Missing byte binding: ${required}`);
    }
    const index = readJson(path.join(this.config.installed, 'resource-index.json'));
    if (index.sourceCommit !== this.config.sourceCommit || index.version !== this.config.version) {
      throw Error('Actual installed source/version differs from the private fixture');
    }
  }
  async launch() {
    await this.custody(true); this.verifyBytes();
    const index = readJson(path.join(this.config.installed, 'resource-index.json'));
    const setId = sha256(path.join(this.config.installed, 'resource-index.json'));
    const listener = createServer();
    await new Promise((resolve, reject) => { listener.once('error', reject); listener.listen(0, '127.0.0.1', resolve); });
    const port = listener.address().port;
    await new Promise(resolve => listener.close(resolve));
    // Reuse the installed-product handshake namespace accepted by the product.
    const ready = path.join(process.env.TMP, `gogoke-update-${randomUUID().replaceAll('-', '')}.ready`);
    // Windows libuv adds non-detached children to a kill-on-parent-close Job.
    // The actual product's custody must not depend on the test driver's life.
    this.stderrFile = path.join(this.config.evidenceDirectory, `${id('product-stderr')}.log`);
    const stderrFd = fs.openSync(this.stderrFile, 'wx');
    try {
      this.child = spawn(path.join(this.config.installed, 'gogoke.exe'), [`--gogoke-update-ready=${ready}`], {
        cwd: this.config.installed, windowsHide: true, detached: true,
        stdio: ['ignore', 'ignore', stderrFd],
        env: { ...process.env, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:
          `--remote-debugging-port=${port} --remote-debugging-address=127.0.0.1` },
      });
    } finally { fs.closeSync(stderrFd); }
    this.childError = null;
    this.child.once('error', error => { this.childError = error; });
    const child = this.child;
    this.child.once('exit', (code, signal) => {
      this.journal.productExits ??= [];
      this.journal.productExits.push({ pid: child.pid, code, signal,
        observedAt: new Date().toISOString(), stderrTail: this.stderr });
      this.save();
    });
    this.endpoint = { pid: this.child.pid, port, setId, generationId: index.generationId,
      readyPath: ready, stderrPath: this.stderrFile };
    this.journal.currentEndpoint = this.endpoint; this.save();
    // The real cold bootstrap receipt arrived at 70 s on Win11. Use the
    // existing 3-minute product-operation bound for readiness as well;
    // this observes the one launch and never repeats a mutation.
    const deadline = Date.now() + 180000;
    let target;
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
          this.journal.startupConsoleFailure = { pid: child.pid, ...message.params };
          this.save();
        });
        this.socket.send(JSON.stringify({ id: ++this.sequence, method: 'Runtime.enable' }));
      }
      failOriginalStartup();
      if (target && fs.existsSync(ready)) break;
      await delay(200);
    }
    if (!target || !fs.existsSync(ready)) {
      this.journal.startupReadiness = {
        targetUrl: target?.url ?? null,
        diagnosticConnected,
        readyReceiptPresent: fs.existsSync(ready),
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
    const receipt = readJson(ready);
    if (receipt.version !== this.config.version || receipt.setId !== setId || receipt.generationId !== index.generationId) {
      throw Error('Installed bootstrap byte identity differs');
    }
    this.endpoint.url = target.url;
    await this.custody();
    const ui = await this.evaluate('({url:location.href,home:!!document.querySelector(".home-product-entry"),conversation:document.querySelectorAll(".composer").length===1,tauri:!!window.__TAURI_INTERNALS__})');
    failOriginalStartup();
    if (ui.url !== target.url || (!ui.home && !ui.conversation) || !ui.tauri) throw Error('Actual Home/conversation User bridge is absent');
    this.journal.launches.push({ ...this.endpoint, sourceCommit: this.config.sourceCommit, bootstrap: receipt, ui }); this.save();
    if (this.config.testerArmy !== false) {
      const { connectWebView } = await import('./e2e-webview.mjs');
      this.tester = await connectWebView(this.endpoint, this.config.evidenceDirectory);
      this.journal.connectionBackend = { name: 'tester-army/e2e', telemetryDisabled: true, agentActs: 0 };
      this.save();
    }
  }
  async connectDiagnostic(target) {
    const socketUrl = new URL(target.webSocketDebuggerUrl);
    if (socketUrl.protocol !== 'ws:' || !['127.0.0.1', 'localhost'].includes(socketUrl.hostname) || Number(socketUrl.port) !== this.endpoint.port) {
      throw Error('Unexpected diagnostic endpoint');
    }
    this.socket = new WebSocket(socketUrl);
    await new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(Error('Diagnostic connection deadline')), 5000);
      this.socket.addEventListener('open', () => { clearTimeout(timer); resolve(); }, { once: true });
      this.socket.addEventListener('error', () => { clearTimeout(timer); reject(Error('Diagnostic connection failed')); }, { once: true });
    });
  }
  evaluate(expression) {
    if (this.tester) {
      if (this.tester.page.url() !== this.endpoint.url) throw Error('Actual e2e page identity changed; no operation dispatched');
      return new Promise((resolve, reject) => {
        const timer = setTimeout(() => reject(Error('Product operation deadline; original request retained, no resend')), 180000);
        this.tester.page.evaluate(expression).then(
          value => { clearTimeout(timer); resolve(value); },
          error => { clearTimeout(timer); reject(error); },
        );
      });
    }
    return new Promise((resolve, reject) => {
      const requestId = ++this.sequence;
      const socket = this.socket;
      const finish = (error, value) => {
        clearTimeout(timer); socket.removeEventListener('message', received); socket.removeEventListener('close', closed);
        error ? reject(error) : resolve(value);
      };
      const closed = () => finish(Error('Actual product connection closed; original request retained, no resend'));
      const received = event => {
        const message = JSON.parse(event.data);
        if (message.id !== requestId) return;
        if (message.error || message.result?.exceptionDetails) finish(Error(JSON.stringify(message.error ?? message.result.exceptionDetails)));
        else finish(null, message.result?.result?.value);
      };
      const timer = setTimeout(() => finish(Error('Product operation deadline; no mutation replay')), 180000);
      socket.addEventListener('message', received); socket.addEventListener('close', closed);
      socket.send(JSON.stringify({ id: requestId, method: 'Runtime.evaluate', params: { expression, returnByValue: true, awaitPromise: true } }));
    });
  }
  async instances() {
    // Only status is exported. No authorization URL, code, output or account is captured here.
    return this.evaluate("window.__TAURI_INTERNALS__.invoke('gogoke_design37_instances').then(p=>({schema:p.schema,instances:p.instances.map(r=>({instanceId:r.instanceId,driverId:r.driverId,version:r.version,revision:r.revision,state:r.state}))}))");
  }
  async operation(family, operation, targetId, payload = {}, expectedRevision = '0', allowed = ['APPLIED']) {
    const request = { schema: 'gogoke.37.operations.v1', family, operation, requestId: id('e2e'),
      domainId: this.config.domainId, targetId, expectedRevision, payload };
    const rawFrame = JSON.stringify(request);
    const record = { request, rawFrame, startedAt: new Date().toISOString(), receipt: null };
    this.journal.operations.push(record); this.save();
    // Persist once before writing. UNKNOWN/disconnect/timeout stops; no implicit resend.
    const raw = await this.evaluate(`window.__TAURI_INTERNALS__.invoke('gogoke_design37_user_operation',{frame:${JSON.stringify(rawFrame)}})`);
    record.receipt = JSON.parse(raw); record.finishedAt = new Date().toISOString(); this.save();
    if (record.receipt.requestId !== request.requestId || record.receipt.targetId !== targetId ||
        record.receipt.family !== family || record.receipt.operation !== operation || !allowed.includes(record.receipt.status)) {
      throw Error(`Original ${family}/${operation} result=${record.receipt.status}; inspect durable request, no replay`);
    }
    return record.receipt;
  }
  async reconcile(record) {
    if (record.receipt?.status !== 'UNKNOWN' || record.request.family !== 'K-SESSION' ||
        !['compact', 'renew-session', 'resume'].includes(record.request.operation) ||
        typeof record.rawFrame !== 'string' || record.rawFrame !== JSON.stringify(record.request)) {
      throw Error('Reconciliation requires the explicit original generation-change UNKNOWN receipt');
    }
    const observation = { request: record.request, rawFrame: record.rawFrame,
      basis: 'EXACT_NATIVE_REQUEST_RECONCILIATION_NOT_NEW_VENDOR_COMMAND', receipt: null };
    this.journal.operations.push(observation); this.save();
    const raw = await this.evaluate(`window.__TAURI_INTERNALS__.invoke('gogoke_design37_user_operation',{frame:${JSON.stringify(record.rawFrame)}})`);
    observation.receipt = JSON.parse(raw); this.save();
    if (observation.receipt.requestId !== record.request.requestId ||
        !['APPLIED', 'REPLAYED', 'UNKNOWN'].includes(observation.receipt.status)) {
      throw Error(`Original native reconciliation returned ${observation.receipt.status}`);
    }
    return observation;
  }
  async seat() {
    let receipt = await this.operation('K-SEAT', 'state-card', this.config.seatId, {}, '0', ['APPLIED', 'STALE']);
    if (receipt.status === 'STALE') receipt = await this.operation('K-SEAT', 'state-card', this.config.seatId, {}, receipt.revision);
    return receipt;
  }
  async closeNormally() {
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
  async preserveFailure() {
    // Retain the original task and child handle until Controller settles and
    // normally closes the real product. No kill, input replay or OS stop claim.
    this.journal.failedProductHeld = Boolean(this.child && this.child.exitCode === null &&
      this.child.signalCode === null && !this.childError);
    this.save();
    if (this.journal.failedProductHeld) {
      await new Promise(resolve => {
        this.child.once('exit', resolve); this.child.once('error', resolve);
      });
    }
    this.journal.failedProductExit = {
      exitCode: this.child?.exitCode ?? null,
      signal: this.child?.signalCode ?? null,
      originalError: this.childError ? String(this.childError) : null,
    };
    this.journal.failedProductHeld = false; this.save();
    this.socket?.close();
    if (this.tester) { await this.tester.dispose(); this.tester = null; }
  }
}
