// Public e2e web-engine connect API; attach the existing dedicated WebView.
// No provider, subscription, credentials, model or agent fixture is configured.
process.env.E2E_TELEMETRY_DISABLED = '1';
process.env.PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD = '1';
const { web, surfaceOf } = await import('@e2e-dev/web');

export async function connectWebView(endpoint, evidenceDirectory) {
  const address = `http://127.0.0.1:${endpoint.port}`;
  const engine = web({ viewport: null, connect: {
    cdpEndpoint: () => address, reconnectEndpoint: () => address,
  } });
  try {
    const signal = AbortSignal.timeout(15000);
    await engine.init({ runId: `installed-${endpoint.pid}`, targetName: 'installed-WebView2',
      projectRoot: evidenceDirectory, app: { site: new URL(endpoint.url).hostname },
      env: process.env, headed: true, workerSlot: 0, signal, log: () => {} });
    await engine.startAttempt({ attemptId: `actual-${endpoint.pid}`, artifactsDir: evidenceDirectory,
      signal, resolveSecret: async () => { throw Error('E2E does not request secrets'); } });
    const pages = surfaceOf(engine).context().pages().filter(page => page.url() === endpoint.url);
    if (pages.length !== 1 || (await pages[0].locator('.home-product-entry').count() !== 1 &&
         await pages[0].locator('.composer').count() !== 1) ||
        !await pages[0].evaluate(() => Boolean(window.__TAURI_INTERNALS__))) {
      throw Error('e2e must retain the unique actual Home/conversation and Tauri bridge');
    }
    return { engine, page: pages[0], dispose: () => engine.dispose({ signal: AbortSignal.timeout(10000), timeoutMs: 10000 }) };
  } catch (error) {
    try { await engine.dispose({ signal: AbortSignal.timeout(10000), timeoutMs: 10000 }); }
    catch (cleanup) { throw Error(`e2e connect: ${error}; cleanup: ${cleanup}`); }
    throw error;
  }
}
