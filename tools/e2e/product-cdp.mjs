// Actual installed WebView and the same User ingress as the product UI.
// Authentication pages are never controlled. No fake bridge or vendor is used.
import fs from 'node:fs';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { createServer } from 'node:net';
import { createHash, randomUUID } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { observeInstalledReadiness, closeInstalledNormally, recordLateReadyAfterFailure, lifecycleSource } from './installed-lifecycle-observation.mjs';
import { isTemporaryHomeCreateRequest, assertSupportedTemporaryHomeOperation, exactAdmissionResubmissionReply } from './installed-temporary-home-lifecycle.mjs';
import { startupLifecycleSource } from './installed-startup-home-lifecycle.mjs';

export const readJson = file => JSON.parse(fs.readFileSync(file, 'utf8').replace(/^\uFEFF/, ''));
export const sha256 = file => createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const textSha256 = value => createHash('sha256').update(value, 'utf8').digest('hex');
export const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
export const id = prefix => `${prefix}_${randomUUID().replaceAll('-', '')}`;
const here = path.dirname(fileURLToPath(import.meta.url));

export class ActualProduct {
  constructor(config, journal) {
    this.config = config; this.journal = journal; this.sequence = 0;
    journal.driverBytes = Object.fromEntries([
      'product-cdp.mjs', 'candidate-close-events.ps1', 'e2e-webview.mjs', 'm1-win11.mjs', 'candidate-custody.ps1',
      'm1-readback.py', 'package-lock.json', 'installed-lifecycle-observation.mjs',
      'installed-temporary-home-lifecycle.mjs',
      'installed-startup-home-lifecycle.mjs', 'causal-close/causal-close.mjs',
      'causal-close/candidate-close-events.ps1',
    ].map(relative => [relative, sha256(path.join(here, relative))]));
    journal.driverBytes['installed-lifecycle-observation.mjs'] = lifecycleSource().sha256;
    journal.driverBytes['installed-startup-home-lifecycle.mjs'] = startupLifecycleSource().sha256;
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
    await this.temporaryHomeLifecycle?.beforeProductLaunch();
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
      this.journal.productSpawnAttemptUtc = new Date().toISOString(); this.save();
      this.child = spawn(path.join(this.config.installed, 'gogoke.exe'), [`--gogoke-update-ready=${ready}`], {
        cwd: this.config.installed, windowsHide: true, detached: true,
        stdio: ['ignore', 'ignore', stderrFd],
        env: { ...process.env, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:
          `--remote-debugging-port=${port} --remote-debugging-address=127.0.0.1` },
      });
    } finally { fs.closeSync(stderrFd); }
    this.journal.productSpawnReturnedUtc = new Date().toISOString(); this.save();
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
    const { target, receipt, ui } = await observeInstalledReadiness(this, ready, port, setId, index);
    this.journal.launches.push({ ...this.endpoint, sourceCommit: this.config.sourceCommit, bootstrap: receipt, ui }); this.save();
    if (this.config.testerArmy !== false) {
      const { connectWebView } = await import('./e2e-webview.mjs');
      this.tester = await connectWebView(this.endpoint, this.config.evidenceDirectory);
      this.journal.connectionBackend = { name: 'tester-army/e2e', telemetryDisabled: true, agentActs: 0 };
      this.save();
    }
    await this.temporaryHomeLifecycle?.afterProductLaunch();
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
  async attachRetained(binding = {}) {
    if (this.child || this.tester || this.socket) throw Error('Retained attach requires an unlaunched product object');
    const endpoint = binding.endpoint ?? this.endpoint ?? this.config.currentEndpoint;
    if (!endpoint || !Number.isSafeInteger(endpoint.pid) || !Number.isSafeInteger(endpoint.port) ||
        !path.isAbsolute(endpoint.readyPath) || !path.isAbsolute(endpoint.stderrPath) ||
        typeof endpoint.url !== 'string' || !endpoint.url) {
      throw Error('Retained attach endpoint binding is incomplete');
    }
    if (binding.provenance) {
      for (const [name, reference] of Object.entries(binding.provenance)) {
        if (!path.isAbsolute(reference?.path ?? '') || !/^[a-f0-9]{64}$/.test(reference.sha256) ||
            sha256(reference.path) !== reference.sha256) {
          throw Error(`Retained provenance changed: ${name}`);
        }
      }
    }
    if (binding.sourcePins) {
      for (const [name, reference] of Object.entries(binding.sourcePins)) {
        if (!path.isAbsolute(reference?.path ?? '') || !/^[a-f0-9]{64}$/.test(reference.sha256) ||
            sha256(reference.path) !== reference.sha256) {
          throw Error(`Retained source pin changed: ${name}`);
        }
      }
    }
    this.endpoint = endpoint;
    this.stderrFile = endpoint.stderrPath;
    this.verifyBytes();
    const resourceIndexPath = path.join(this.config.installed, 'resource-index.json');
    const resourceIndex = readJson(resourceIndexPath);
    const resourceSetId = sha256(resourceIndexPath);
    if (endpoint.setId !== resourceSetId || endpoint.generationId !== resourceIndex.generationId ||
        endpoint.sourceCommit && endpoint.sourceCommit !== this.config.sourceCommit) {
      throw Error('Retained endpoint byte identity differs from the installed product');
    }
    if (!fs.existsSync(endpoint.readyPath)) throw Error('Retained readiness receipt is absent');
    const ready = readJson(endpoint.readyPath);
    if (ready.version !== this.config.version || ready.setId !== endpoint.setId ||
        ready.generationId !== endpoint.generationId ||
        (binding.readySha256 && (!/^[a-f0-9]{64}$/.test(binding.readySha256) ||
          sha256(endpoint.readyPath) !== binding.readySha256))) {
      throw Error('Retained readiness receipt changed');
    }
    const resourceUrl = new URL(endpoint.url);
    if (!['http:', 'https:', 'gogoke-resource:'].includes(resourceUrl.protocol) ||
        !['gogoke-resource.localhost', 'localhost'].includes(resourceUrl.hostname) ||
        resourceUrl.pathname !== `/${endpoint.setId}/index.html`) {
      throw Error('Retained resource URL identity differs');
    }
    await this.custody();
    const response = await fetch(`http://127.0.0.1:${endpoint.port}/json/list`);
    if (!response.ok) throw Error('Retained diagnostic target list unavailable');
    const targets = (await response.json()).filter(row => row.type === 'page' && row.url === endpoint.url);
    if (targets.length !== 1) throw Error('Retained resource URL is not unique');
    const target = targets[0];
    await this.connectDiagnostic(target);
    this.socket.send(JSON.stringify({ id: ++this.sequence, method: 'Runtime.enable' }));
    const ui = await this.evaluate('({url:location.href,home:!!document.querySelector(".home-product-entry"),tauri:!!window.__TAURI_INTERNALS__})');
    if (ui.url !== endpoint.url || ui.home !== true || ui.tauri !== true) {
      throw Error('Retained Home/Tauri page identity differs');
    }
    this.journal.currentEndpoint = endpoint;
    this.journal.retainedAttachment = {
      observationMode: 'ATTACHED_RETAINED_PRODUCT', endpoint,
      resourceUrl: target.url, diagnosticType: target.type, ui,
      parentChildObservation: 'NOT_AVAILABLE_TO_CURRENT_OBSERVER',
      provenance: binding.provenance ?? null,
      sourcePins: binding.sourcePins ?? null,
    };
    this.save();
    return { endpoint, target, ui };
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
  async operation(family, operation, targetId, payload = {}, expectedRevision = '0', allowed = ['APPLIED'],
      domainId = this.config.domainId) {
    const request = { schema: 'gogoke.37.operations.v1', family, operation, requestId: id('e2e'),
      domainId, targetId, expectedRevision, payload };
    assertSupportedTemporaryHomeOperation(request);
    const rawFrame = JSON.stringify(request);
    const record = { request, rawFrame, startedAt: new Date().toISOString(), receipt: null };
    if ((isTemporaryHomeCreateRequest(request) ||
        (family === 'K-INSTANCE' && operation === 'temporary-home-close')) &&
        !this.temporaryHomeLifecycle) {
      throw Error('Temporary-HOME operation blocked: shared case lifecycle producer anchor is missing');
    }
    this.temporaryHomeLifecycle?.beforeDispatch(record);
    this.journal.operations.push(record); this.save();
    // Persist once before writing. UNKNOWN/disconnect/timeout stops; no implicit resend.
    let raw;
    try {
      raw = await this.evaluate(`window.__TAURI_INTERNALS__.invoke('gogoke_design37_user_operation',{frame:${JSON.stringify(rawFrame)}})`);
    } catch (error) {
      this.temporaryHomeLifecycle?.afterDispatchError(record, error);
      this.save();
      throw error;
    }
    record.rawReceipt = raw; this.save();
    record.receipt = JSON.parse(raw); record.finishedAt = new Date().toISOString(); this.save();
    if (record.receipt?.schema !== request.schema ||
        record.receipt?.requestId !== request.requestId || record.receipt?.targetId !== targetId ||
        record.receipt?.family !== family || record.receipt?.operation !== operation ||
        typeof record.receipt?.status !== 'string') {
      const error = Error(`Original ${family}/${operation} result=${record.receipt?.status}; inspect durable request, no replay`);
      this.temporaryHomeLifecycle?.afterDispatchError(record, error);
      this.save();
      throw error;
    }
    if (!allowed.includes(record.receipt.status)) {
      const error = Error(`Original ${family}/${operation} result=${record.receipt.status}; inspect durable request, no replay`);
      this.temporaryHomeLifecycle?.afterRejectedReceipt(record);
      this.save();
      throw error;
    }
    this.temporaryHomeLifecycle?.afterReceipt(record);
    this.save();
    return record.receipt;
  }
  async resubmitDeniedAdmission(originalRecord, continuation = {}) {
    const request = originalRecord?.request;
    if (!this.temporaryHomeLifecycle?.beforeAdmissionResubmission ||
        !this.temporaryHomeLifecycle?.afterAdmissionResubmission ||
        !this.temporaryHomeLifecycle?.afterAdmissionResubmissionError) {
      throw Error('Explicit denied admission resubmission lifecycle hook is missing');
    }
    if (!this.journal.operations?.includes(originalRecord) ||
        request?.family !== 'K-SESSION' || request.operation !== 'admission-reserve' ||
        originalRecord.rawFrame !== JSON.stringify(request) ||
        originalRecord.receipt?.status !== 'DENIED') {
      throw Error('Resubmission requires this exact original DENIED admission record');
    }
    if (continuation.rawSha256 && textSha256(originalRecord.rawFrame) !== continuation.rawSha256) {
      throw Error('Resubmission raw-frame binding differs');
    }
    if (this.journal.operations.some(row => row.resubmissionOf === request.requestId)) {
      throw Error('Original DENIED admission already has a resubmission; no third dispatch');
    }
    const record = {
      kind: 'DENIED_ADMISSION_RESUBMISSION', resubmissionOf: request.requestId,
      request, rawFrame: originalRecord.rawFrame,
      originalRawSha256: textSha256(originalRecord.rawFrame),
      continuation, startedAt: new Date().toISOString(), receipt: null,
    };
    this.temporaryHomeLifecycle.beforeAdmissionResubmission(originalRecord, record, continuation);
    this.journal.operations.push(record); this.save();
    let receiptHooked = false;
    try {
      const raw = await this.evaluate(
        `window.__TAURI_INTERNALS__.invoke('gogoke_design37_user_operation',{frame:${JSON.stringify(record.rawFrame)}})`);
      record.rawReceipt = raw;
      record.receipt = JSON.parse(raw);
      record.finishedAt = new Date().toISOString();
      if (!exactAdmissionResubmissionReply(record)) {
        record.status = 'RECEIPT_INVALID';
        this.save();
        throw Error(`Denied admission resubmission identity differs: ${record.receipt?.status ?? 'UNKNOWN'}`);
      }
      record.status = record.receipt.status;
      this.temporaryHomeLifecycle.afterAdmissionResubmission(record);
      receiptHooked = true;
      this.save();
      if (record.receipt.status !== 'APPLIED') {
        throw Error(`Denied admission resubmission result=${record.receipt.status}; no replay`);
      }
      return record.receipt;
    } catch (error) {
      if (!record.finishedAt) record.finishedAt = new Date().toISOString();
      record.originalError = String(error?.stack ?? error);
      if (!receiptHooked) this.temporaryHomeLifecycle.afterAdmissionResubmissionError(record, error);
      this.save();
      throw error;
    }
  }
  async reconcile(record) {
    assertSupportedTemporaryHomeOperation(record?.request);
    if (record.receipt?.status !== 'UNKNOWN' || record.request.family !== 'K-SESSION' ||
        !['compact', 'renew-session', 'resume'].includes(record.request.operation) ||
        typeof record.rawFrame !== 'string' || record.rawFrame !== JSON.stringify(record.request)) {
      throw Error('Reconciliation requires the explicit original generation-change UNKNOWN receipt');
    }
    if (!this.temporaryHomeLifecycle) {
      throw Error('Temporary-HOME reconciliation blocked: shared case lifecycle producer anchor is missing');
    }
    this.temporaryHomeLifecycle.beforeReconciliation(record);
    const observation = { request: record.request, rawFrame: record.rawFrame,
      basis: 'EXACT_NATIVE_REQUEST_RECONCILIATION_NOT_NEW_VENDOR_COMMAND', receipt: null };
    this.journal.operations.push(observation); this.save();
    const raw = await this.evaluate(`window.__TAURI_INTERNALS__.invoke('gogoke_design37_user_operation',{frame:${JSON.stringify(record.rawFrame)}})`);
    observation.receipt = JSON.parse(raw); this.save();
    if (observation.receipt?.requestId !== record.request.requestId ||
        observation.receipt?.targetId !== record.request.targetId ||
        observation.receipt?.family !== record.request.family ||
        observation.receipt?.operation !== record.request.operation ||
        !['APPLIED', 'REPLAYED', 'UNKNOWN'].includes(observation.receipt?.status)) {
      const error = Error(`Original native reconciliation returned ${observation.receipt?.status}`);
      this.temporaryHomeLifecycle?.afterDispatchError(record, error);
      this.save();
      throw error;
    }
    this.temporaryHomeLifecycle?.afterReconciliation(record, observation);
    this.save();
    return observation;
  }
  async seat() {
    let receipt = await this.operation('K-SEAT', 'state-card', this.config.seatId, {}, '0', ['APPLIED', 'STALE']);
    if (receipt.status === 'STALE') receipt = await this.operation('K-SEAT', 'state-card', this.config.seatId, {}, receipt.revision);
    return receipt;
  }
  async closeNormally() {
    if (this.temporaryHomeLifecycle) {
      try {
        const teardown = await this.temporaryHomeLifecycle.beforeCaptionClose();
        if (!teardown.complete) throw Error('Temporary-HOME teardown incomplete: ' + teardown.errors.join('; '));
      }
      catch (error) {
        this.journal.temporaryHomeLifecycle = {
          manifest: this.temporaryHomeLifecycle.manifestPath,
          state: 'CASE_TEARDOWN_INCOMPLETE',
          error: String(error?.stack ?? error),
        };
        this.save();
        // Keep the original USER pipe available. Caption shutdown can itself
        // stop the resident generation; it must not follow an unresolved stop.
        throw error;
      }
    }
    return closeInstalledNormally(this);
  }
  async preserveFailure() {
    // Retain the original task and child handle until Controller settles and
    // normally closes the real product. No kill, input replay or OS stop claim.
    recordLateReadyAfterFailure(this);
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
    recordLateReadyAfterFailure(this);
    this.socket?.close();
    if (this.tester) { await this.tester.dispose(); this.tester = null; }
  }
}
