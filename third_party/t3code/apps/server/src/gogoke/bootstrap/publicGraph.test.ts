import * as NodeAssert from "node:assert/strict";
import * as NodeFS from "node:fs";
import * as NodePath from "node:path";
import * as NodeTest from "node:test";
import * as NodeURL from "node:url";

const assert: typeof NodeAssert = NodeAssert;
const test: typeof NodeTest.test = NodeTest.test;
const sourceRoot = NodePath.resolve(NodeURL.fileURLToPath(new URL("../", import.meta.url)));
const entry = NodePath.join(sourceRoot, "index.ts");

const importPattern = /(?:from\s*|import\s*)["']([^"']+)["']/gu;

function localDependencies(file: string): ReadonlyArray<string> {
  const source = NodeFS.readFileSync(file, "utf8");
  const dependencies: string[] = [];
  for (const match of source.matchAll(importPattern)) {
    const specifier = match[1];
    if (specifier === undefined || !specifier.startsWith(".")) continue;
    assert.equal(
      specifier.includes("persistence/Layers/Sqlite") || specifier.includes("nodeSqliteClient"),
      false,
      `forbidden persistence edge: ${file} -> ${specifier}`,
    );
    const resolved = NodePath.resolve(NodePath.dirname(file), specifier);
    assert.equal(
      NodeFS.existsSync(resolved) && NodeFS.statSync(resolved).isFile(),
      true,
      `unresolved production import: ${file} -> ${specifier}`,
    );
    dependencies.push(resolved);
  }
  return dependencies;
}

function reachableProductionGraph(): ReadonlySet<string> {
  const pending = [entry];
  const visited = new Set<string>();
  while (pending.length > 0) {
    const current = pending.pop()!;
    if (visited.has(current)) continue;
    visited.add(current);
    pending.push(...localDependencies(current));
  }
  return visited;
}

test("canonical Gogoke graph reaches the typed native store and no Node SQLite provider", () => {
  const graph = reachableProductionGraph();
  const relative = [...graph].map((file) =>
    NodePath.relative(sourceRoot, file).replaceAll("\\", "/"),
  );
  assert.equal(relative.includes("persistence/base/nativeHostClient.ts"), true);
  assert.equal(relative.includes("bootstrap/nativeStoreService.ts"), true);
  assert.equal(
    relative.some((file) => file.includes("persistence/Layers/Sqlite")),
    false,
  );
  assert.equal(
    relative.some((file) => file.endsWith("server.ts")),
    false,
  );

  const forbiddenImports: string[] = [];
  for (const file of graph) {
    const source = NodeFS.readFileSync(file, "utf8");
    if (
      source.includes('"node:sqlite"') ||
      source.includes("'node:sqlite'") ||
      source.includes("nodeSqliteClient") ||
      source.includes("persistence/Layers/Sqlite")
    ) {
      forbiddenImports.push(NodePath.relative(sourceRoot, file).replaceAll("\\", "/"));
    }
  }
  assert.deepEqual(forbiddenImports, []);
});
