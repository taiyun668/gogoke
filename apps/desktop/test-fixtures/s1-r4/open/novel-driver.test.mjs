import assert from "node:assert/strict";
import { createHash, randomBytes } from "node:crypto";
import { readdir, readFile, stat } from "node:fs/promises";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { runNovelDriverConformance } from "../../../../../third_party/t3code/apps/server/src/gogoke/adapters/conformance/novel.ts";

const fixtureDirectory = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(fixtureDirectory, "../../../../..");
const runtimeCatalogRoot = path.join(
  repositoryRoot,
  "third_party/t3code/apps/server/src/gogoke/runtimeCatalog",
);
const migrationRoot = path.join(
  repositoryRoot,
  "third_party/t3code/apps/server/src/gogoke/migrations",
);

async function filesBelow(root) {
  const result = [];
  for (const entry of await readdir(root, { withFileTypes: true })) {
    const target = path.join(root, entry.name);
    if (entry.isDirectory()) result.push(...(await filesBelow(target)));
    else if (entry.isFile()) result.push(target);
  }
  return result.sort();
}

async function treeDigest(root) {
  const hash = createHash("sha256");
  for (const file of await filesBelow(root)) {
    hash.update(path.relative(root, file).replaceAll("\\", "/"));
    hash.update("\0");
    hash.update(await readFile(file));
    hash.update("\0");
  }
  return `sha256:${hash.digest("hex")}`;
}

test("post-build novel driver remains open and does not rewrite migration sources", async () => {
  assert.equal((await stat(runtimeCatalogRoot)).isDirectory(), true);
  const coreBuildDigest = await treeDigest(runtimeCatalogRoot);
  const migrationBefore = await treeDigest(migrationRoot);

  const first = runNovelDriverConformance(coreBuildDigest, randomBytes);
  const second = runNovelDriverConformance(coreBuildDigest, randomBytes);

  assert.match(first.driverId, /^mock_novel_[0-9a-f]{16}$/u);
  assert.match(second.driverId, /^mock_novel_[0-9a-f]{16}$/u);
  assert.notEqual(first.driverId, second.driverId);
  assert.equal(first.reinstalled, true);
  assert.equal(first.switchedVersion, "2.0.0");
  assert.equal(await treeDigest(migrationRoot), migrationBefore);
});
