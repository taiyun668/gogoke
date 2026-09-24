#!/usr/bin/env node
// Cloud-only staging and byte-identity check for the installed Windows service.
import { createHash } from 'node:crypto';
import { spawn, spawnSync } from 'node:child_process';
import { createServer } from 'node:net';
import { setTimeout as delay } from 'node:timers/promises';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { fileURLToPath } from 'node:url';
import { selectCliRuntimeExternalDependencies } from '../../third_party/t3code/scripts/lib/cli-external-packages.ts';
import serverPackage from '../../third_party/t3code/apps/server/package.json' with { type: 'json' };

const [mode, root, other] = process.argv.slice(2);
const repo = path.resolve(import.meta.dirname, '../..');
const desktop = path.join(repo, 'apps/desktop');
const service = path.join(desktop, 'src-tauri/resources/gogoke-service');
const sidecar = path.join(desktop, 'src-tauri/binaries/gogoke-native-host-x86_64-pc-windows-msvc.exe');
const manifestPath = path.join(desktop, 'generated/gogoke-package-hashes.json');
const required = ['dist/bin.mjs', 'runtime/node.exe', 'runtime/LICENSE', 'fixtures/controlled-pi.mjs', 'T3-LICENSE', 'T3-runtime-notices.html', 'native-host-notices.html'];

function ensure(file) {
  if (!fs.statSync(file, { throwIfNoEntry: false })?.isFile()) throw new Error(`required file missing: ${file}`);
  return file;
}
function digest(file) { return createHash('sha256').update(fs.readFileSync(ensure(file))).digest('hex'); }
function copy(source, target) {
  fs.mkdirSync(path.dirname(target), { recursive: true });
  fs.copyFileSync(ensure(source), target);
  if (digest(source) !== digest(target)) throw new Error(`copy changed bytes: ${target}`);
}
function within(root, candidate) {
  const relative = path.relative(path.resolve(root), path.resolve(candidate));
  return relative !== '' && relative !== '..' && !relative.startsWith(`..${path.sep}`) && !path.isAbsolute(relative);
}
function assertPhysicalRuntime(root, removeBin = false) {
  const resolvedRoot = fs.realpathSync(root);
  function visit(dir) {
    if (!within(root, dir) && path.resolve(dir) !== path.resolve(root)) throw new Error(`runtime traversal escaped stage: ${dir}`);
    for (const name of fs.readdirSync(dir)) {
      const child = path.join(dir, name);
      if (!within(root, child)) throw new Error(`runtime path escaped stage: ${child}`);
      const stat = fs.lstatSync(child);
      if (name === '.bin' && removeBin) {
        if (!stat.isDirectory() || stat.isSymbolicLink() ||
            !within(resolvedRoot, fs.realpathSync(child))) {
          throw new Error(`refusing to remove runtime shim path outside stage: ${child}`);
        }
        fs.rmSync(child, { recursive: true, force: true });
        continue;
      }
      if (stat.isSymbolicLink()) throw new Error(`runtime link cannot be bundled as a physical resource: ${child}`);
      if (stat.isDirectory()) visit(child);
    }
  }
  visit(root);
}
function externalRoots() {
  const names = Object.keys(selectCliRuntimeExternalDependencies(serverPackage.dependencies));
  for (const name of names) ensure(path.join(service, 'node_modules', name, 'package.json'));
  ensure(path.join(service, 'node_modules/@ff-labs/fff-bin-win32-x64/package.json'));
  return names;
}
// Read the actual redistributed texts, including nested native-component notices.
export function licenseSources(directory) {
  const sources = [];
  function visit(dir) {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      if (['node_modules', '.git', 'target'].includes(entry.name)) continue;
      const file = path.join(dir, entry.name);
      if (entry.isSymbolicLink()) throw new Error('license source is not a physical file');
      if (entry.isDirectory()) visit(file);
      else if (/^(LICEN[CS]E|NOTICE|COPYING|COPYRIGHT|AUTHORS)([._-].*)?$/i.test(entry.name)) {
        const text = new TextDecoder('utf-8', { fatal: true }).decode(fs.readFileSync(file)).trim();
        if (!text) throw new Error(`empty license source: ${entry.name}`);
        sources.push({ name: path.relative(directory, file).replaceAll('\\', '/'), text, sha256: digest(file) });
      }
    }
  }
  visit(directory);
  return sources.sort((a, b) => a.name.localeCompare(b.name));
}
export function licenses(serviceRoot = service, requireText = true) {
  const inventory = [];
  function visitModules(modules) {
    if (!fs.existsSync(modules)) return;
    function visitPackage(dir) {
      const pkg = JSON.parse(fs.readFileSync(ensure(path.join(dir, 'package.json')), 'utf8'));
      if (typeof pkg.name !== 'string' || typeof pkg.version !== 'string') throw new Error('runtime package identity missing');
      const packagePath = path.relative(serviceRoot, dir).replaceAll('\\', '/');
      const sources = licenseSources(dir);
      inventory.push({ id: `${pkg.name}@${pkg.version}`, license: pkg.license ?? null, packagePath,
        files: sources.map((source) => ({ path: `${packagePath}/${source.name}`, sha256: source.sha256 })) });
      visitModules(path.join(dir, 'node_modules'));
    }
    for (const entry of fs.readdirSync(modules, { withFileTypes: true })) {
      if (entry.name.startsWith('.')) continue;
      const child = path.join(modules, entry.name);
      if (entry.isSymbolicLink()) throw new Error('runtime package is not physical');
      if (!entry.isDirectory()) continue;
      if (entry.name.startsWith('@')) {
        for (const name of fs.readdirSync(child)) visitPackage(path.join(child, name));
      } else visitPackage(child);
    }
  }
  visitModules(path.join(serviceRoot, 'node_modules'));
  if (inventory.length === 0) throw new Error('runtime package inventory empty');
  const missing = inventory.filter((item) => item.files.length === 0).map((item) => item.id);
  if (requireText && missing.length) throw new Error(`runtime license text missing: ${missing.join(', ')}`);
  return inventory.sort((a, b) => a.id.localeCompare(b.id) || a.packagePath.localeCompare(b.packagePath));
}

// npm's exact-version gitHead is the only upstream fallback. Mutable tags,
// guessed copyright holders and SPDX template text are never substitutes.
export function upstreamLicenseBasis(pkg, metadata) {
  if (metadata.name !== pkg.name || metadata.version !== pkg.version ||
      !/^[0-9a-f]{40}$/.test(metadata.gitHead ?? '')) throw new Error(`immutable upstream license revision missing: ${pkg.name}@${pkg.version}`);
  const repository = typeof metadata.repository === 'string' ? metadata.repository : metadata.repository?.url;
  const match = /^(?:git\+)?https:\/\/github\.com\/([A-Za-z0-9_.-]+)\/([A-Za-z0-9_.-]+?)(?:\.git)?$/.exec(repository ?? '');
  if (!match) throw new Error(`unsupported upstream license repository: ${pkg.name}@${pkg.version}`);
  const directory = typeof metadata.repository === 'object' ? metadata.repository.directory ?? '' : '';
  if (directory && (!/^[A-Za-z0-9_./-]+$/.test(directory) || directory.startsWith('/') || directory.split('/').some((v) => !v || v === '.' || v === '..'))) throw new Error('unsafe upstream license directory');
  return { repository: `${match[1]}/${match[2]}`, commit: metadata.gitHead, directory };
}
async function boundedSource(url, optional = false) {
  const response = await fetch(url, { signal: AbortSignal.timeout(15000), redirect: 'error' });
  if (optional && response.status === 404) return null;
  if (!response.ok) throw new Error(`license source HTTP ${response.status}`);
  const chunks = []; let length = 0;
  for await (const chunk of response.body) {
    length += chunk.length;
    if (length > 4 * 1024 * 1024) throw new Error('license source exceeds bound');
    chunks.push(chunk);
  }
  const text = new TextDecoder('utf-8', { fatal: true }).decode(Buffer.concat(chunks));
  if (!text.trim()) throw new Error('license source empty');
  return text;
}
async function hydrateRuntimeNotices() {
  const cache = new Map();
  for (const item of licenses(service, false).filter((item) => item.files.length === 0)) {
    const dir = path.join(service, item.packagePath);
    const pkg = JSON.parse(fs.readFileSync(path.join(dir, 'package.json'), 'utf8'));
    const registry = `https://registry.npmjs.org/${encodeURIComponent(pkg.name)}/${encodeURIComponent(pkg.version)}`;
    const metadata = JSON.parse(await boundedSource(registry));
    const basis = upstreamLicenseBasis(pkg, metadata);
    const directories = [];
    let directory = basis.directory;
    for (;;) {
      directories.push(directory);
      if (!directory) break;
      directory = directory.includes('/') ? directory.slice(0, directory.lastIndexOf('/')) : '';
    }
    let source;
    for (const parent of directories) {
      for (const name of ['LICENSE', 'LICENSE.md', 'LICENSE.txt', 'LICENSE-MIT', 'LICENCE', 'COPYING']) {
        const sourcePath = [parent, name].filter(Boolean).join('/');
        const url = `https://raw.githubusercontent.com/${basis.repository}/${basis.commit}/${sourcePath}`;
        if (!cache.has(url)) cache.set(url, await boundedSource(url, true));
        const text = cache.get(url);
        if (text) { source = { url, path: sourcePath, text }; break; }
      }
      if (source) break;
    }
    if (!source) throw new Error(`pinned upstream license text missing: ${item.id}`);
    const sha256 = createHash('sha256').update(source.text).digest('hex');
    fs.writeFileSync(path.join(dir, 'LICENSE.gogoke-upstream.txt'), source.text);
    fs.writeFileSync(path.join(dir, 'UPSTREAM-LICENSE-PROVENANCE.json'), JSON.stringify({
      package: item.id, registry, repository: basis.repository, commit: basis.commit,
      path: source.path, source: source.url, sha256, claim: 'EXACT_UPSTREAM_SOURCE_NOT_LEGAL_ACCEPTANCE',
    }, null, 2) + '\n');
  }
}

function escapeHtml(text) {
  return String(text).replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;').replaceAll('"', '&quot;');
}
function noticeSection(title, items, provenance) {
  return `<section><h1>${escapeHtml(title)}</h1><p>${escapeHtml(provenance)}. Source text inventory, not legal acceptance.</p>` +
    items.map((item) => `<h2>${escapeHtml(item.id)}</h2><p>Declared license: ${escapeHtml(typeof item.license === 'string' ? item.license : JSON.stringify(item.license))}</p>` +
      item.sources.map((source) => `<h3>${escapeHtml(source.name)} · SHA-256 ${source.sha256}</h3><pre>${escapeHtml(source.text)}</pre>`).join('\n')).join('\n') + '</section>\n';
}
function appendNotice(file, id, section) {
  const start = `<!-- ${id}:start -->`, end = `<!-- ${id}:end -->`;
  let html = fs.readFileSync(ensure(file), 'utf8');
  if (html.includes(start)) {
    const a = html.indexOf(start), b = html.indexOf(end, a);
    if (b < a) throw new Error('incomplete prior package notice');
    html = html.slice(0, a) + html.slice(b + end.length);
  }
  if (!html.includes('</html>')) throw new Error('generated notice is not HTML');
  fs.writeFileSync(file, html.replace('</html>', `${start}\n${section}${end}\n</html>`));
}
function packageNotices(inventory) {
  const pnpmLock = digest(path.join(repo, 'third_party/t3code/pnpm-lock.yaml'));
  const runtime = inventory.map((item) => ({ ...item, sources: item.files.map((source) => ({
    name: source.path, sha256: source.sha256,
    text: fs.readFileSync(ensure(path.join(service, source.path)), 'utf8'),
  })) }));
  const t3 = noticeSection('Gogoke T3 service runtime dependency notices', runtime, `pnpm-lock.yaml SHA-256: ${pnpmLock}`);
  const nativeRoot = path.join(desktop, 'native-host');
  // Metadata only: native compilation remains in the preceding cloud build step.
  const result = spawnSync('cargo', ['+1.85.1', 'metadata', '--locked', '--format-version=1',
    '--manifest-path', path.join(nativeRoot, 'Cargo.toml'), '--filter-platform', 'x86_64-pc-windows-msvc'],
    { encoding: 'utf8', timeout: 120000, maxBuffer: 16 * 1024 * 1024, windowsHide: true });
  if (result.error || result.status !== 0) throw new Error('native-host locked dependency metadata failed');
  const metadata = JSON.parse(result.stdout);
  if (!metadata.resolve?.root || !Array.isArray(metadata.resolve.nodes)) throw new Error('native-host dependency graph missing');
  const ids = new Set(metadata.resolve.nodes.map((item) => item.id));
  const crates = metadata.packages.filter((item) => ids.has(item.id) && item.id !== metadata.resolve.root).map((item) => {
    const sources = licenseSources(path.dirname(item.manifest_path));
    if (sources.length === 0) throw new Error(`native-host license text missing: ${item.name}@${item.version}`);
    return { id: `${item.name}@${item.version}`, license: item.license, sources };
  });
  if (!crates.some((item) => item.id === 'ryu-js@1.0.3') || !crates.some((item) => item.id === 'cc@1.4.2')) throw new Error('native-host pinned dependency roots absent');
  const sqliteHeader = fs.readFileSync(path.join(nativeRoot, 'vendor/sqlite-3.53.2/sqlite3.h'), 'utf8');
  const sqliteNotice = sqliteHeader.split('\n').slice(0, 10).join('\n');
  if (!sqliteNotice.includes('The author disclaims copyright')) throw new Error('SQLite source notice missing');
  crates.push({ id: 'SQLite@3.53.2 (Route B patched)', license: 'Upstream copyright disclaimer',
    sources: [{ name: 'vendor/sqlite-3.53.2/sqlite3.h:1-10', text: sqliteNotice,
      sha256: digest(path.join(nativeRoot, 'vendor/sqlite-3.53.2/sqlite3.h')) }] });
  const cargoLock = digest(path.join(nativeRoot, 'Cargo.lock'));
  const native = noticeSection('Gogoke native-host dependency notices', crates, `native-host/Cargo.lock SHA-256: ${cargoLock}`);
  for (const [name, content] of [['T3-runtime-notices.html', t3], ['native-host-notices.html', native]]) {
    fs.writeFileSync(path.join(service, name), `<!doctype html><html lang="en"><meta charset="utf-8">${content}</html>\n`);
  }
  appendNotice(path.join(desktop, 'generated/npm-production-notices.html'), 'gogoke-t3-service', t3);
  appendNotice(path.join(desktop, 'generated/rust-dependency-notices.html'), 'gogoke-native-host', native);
  return { state: 'SOURCE_TEXT_COLLECTED_NOT_LEGAL_ACCEPTANCE', runtimePackages: runtime.length,
    nativeDependencies: crates.map((item) => item.id), pnpmLock, cargoLock };
}
export function fileHashes(directory) {
  assertPhysicalRuntime(directory);
  const hashes = {};
  function visit(dir) {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      const file = path.join(dir, entry.name);
      if (entry.isDirectory()) visit(file);
      else if (entry.isFile()) hashes[path.relative(directory, file).replaceAll('\\', '/')] = digest(file);
      else throw new Error('unsupported runtime file type');
    }
  }
  visit(directory);
  return Object.fromEntries(Object.entries(hashes).sort(([a], [b]) => a.localeCompare(b)));
}
async function writeManifest() {
  await hydrateRuntimeNotices();
  assertPhysicalRuntime(path.join(service, 'node_modules'));
  const inventory = licenses();
  const noticeCoverage = packageNotices(inventory);
  const files = Object.fromEntries(required.map((name) => [name, digest(path.join(service, name))]));
  files['gogoke-native-host.exe'] = digest(sidecar);
  const nativeBinaries = [];
  function findNative(dir) {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      const file = path.join(dir, entry.name);
      if (entry.isSymbolicLink()) continue;
      if (entry.isDirectory()) findNative(file);
      else if (entry.name.endsWith('.node')) nativeBinaries.push(path.relative(service, file).replaceAll('\\', '/'));
    }
  }
  findNative(path.join(service, 'node_modules'));
  if (nativeBinaries.length === 0) throw new Error('deployed external closure has no native .node binary');
  const record = { schema: 1, nodeVersion: process.version, files, externalRoots: externalRoots(), runtimePackages: inventory, packagesMissingNoticeFiles: inventory.filter((p) => p.files.length === 0).map((p) => p.id) };
  record.noticeCoverage = noticeCoverage;
  record.runtimeFileHashes = fileHashes(service);
  record.nativeBinaries = nativeBinaries.sort();
  record.nativeBinaryHashes = Object.fromEntries(record.nativeBinaries.map((name) => [name, digest(path.join(service, name))]));
  fs.mkdirSync(path.dirname(manifestPath), { recursive: true });
  fs.writeFileSync(manifestPath, JSON.stringify(record, null, 2) + '\n');
  console.log(`staged ${record.externalRoots.length} external roots; ${inventory.length} runtime packages; ${record.packagesMissingNoticeFiles.length} without license/notice files`);
}
export function verify(destination, manifest) {
  const record = JSON.parse(fs.readFileSync(ensure(manifest), 'utf8'));
  if (record.schema !== 1) throw new Error('unknown package manifest');
  if ([...required, 'gogoke-native-host.exe'].some((name) => !/^[0-9a-f]{64}$/.test(record.files?.[name] ?? ''))) throw new Error('required package inventory incomplete');
  if (record.noticeCoverage?.state !== 'SOURCE_TEXT_COLLECTED_NOT_LEGAL_ACCEPTANCE' ||
      record.packagesMissingNoticeFiles?.length !== 0 || !record.runtimeFileHashes) throw new Error('package notices or runtime hashes incomplete');
  for (const [name, expected] of Object.entries(record.files)) {
    const target = name === 'gogoke-native-host.exe'
      ? path.resolve(destination) === path.resolve(path.join(desktop, 'src-tauri/resources')) ? sidecar : path.join(destination, name)
      : path.join(destination, 'gogoke-service', name);
    if (digest(target) !== expected) throw new Error(`installed byte mismatch: ${name}`);
  }
  for (const pkg of record.runtimePackages) for (const item of pkg.files) {
    const target = path.join(destination, 'gogoke-service', item.path);
    if (digest(target) !== item.sha256) throw new Error(`installed notice mismatch: ${item.path}`);
  }
  for (const name of record.externalRoots) ensure(path.join(destination, 'gogoke-service/node_modules', name, 'package.json'));
  for (const name of record.nativeBinaries) {
    if (digest(path.join(destination, 'gogoke-service', name)) !== record.nativeBinaryHashes[name]) throw new Error(`installed native addon byte mismatch: ${name}`);
  }
  const actual = fileHashes(path.join(destination, 'gogoke-service'));
  const expectedNames = Object.keys(record.runtimeFileHashes);
  if (Object.keys(actual).length !== expectedNames.length || expectedNames.some((name) => actual[name] !== record.runtimeFileHashes[name])) throw new Error('installed runtime closure byte mismatch');
  console.log(`PASS installed byte identity: ${Object.keys(record.files).length} required files and runtime notice inventory`);
}
async function smokeTauri(installed, request) {
  if (process.platform !== 'win32' || process.env.GITHUB_ACTIONS !== 'true') throw new Error('installed Tauri smoke is cloud Windows only');
  const temp = fs.mkdtempSync(path.join(os.tmpdir(), 'gogoke-ui-smoke-'));
  const listener = createServer();
  await new Promise((resolve, reject) => { listener.once('error', reject); listener.listen(0, '127.0.0.1', resolve); });
  const port = listener.address().port;
  await new Promise((resolve) => listener.close(resolve));
  // Microsoft WebView2 documented per-process diagnostic flags. No registry,
  // product configuration, security policy, or Owner-machine setting is changed.
  const child = spawn(ensure(path.join(installed, 'gogoke.exe')), [], {
    cwd: temp, stdio: 'ignore', windowsHide: false,
    env: { ...process.env, APPDATA: path.join(temp, 'roaming'), LOCALAPPDATA: path.join(temp, 'local'),
      WEBVIEW2_USER_DATA_FOLDER: path.join(temp, 'webview'),
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port} --remote-debugging-address=127.0.0.1` },
  });
  let spawnError, exited = false, socket;
  child.once('error', (error) => { spawnError = error; });
  child.once('exit', () => { exited = true; });
  try {
    const deadline = Date.now() + 45000;
    let target;
    while (Date.now() < deadline && !target) {
      if (spawnError || exited) throw new Error('installed Tauri process failed before WebView readiness');
      try {
        const reply = await fetch(`http://127.0.0.1:${port}/json/list`, { signal: AbortSignal.timeout(2000) });
        if (reply.ok) {
          const pages = await reply.json();
          target = pages.find((page) => page.type === 'page' && /^(https?:\/\/tauri\.localhost|tauri:\/\/localhost)(\/|$)/.test(page.url));
        }
      } catch { /* Startup-only read retry; product invoke below is never retried. */ }
      if (!target) await delay(250);
    }
    if (!target) throw new Error('installed Tauri WebView target unavailable');
    const endpoint = new URL(target.webSocketDebuggerUrl);
    if (!['127.0.0.1', 'localhost'].includes(endpoint.hostname) || Number(endpoint.port) !== port || endpoint.protocol !== 'ws:') throw new Error('unexpected WebView debugger endpoint');
    socket = new WebSocket(endpoint);
    await new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error('WebView debugger connection timeout')), 5000);
      socket.addEventListener('open', () => { clearTimeout(timer); resolve(); }, { once: true });
      socket.addEventListener('error', () => { clearTimeout(timer); reject(new Error('WebView debugger connection failed')); }, { once: true });
    });
    let sequence = 0;
    const evaluate = (expression) => new Promise((resolve, reject) => {
      const id = ++sequence;
      const finish = (error, result) => {
        clearTimeout(timer); socket.removeEventListener('message', onMessage); socket.removeEventListener('close', onClose);
        if (error) reject(error); else resolve(result);
      };
      const onClose = () => finish(new Error('WebView closed; product operation not retried'));
      const onMessage = (event) => {
        const value = JSON.parse(event.data);
        if (value.id !== id) return;
        if (value.error || value.result?.exceptionDetails) finish(new Error('installed Tauri invoke failed: ' + JSON.stringify(value.error ?? value.result.exceptionDetails)));
        else finish(null, value.result?.result?.value);
      };
      const timer = setTimeout(() => finish(new Error('Tauri invoke timed out; product operation not retried')), 45000);
      socket.addEventListener('message', onMessage); socket.addEventListener('close', onClose);
      socket.send(JSON.stringify({ id, method: 'Runtime.evaluate', params: { expression, returnByValue: true, awaitPromise: true } }));
    });
    let ready = false;
    while (Date.now() < deadline && !ready) {
      ready = await evaluate('Boolean(window.__TAURI_INTERNALS__ && document.querySelector(".home-product-entry"))');
      if (!ready) await delay(250);
    }
    if (!ready) throw new Error('installed Home product entry did not render');
    const response = await evaluate(`window.__TAURI_INTERNALS__.invoke('gogoke_r2_goal_probe', {request:${JSON.stringify(request)}})`);
    assertSmokeResponse(response, request);
    // A missing mandatory runtime must fail at the same Tauri product ingress.
    const node = path.join(installed, 'gogoke-service/runtime/node.exe');
    const held = node + '.r204-held';
    fs.renameSync(node, held);
    try {
      const rejected = await evaluate(`window.__TAURI_INTERNALS__.invoke('gogoke_r2_goal_probe', {request:${JSON.stringify(request)}}).then(() => 'UNEXPECTED_SUCCESS', error => String(error))`);
      if (rejected !== 'GOGOKE_PRODUCT_COMPONENT_MISSING:node-runtime') throw new Error('missing installed Node was not rejected by Tauri');
    } finally { fs.renameSync(held, node); }
    const record = JSON.parse(fs.readFileSync(ensure(manifestPath), 'utf8'));
    record.installedSmoke = { state: 'PASS', platform: 'WINDOWS_CLOUD_NOT_OWNER_WIN11',
      runId: process.env.GITHUB_RUN_ID, sourceSha: process.env.GITHUB_SHA,
      entry: 'installed gogoke.exe Home -> gogoke_r2_goal_probe -> installed dist/bin.mjs -> native-host',
      controlledTask: 'VALIDATED_TEST_RESULT_NOT_ADOPTED', missingNode: 'REJECTED_AT_TAURI_INGRESS',
      appSha256: digest(path.join(installed, 'gogoke.exe')), adoption: false, release: false };
    fs.writeFileSync(manifestPath, JSON.stringify(record, null, 2) + '\n');
    console.log('PASS installed Tauri Home and product invoke; missing Node rejected at the same ingress');
  } finally {
    socket?.close();
    if (!exited && child.pid) {
      const killed = spawnSync('taskkill', ['/PID', String(child.pid), '/T', '/F'], { encoding: 'utf8', timeout: 10000, windowsHide: true });
      if (killed.error) throw new Error('owned cloud smoke process cleanup unknown; temp retained');
      for (let i = 0; i < 20 && !exited; i += 1) await delay(250);
    }
    if (!exited && !spawnError) throw new Error('owned cloud smoke process exit unconfirmed; temp retained');
    const resolved = fs.realpathSync(temp);
    if (!within(fs.realpathSync(os.tmpdir()), resolved) || !path.basename(resolved).startsWith('gogoke-ui-smoke-')) throw new Error('unsafe cloud smoke temp cleanup');
    fs.rmSync(resolved, { recursive: true, force: true });
  }
}

function assertSmokeResponse(response, request) {
    if (response.goal?.id !== request.goal.id || response.caller?.admitted !== true || response.caller?.role !== 'controller' || response.nativeHost?.reachable !== true || response.ledgerReadback?.state !== 'COMMITTED_BYTES_VERIFIED_NOT_ADOPTED' || response.ledgerReadback?.gitBlob !== 'a20115fdd5acf9e7e5025c3b3ca50696001badac' || response.acceptance !== 'TEST_FIXTURE_NOT_ADOPTED' || response.testLedgerDraft !== undefined) throw new Error('installed service did not prove admitted native and Git readback without draft');
    const task = response.controlledTask;
    if (task?.state !== 'VALIDATED_TEST_RESULT_NOT_ADOPTED' || task?.objectiveOutcomeReceiptId !== 'outcome-receipt-r2-02-test' || task?.evaluationReceiptId !== 'evaluation-receipt-r2-02-test' || task?.dreamProposalState !== 'DRAFT_TEST_ONLY_NOT_ACTIVATED' || !['manifestHash', 'objectiveOutcomeContentHash', 'evaluationContentHash', 'metricsHash', 'dreamRunContentHash', 'dreamProposalContentHash'].every((key) => /^sha256:[0-9a-f]{64}$/.test(task[key]))) throw new Error('installed controlled task lacked validated Result, Outcome, Evaluation, or Dream');
}

async function smoke(destination) {
  const installed = path.resolve(destination);
  const temp = fs.mkdtempSync(path.join(os.tmpdir(), 'gogoke-service-smoke-'));
  const request = {
    goal: { id: 'goal-r2-01', title: 'Reach the native Product Authority from the Gogoke product entry' },
    ledger: { repository: 'taiyun668/gogoke', commit: '6765d4e11ace61c47b9aeb123e0ef4770ab072c0', path: 'apps/desktop/test-fixtures/s1-r4/ledger/r2-02-source-reference.json', contentHash: 'sha256:b57db8a5fec4d9a4a09ca1e356c865017f88473916c5debeefa0ca2d87b08d08' },
    runControlledTask: true,
  };
  try {
    const result = spawnSync(path.join(installed, 'gogoke-service/runtime/node.exe'), [path.join(installed, 'gogoke-service/dist/bin.mjs'), '--root', temp, '--native-host', path.join(installed, 'gogoke-native-host.exe')], { cwd: path.join(installed, 'gogoke-service'), input: JSON.stringify(request), encoding: 'utf8', timeout: 180000, maxBuffer: 1024 * 1024, windowsHide: true });
    if (result.error || result.status !== 0) throw new Error(`installed service failed: ${result.error?.message ?? result.status}; ${result.stderr?.slice(-3000)}`);
    const response = JSON.parse(result.stdout);
    assertSmokeResponse(response, request);
    await smokeTauri(installed, request);
    console.log('PASS installed service native admission, read-only Git fact, and controlled Result/Outcome/Evaluation/Dream without adoption');
  } finally {
    const tempRoot = fs.realpathSync(os.tmpdir());
    const resolved = fs.realpathSync(temp);
    if (!within(tempRoot, resolved) || !path.basename(resolved).startsWith('gogoke-service-smoke-')) throw new Error('refusing to remove temp outside gogoke smoke root');
    fs.rmSync(resolved, { recursive: true, force: true });
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
try {
if (mode === 'stage') {
  if (path.resolve(root) !== path.resolve(service)) throw new Error('unexpected service stage');
  if (process.version !== 'v24.13.1') throw new Error(`staged Node version differs from pinned LICENSE: ${process.version}`);
  copy(process.execPath, path.join(service, 'runtime/node.exe'));
  copy(ensure(other), path.join(service, 'runtime/LICENSE'));
  copy(path.join(repo, 'apps/desktop/test-fixtures/s1-r4/ledger/controlled-pi.mjs'), path.join(service, 'fixtures/controlled-pi.mjs'));
  copy(path.join(repo, 'third_party/t3code/LICENSE'), path.join(service, 'T3-LICENSE'));
  assertPhysicalRuntime(path.join(service, 'node_modules'), true);
  externalRoots();
} else if (mode === 'seal') {
  await writeManifest();
} else if (mode === 'verify') {
  verify(ensureDir(root), ensure(other));
} else if (mode === 'smoke') {
  await smoke(ensureDir(root));
 } else throw new Error('usage: stage <service-dir> <node-license> | seal | verify <installed-dir> <manifest> | smoke <installed-dir>');
} catch (error) {
  let message = String(error?.message ?? error);
  for (const value of [process.env.GITHUB_TOKEN, process.env.GH_TOKEN, repo, os.tmpdir(), root, other]) {
    if (value) message = message.replaceAll(value, '[redacted]');
  }
  message = message.slice(0, 5000);
  const failure = { schema: 'gogoke.r2-04.package-operation.v1', operation: mode, state: 'FAIL',
    sourceSha: process.env.GITHUB_SHA ?? 'LOCAL_DIAGNOSTIC', runId: process.env.GITHUB_RUN_ID ?? null,
    message, nativeLocallyCompiled: false };
  fs.mkdirSync(path.dirname(manifestPath), { recursive: true });
  let record = {};
  try { record = JSON.parse(fs.readFileSync(manifestPath, 'utf8')); } catch { /* First failed seal has no inventory. */ }
  record.packageOperationFailure = failure;
  fs.writeFileSync(manifestPath, JSON.stringify(record, null, 2) + '\n');
  console.error('::error::' + message.replaceAll('%', '%25').replaceAll('\r', '%0D').replaceAll('\n', '%0A'));
  process.exitCode = 1;
}
}

function ensureDir(dir) {
  if (!fs.statSync(dir, { throwIfNoEntry: false })?.isDirectory()) throw new Error(`required directory missing: ${dir}`);
  return dir;
}
