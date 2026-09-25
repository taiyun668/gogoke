import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, writeFileSync, rmSync, unlinkSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { collectProductionNotices, renderNotices } from "./generate-npm-notices.mjs";

test("production notice generator rejects missing text, unknown license and path escape", () => {
  const root = mkdtempSync(join(tmpdir(), "gogoke-npm-notice-test-"));
  try {
    writeFileSync(join(root, "package.json"), JSON.stringify({ name: "gogoke", version: "0.1.3" }));
    const packages = [{ name: "gogoke", versionInfo: "0.1.3", packageFileName: "" }];
    for (let index = 0; index < 100; index += 1) {
      const name = `fixture-${index}`;
      const packageFileName = `node_modules/${name}`;
      const dir = join(root, packageFileName);
      mkdirSync(dir, { recursive: true });
      writeFileSync(join(dir, "LICENSE"), `Copyright fixture ${index}\nMIT license text`, "utf8");
      packages.push({ name, versionInfo: "1.0.0", licenseDeclared: "MIT", packageFileName });
    }
    const sbom = { packages };
    const notices = collectProductionNotices(sbom, root);
    assert.equal(notices.length, 100);
    assert.match(renderNotices(notices, "abc"), /Copyright fixture 99/);
    writeFileSync(join(root, "package.json"), JSON.stringify({ name: "gogoke", version: "0.1.4" }));
    assert.throws(() => collectProductionNotices(sbom, root), /root SPDX identity/);
    packages[0].versionInfo = "0.1.4";
    assert.equal(collectProductionNotices(sbom, root).length, 100);

    unlinkSync(join(root, "node_modules/fixture-0/LICENSE"));
    assert.throws(() => collectProductionNotices(sbom, root), /license\/copyright text missing/);
    writeFileSync(join(root, "node_modules/fixture-0/LICENSE"), "MIT text", "utf8");
    packages[1].licenseDeclared = "NOASSERTION";
    assert.throws(() => collectProductionNotices(sbom, root), /incomplete production package metadata/);
    packages[1].licenseDeclared = "MIT";
    packages[1].packageFileName = "../outside";
    assert.throws(() => collectProductionNotices(sbom, root), /escapes installed dependency root/);
    assert.throws(() => collectProductionNotices({ packages: [] }, root), /package set is empty/);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

import { createHash } from 'node:crypto';
import { licenses, fileHashes, verify, upstreamLicenseBasis } from '../ci/gogoke-package-service.mjs';

test('package notices include nested component texts without treating CJS metadata as a package', () => {
  const root = mkdtempSync(join(tmpdir(), 'gogoke-package-notice-'));
  try {
    const pkg = join(root, 'node_modules/fixture');
    mkdirSync(join(pkg, 'dist'), { recursive: true });
    mkdirSync(join(pkg, 'vendor'), { recursive: true });
    writeFileSync(join(pkg, 'package.json'), JSON.stringify({ name: 'fixture', version: '1.0.0', license: 'MIT' }));
    writeFileSync(join(pkg, 'dist/package.json'), JSON.stringify({ type: 'commonjs' }));
    writeFileSync(join(pkg, 'LICENSE-MIT'), 'Fixture permission text');
    writeFileSync(join(pkg, 'vendor/NOTICE'), 'Vendored component copyright');
    const inventory = licenses(root);
    assert.equal(inventory.length, 1);
    assert.equal(inventory[0].files.length, 2);
    unlinkSync(join(pkg, 'LICENSE-MIT'));
    unlinkSync(join(pkg, 'vendor/NOTICE'));
    assert.throws(() => licenses(root), /runtime license text missing: fixture@1.0.0/);
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test('installed identity detects changed JS chunks, missing resources and added files', () => {
  const root = mkdtempSync(join(tmpdir(), 'gogoke-package-hash-'));
  try {
    const service = join(root, 'gogoke-service');
    mkdirSync(join(service, 'dist'), { recursive: true });
    mkdirSync(join(service, 'node_modules'), { recursive: true });
    const chunk = join(service, 'dist/chunk.mjs');
    writeFileSync(chunk, 'export const value = 1;');
    const names = ['dist/bin.mjs', 'runtime/node.exe', 'runtime/LICENSE', 'fixtures/controlled-pi.mjs', 'T3-LICENSE', 'T3-runtime-notices.html', 'native-host-notices.html'];
    for (const name of names) {
      const target = join(service, name);
      mkdirSync(join(target, '..'), { recursive: true });
      writeFileSync(target, 'package fixture');
    }
    writeFileSync(join(root, 'gogoke-native-host.exe'), 'package fixture');
    const files = Object.fromEntries([...names, 'gogoke-native-host.exe'].map((name) => [name, createHash('sha256').update('package fixture').digest('hex')]));
    const manifest = join(root, 'manifest.json');
    writeFileSync(manifest, JSON.stringify({ schema: 1, files, runtimePackages: [], externalRoots: [],
      nativeBinaries: [], nativeBinaryHashes: {}, packagesMissingNoticeFiles: [],
      noticeCoverage: { state: 'SOURCE_TEXT_COLLECTED_NOT_LEGAL_ACCEPTANCE' }, runtimeFileHashes: fileHashes(service) }));
    verify(root, manifest);
    const empty = join(root, 'empty.json');
    writeFileSync(empty, JSON.stringify({ schema: 1, files: {} }));
    assert.throws(() => verify(root, empty), /required package inventory incomplete/);
    writeFileSync(chunk, 'export const value = 2;');
    assert.throws(() => verify(root, manifest), /runtime closure byte mismatch/);
    unlinkSync(chunk);
    assert.throws(() => verify(root, manifest), /runtime closure byte mismatch/);
    writeFileSync(chunk, 'export const value = 1;');
    writeFileSync(join(service, 'unexpected.mjs'), 'not part of this build');
    assert.throws(() => verify(root, manifest), /runtime closure byte mismatch/);
  } finally { rmSync(root, { recursive: true, force: true }); }
});


test('upstream notice fallback requires an exact version and immutable repository commit', () => {
  const pkg = { name: '@fixture/package', version: '1.0.0' };
  const metadata = { ...pkg, gitHead: 'a'.repeat(40), repository: { url: 'git+https://github.com/fixture/source.git', directory: 'packages/component' } };
  assert.deepEqual(upstreamLicenseBasis(pkg, metadata), { repository: 'fixture/source', commit: 'a'.repeat(40), directory: 'packages/component' });
  assert.deepEqual(upstreamLicenseBasis(pkg, { ...metadata, repository: { url: 'git://github.com/fixture/source.git', directory: 'packages/component' } }), { repository: 'fixture/source', commit: 'a'.repeat(40), directory: 'packages/component' });
  assert.deepEqual(upstreamLicenseBasis(pkg, { ...metadata, repository: { url: 'git+ssh://git@github.com/fixture/source.git', directory: 'packages/component' } }), { repository: 'fixture/source', commit: 'a'.repeat(40), directory: 'packages/component' });
  const opencode = { name: '@opencode-ai/sdk', version: '1.18.32' };
  assert.deepEqual(upstreamLicenseBasis(opencode, { ...opencode }), {
    repository: 'anomalyco/opencode',
    commit: '0027387dc5c59793c12dfc531abc78f825ed6868',
    directory: 'packages/sdk/js',
  });
  for (const delta of [{gitHead: 'main'}, {gitHead: undefined}, {version: '2.0.0'},
    {repository: {url: 'https://example.com/source'}},
    {repository: {url: metadata.repository.url, directory: '../escape'}}]) {
    assert.throws(() => upstreamLicenseBasis(pkg, {...metadata, ...delta}));
  }
});

test('ffi platform notice pins cover only the verified package versions', () => {
  for (const platform of ['linux-x64-gnu', 'linux-x64-musl', 'win32-ia32-msvc', 'win32-x64-msvc']) {
    const pkg = { name: `@yuuang/ffi-rs-${platform}`, version: '1.3.2' };
    assert.deepEqual(upstreamLicenseBasis(pkg, { ...pkg }), {
      repository: 'zhangyuang/node-ffi-rs',
      commit: '9c5ba3452d8cedbb0b30c7ce4dd0138135bd06ea',
      directory: '',
    });
    assert.throws(() => upstreamLicenseBasis(pkg, { ...pkg, version: '1.3.3' }), /metadata mismatch/);
    const changed = { ...pkg, version: '1.3.3' };
    assert.throws(() => upstreamLicenseBasis(changed, { ...changed }), /immutable upstream license revision missing/);
  }
  const unpinned = { name: '@yuuang/ffi-rs-win32-arm64-msvc', version: '1.3.2' };
  assert.throws(() => upstreamLicenseBasis(unpinned, { ...unpinned }), /immutable upstream license revision missing/);
});
