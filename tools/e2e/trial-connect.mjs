// First trial of the public tester-army/e2e web engine against the actual WebView.
// No runner init wizard, subscription, model call, agent.act or authentication.
process.env.E2E_TELEMETRY_DISABLED = '1';
process.env.PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD = '1';
const { web, surfaceOf } = await import('@e2e-dev/web');
const { ActualProduct, readJson, id } = await import('./product-cdp.mjs');
const config = readJson(process.argv[2]);
// The trial creates its own e2e connection after the proven raw bootstrap.
config.testerArmy = false;
const journal = { schema: 'gogoke.37.e2e-connect-trial.v1', state: 'RUNNING', acceptance: false,
  telemetryDisabled: process.env.E2E_TELEMETRY_DISABLED === '1', agentActs: 0,
  authenticationActions: false, launches: [], closes: [], operations: [] };
const product = new ActualProduct(config, journal);
let engine;
try {
  await product.launch();
  const endpoint = `http://127.0.0.1:${product.endpoint.port}`;
  // A fresh dedicated actual WebView already exists. No isolated substitute context.
  engine = web({ viewport: null, connect: {
    cdpEndpoint: () => endpoint, reconnectEndpoint: () => endpoint,
  } });
  const signal = AbortSignal.timeout(15000);
  await engine.init({ runId: id('connectTrial'), targetName: 'installed-WebView2',
    projectRoot: config.evidenceDirectory, app: { site: new URL(product.endpoint.url).hostname },
    env: process.env, headed: true, workerSlot: 0, signal, log: () => {} });
  journal.stage = 'ENGINE_INITIALIZED_CONNECT_PENDING'; product.save();
  await engine.startAttempt({ attemptId: id('hardAssertions'), artifactsDir: config.evidenceDirectory,
    signal, resolveSecret: async () => { throw Error('No secret may be requested in this trial'); } });
  const pages = surfaceOf(engine).context().pages().filter(page => page.url() === product.endpoint.url);
  if (pages.length !== 1) throw Error('e2e did not retain the unique actual installed WebView');
  const page = pages[0];
  if (await page.locator('.home-product-entry').count() !== 1) throw Error('Actual Home hard locator failed');
  const hasTauri = await page.evaluate(() => Boolean(window.__TAURI_INTERNALS__));
  if (!hasTauri) throw Error('The original Tauri bridge is absent');
  const instances = await product.instances();
  journal.instances = instances;
  if (!instances.instances.some(row => row.instanceId === 'codexTestM1' && row.state === 'LOGGED_IN')) {
    throw Error('Original test instance is not automatically LOGGED_IN');
  }
  journal.state = 'PASS_ACTUAL_WEBVIEW_CONNECT_HARD_ASSERTIONS';
  journal.stage = 'CONNECTED_ORIGINAL_WEBVIEW_HARD_ASSERTIONS_PASSED';
} catch (error) {
  journal.state = engine ? 'TRIAL_FAILED_USE_EXISTING_CDP' : 'FAIL_INSTRUMENT_BEFORE_E2E_CONNECT';
  journal.originalError = String(error.stack ?? error);
} finally {
  if (engine) {
    try { await engine.dispose({ signal: AbortSignal.timeout(10000), timeoutMs: 10000 }); }
    catch (error) { journal.engineCleanupError = String(error); }
  }
  try { if (product.child?.exitCode === null) await product.closeNormally(); }
  catch (error) { journal.productCloseError = String(error); process.exitCode = 1; }
  product.save();
}
