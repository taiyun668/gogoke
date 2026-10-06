// Actual installed WebView and the same User ingress as the product UI.
// Authentication pages are never controlled. No fake bridge or vendor is used.
import fs from 'node:fs';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { createServer } from 'node:net';
import { createHash, randomUUID } from 'node:crypto';
import { fileURLToPath } from 'node:url';

export const readJson = file => JSON.parse(fs.readFileSync(file, 'utf8').replace(/^\uFEFF/, ''));
export const sha256 = file => createHash('sha256').update(fs.readFileSync(file)).digest('hex');
export const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
export const id = prefix => `${prefix}_${randomUUID().replaceAll('-', '')}`;
const here = path.dirname(fileURLToPath(import.meta.url));

export class ActualProduct {
  constructor(config, journal) {
    this.config = config; this.journal = journal; this.sequence = 0;
    journal.driverBytes = Object.fromEntries([
      'product-cdp.mjs', 'e2e-webview.mjs', 'm1-win11.mjs', 'candidate-custody.ps1',
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
    const deadline = Date.now() + 60000;
    let target;
    while (Date.now() < deadline) {
      if (this.childError || this.child.exitCode !== null) throw Error(`Installed product launch failed: ${this.childError ?? this.child.exitCode}; ${this.stderr}`);
      // Read-only readiness sampling. A launch or mutation is never retried.
      try {
        const response = await fetch(`http://127.0.0.1:${port}/json/list`, { signal: AbortSignal.timeout(2000) });
        if (response.ok) target = (await response.json()).find(page => {
          if (page.type !== 'page') return false;
          const url = new URL(page.url);
          return ['http:', 'https:', 'gogoke-resource:'].includes(url.protocol) &&
            ['gogoke-resource.localhost', 'localhost'].includes(url.hostname) &&
            url.pathname === `/${setId}/index.html`;
        });
      } catch (error) { this.lastReadinessError = String(error); }
      if (target && fs.existsSync(ready)) break;
      await delay(200);
    }
    if (!target || !fs.existsSync(ready)) {
      this.journal.startupStderr = this.stderr; this.save();
      throw Error(`Actual product readiness missing: ${this.lastReadinessError ?? 'no target or ready receipt'}; original stderr=${this.stderr}`);
    }
    const receipt = readJson(ready);
    if (receipt.version !== this.config.version || receipt.setId !== setId || receipt.generationId !== index.generationId) {
      throw Error('Installed bootstrap byte identity differs');
    }
    this.endpoint.url = target.url;
    await this.custody();
    const socketUrl = new URL(target.webSocketDebuggerUrl);
    if (socketUrl.protocol !== 'ws:' || !['127.0.0.1', 'localhost'].includes(socketUrl.hostname) || Number(socketUrl.port) !== port) {
      throw Error('Unexpected diagnostic endpoint');
    }
    this.socket = new WebSocket(socketUrl);
    await new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(Error('Diagnostic connection deadline')), 5000);
      this.socket.addEventListener('open', () => { clearTimeout(timer); resolve(); }, { once: true });
      this.socket.addEventListener('error', () => { clearTimeout(timer); reject(Error('Diagnostic connection failed')); }, { once: true });
    });
    const ui = await this.evaluate('({url:location.href,home:!!document.querySelector(".home-product-entry"),tauri:!!window.__TAURI_INTERNALS__})');
    if (ui.url !== target.url || !ui.home || !ui.tauri) throw Error('Actual Home/User bridge is absent');
    this.journal.launches.push({ ...this.endpoint, sourceCommit: this.config.sourceCommit, bootstrap: receipt, ui }); this.save();
    if (this.config.testerArmy !== false) {
      const { connectWebView } = await import('./e2e-webview.mjs');
      this.tester = await connectWebView(this.endpoint, this.config.evidenceDirectory);
      this.journal.connectionBackend = { name: 'tester-army/e2e', telemetryDisabled: true, agentActs: 0 };
      this.save();
    }
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
    const exited = new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(Error('Original caption close did not exit; no force kill')), 30000);
      this.child.once('exit', code => { clearTimeout(timer); resolve(code); });
    });
    // A dispatch error still leaves the original bounded exit observer handled.
    exited.catch(() => {});
    const dispatched = await this.evaluate('(()=>{const b=document.querySelector(".window-caption-control-close");if(!b)throw Error("Actual caption close absent");setTimeout(()=>b.click(),100);return "CAPTION_CLOSE_DISPATCHED"})()');
    if (dispatched !== 'CAPTION_CLOSE_DISPATCHED') throw Error('Actual caption close was not dispatched');
    const code = await exited;
    this.socket.close();
    if (this.tester) {
      await this.tester.dispose(); this.tester = null;
    }
    this.journal.closes.push({ pid: this.endpoint.pid, exitCode: code, forceKill: false }); this.save();
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
    this.journal.failedProductHeld = false; this.save();
    this.socket?.close();
    if (this.tester) { await this.tester.dispose(); this.tester = null; }
  }
}
