#!/usr/bin/env node
// Cloud-only staging and byte-identity check for the installed Windows service.
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { selectCliRuntimeExternalDependencies } from '../../third_party/t3code/scripts/lib/cli-external-packages.ts';
import serverPackage from '../../third_party/t3code/apps/server/package.json' with { type: 'json' };

const [mode, root, other] = process.argv.slice(2);
const repo = path.resolve(import.meta.dirname, '../..');
const desktop = path.join(repo, 'apps/desktop');
const service = path.join(desktop, 'src-tauri/resources/gogoke-service');
const sidecar = path.join(desktop, 'src-tauri/binaries/gogoke-native-host-x86_64-pc-windows-msvc.exe');
const manifestPath = path.join(desktop, 'generated/gogoke-package-hashes.json');
const required = ['dist/bin.mjs', 'runtime/node.exe', 'runtime/LICENSE', 'fixtures/controlled-pi.mjs', 'T3-LICENSE'];

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
  function visit(dir) {
    if (!within(root, dir) && path.resolve(dir) !== path.resolve(root)) throw new Error(`runtime traversal escaped stage: ${dir}`);
    for (const name of fs.readdirSync(dir)) {
      const child = path.join(dir, name);
      if (!within(root, child)) throw new Error(`runtime path escaped stage: ${child}`);
      if (name === '.bin' && removeBin) {
        fs.rmSync(child, { recursive: true, force: true });
        continue;
      }
      const stat = fs.lstatSync(child);
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
function licenses() {
  const inventory = [];
  const visited = new Set();
  function visit(dir) {
    if (!fs.existsSync(dir)) return;
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      if (entry.name === '.bin') continue;
      const child = path.join(dir, entry.name);
      if (entry.isSymbolicLink()) {
        const resolved = fs.realpathSync(child);
        if (!resolved.startsWith(service + path.sep)) throw new Error(`external runtime link escapes stage: ${child}`);
        continue;
      }
      if (!entry.isDirectory()) continue;
      const pkgFile = path.join(child, 'package.json');
      if (fs.existsSync(pkgFile)) {
        const pkg = JSON.parse(fs.readFileSync(pkgFile, 'utf8'));
        const id = `${pkg.name}@${pkg.version}`;
        if (!visited.has(id)) {
          visited.add(id);
          const noticeFiles = fs.readdirSync(child).filter((name) => /^(LICENSE|LICENCE|NOTICE|COPYING)(\.|$)/i.test(name) && fs.statSync(path.join(child, name)).isFile());
          inventory.push({ id, license: pkg.license ?? null, files: noticeFiles.map((name) => ({ path: path.relative(service, path.join(child, name)).replaceAll('\\', '/'), sha256: digest(path.join(child, name)) })) });
        }
      }
      visit(child);
    }
  }
  visit(path.join(service, 'node_modules'));
  inventory.sort((a, b) => a.id.localeCompare(b.id));
  return inventory;
}
function writeManifest() {
  assertPhysicalRuntime(path.join(service, 'node_modules'));
  const files = Object.fromEntries(required.map((name) => [name, digest(path.join(service, name))]));
  files['gogoke-native-host.exe'] = digest(sidecar);
  const inventory = licenses();
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
  record.nativeBinaries = nativeBinaries.sort();
  record.nativeBinaryHashes = Object.fromEntries(record.nativeBinaries.map((name) => [name, digest(path.join(service, name))]));
  fs.mkdirSync(path.dirname(manifestPath), { recursive: true });
  fs.writeFileSync(manifestPath, JSON.stringify(record, null, 2) + '\n');
  console.log(`staged ${record.externalRoots.length} external roots; ${inventory.length} runtime packages; ${record.packagesMissingNoticeFiles.length} without license/notice files`);
}
function verify(destination, manifest) {
  const record = JSON.parse(fs.readFileSync(ensure(manifest), 'utf8'));
  if (record.schema !== 1) throw new Error('unknown package manifest');
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
  assertPhysicalRuntime(path.join(destination, 'gogoke-service/node_modules'));
  console.log(`PASS installed byte identity: ${Object.keys(record.files).length} required files and runtime notice inventory`);
}
function smoke(destination) {
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
    if (response.goal?.id !== request.goal.id || response.caller?.admitted !== true || response.caller?.role !== 'controller' || response.nativeHost?.reachable !== true || response.ledgerReadback?.state !== 'COMMITTED_BYTES_VERIFIED_NOT_ADOPTED' || response.ledgerReadback?.gitBlob !== 'a20115fdd5acf9e7e5025c3b3ca50696001badac' || response.acceptance !== 'TEST_FIXTURE_NOT_ADOPTED' || response.testLedgerDraft !== undefined) throw new Error('installed service did not prove admitted native and Git readback without draft');
    const task = response.controlledTask;
    if (task?.state !== 'VALIDATED_TEST_RESULT_NOT_ADOPTED' || task?.objectiveOutcomeReceiptId !== 'outcome-receipt-r2-02-test' || task?.evaluationReceiptId !== 'evaluation-receipt-r2-02-test' || task?.dreamProposalState !== 'DRAFT_TEST_ONLY_NOT_ACTIVATED' || !['manifestHash', 'objectiveOutcomeContentHash', 'evaluationContentHash', 'metricsHash', 'dreamRunContentHash', 'dreamProposalContentHash'].every((key) => /^sha256:[0-9a-f]{64}$/.test(task[key]))) throw new Error('installed controlled task lacked validated Result, Outcome, Evaluation, or Dream');
    console.log('PASS installed service native admission, read-only Git fact, and controlled Result/Outcome/Evaluation/Dream without adoption');
  } finally {
    const tempRoot = fs.realpathSync(os.tmpdir());
    const resolved = fs.realpathSync(temp);
    if (!within(tempRoot, resolved) || !path.basename(resolved).startsWith('gogoke-service-smoke-')) throw new Error('refusing to remove temp outside gogoke smoke root');
    fs.rmSync(resolved, { recursive: true, force: true });
  }
}

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
  writeManifest();
} else if (mode === 'verify') {
  verify(ensureDir(root), ensure(other));
} else if (mode === 'smoke') {
  smoke(ensureDir(root));
} else throw new Error('usage: stage <service-dir> <node-license> | seal | verify <installed-dir> <manifest> | smoke <installed-dir>');

function ensureDir(dir) {
  if (!fs.statSync(dir, { throwIfNoEntry: false })?.isDirectory()) throw new Error(`required directory missing: ${dir}`);
  return dir;
}
