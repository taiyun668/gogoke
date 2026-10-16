// Close an already-held failed launch through the shared normal-close instrument.
// The original driver remains the source of its own child exit event.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { createHash } from 'node:crypto';

const digest = p => createHash('sha256').update(fs.readFileSync(p)).digest('hex');
const read = p => JSON.parse(fs.readFileSync(p, 'utf8').replace(/^\uFEFF/, ''));
const need = (x, why) => { if (!x) throw Error(why); };
const pin = r => need(r && path.isAbsolute(r.path) && digest(r.path) === r.sha256, 'Retained close source changed');
const inputPath = process.argv[2];
need(inputPath && path.isAbsolute(inputPath), 'Pinned close input required');
const input = read(inputPath);
need(input.schema === 'gogoke.37.retained-installed-close-input.v1' && input.acceptance === false &&
  input.actorSha256 === digest(fileURLToPath(import.meta.url)), 'Retained close input differs');
for (const name of ['config', 'failureSnapshot', 'productCdp', 'lifecycleModule']) pin(input[name]);
const config = read(input.config.path), failed = read(input.failureSnapshot.path);
need(failed.state === 'FAIL_CONTINUATION_ORIGINAL_REQUESTS_RETAINED' && failed.failedProductHeld === true &&
  failed.launches.length === 1 && failed.closes.length === 0 &&
  failed.driverBytes['product-cdp.mjs'] === input.productCdp.sha256 &&
  failed.driverBytes['installed-lifecycle-observation.mjs'] === input.lifecycleModule.sha256 &&
  failed.candidateVersion === config.version && failed.sourceCommit === config.sourceCommit &&
  failed.stateRoot === config.stateRoot && JSON.stringify(failed.candidateInstalledSha256) === JSON.stringify(config.installedSha256),
  'Original held candidate source differs');
const endpoint = failed.launches[0];
need(Number.isSafeInteger(endpoint.pid) && endpoint.pid > 0 && Number.isSafeInteger(endpoint.port) &&
  endpoint.url === failed.currentEndpoint.url && endpoint.pid === failed.currentEndpoint.pid &&
  endpoint.setId === config.installedSha256['resource-index.json'] &&
  failed.readinessTiming.readyAcceptedBeforeDeadline === true && path.isAbsolute(input.retainedJournalPath) &&
  path.isAbsolute(input.outputDirectory) && path.dirname(input.result) === input.outputDirectory &&
  !fs.existsSync(input.result), 'Original held endpoint or fresh result differs');
fs.mkdirSync(input.outputDirectory, { recursive: true });
const { ActualProduct } = await import(pathToFileURL(input.productCdp.path).href);
const journal = { schema: 'gogoke.37.retained-installed-close.v1', state: 'CLOSE_PENDING', acceptance: false,
  originalFailureRetained: true, inputSha256: digest(inputPath), failureSnapshot: input.failureSnapshot,
  retainedJournalPath: input.retainedJournalPath, sourceCommit: failed.sourceCommit,
  candidateVersion: failed.candidateVersion, candidateInstalledSha256: failed.candidateInstalledSha256,
  stateRoot: failed.stateRoot, launches: [endpoint], operations: [], sessions: [], closes: [],
  loadedCloseEntry: { path: fileURLToPath(import.meta.url), sha256: input.actorSha256 } };
const product = new ActualProduct({ ...config, evidenceDirectory: input.outputDirectory, result: input.result }, journal);
product.endpoint = endpoint;
product.stderrFile = endpoint.stderrPath;
let poll;
const source = () => {
  const current = read(input.retainedJournalPath);
  need(current.originalCaseId === failed.originalCaseId && current.sourceCommit === failed.sourceCommit &&
    current.launches.length === 1 && current.launches[0].pid === endpoint.pid &&
    current.driverBytes['product-cdp.mjs'] === input.productCdp.sha256 &&
    current.operations.at(-1)?.rawFrame === failed.operations.at(-1)?.rawFrame,
    'Original retained driver journal changed identity');
  return current;
};
const originalExit = () => source().productExits?.find(x => x.pid === endpoint.pid);
// This adapter reads the retained parent's actual ChildProcess exit receipt.
// It never treats PID absence, a timer, or a fresh process as an exit code.
product.child = {
  get exitCode() { return originalExit()?.code ?? null; },
  get signalCode() { return originalExit()?.signal ?? null; },
  once(event, callback) {
    need(event === 'exit' && !poll, 'Only one original child-exit subscription permitted');
    poll = setInterval(() => {
      try {
        const event = originalExit();
        if (!event) return;
        clearInterval(poll); poll = null;
        need(Number.isInteger(event.code) && event.signal === null && Number.isFinite(Date.parse(event.observedAt)),
          'Original child exit receipt is incomplete');
        journal.retainedNodeExit = { sourcePath: input.retainedJournalPath,
          sourceSha256: digest(input.retainedJournalPath), event };
        product.save(); callback(event.code, event.signal);
      } catch (error) { clearInterval(poll); poll = null; journal.sourceError = String(error); product.save(); }
    }, 100);
  },
};
try {
  source(); product.verifyBytes(); await product.custody();
  const targets = await fetch(`http://127.0.0.1:${endpoint.port}/json/list`).then(r => r.json());
  const matching = targets.filter(x => x.type === 'page' && x.url === endpoint.url);
  need(matching.length === 1, 'Original installed WebView is not unique');
  await product.connectDiagnostic(matching[0]);
  product.socket.send(JSON.stringify({ id: ++product.sequence, method: 'Runtime.enable' }));
  const ui = await product.evaluate('({url:location.href,home:!!document.querySelector(".home-product-entry"),tauri:!!window.__TAURI_INTERNALS__})');
  need(ui.url === endpoint.url && ui.home && ui.tauri, 'Original Home/USER view differs');
  product.save(); await product.closeNormally();
  need(journal.closes.length === 1 && journal.closes[0].exitCode === 0, 'Original shared normal close not complete');
  journal.state = 'RETAINED_FAILED_LAUNCH_CLOSED_REVIEW_REQUIRED'; product.save();
} catch (error) {
  journal.state = 'RETAINED_CLOSE_FAILED_ORIGINAL_FAILURE_PRESERVED'; journal.error = String(error?.stack ?? error);
  product.save(); process.exitCode = 1;
} finally { if (poll) clearInterval(poll); product.socket?.close(); }
