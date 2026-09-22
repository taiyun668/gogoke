import * as NodeAssert from "node:assert/strict";
import * as NodeFS from "node:fs";
import * as NodePath from "node:path";
import * as NodeTest from "node:test";
import * as NodeURL from "node:url";

const assert: typeof NodeAssert = NodeAssert;
const test: typeof NodeTest.test = NodeTest.test;

const gogokeRoot = NodePath.resolve(NodeURL.fileURLToPath(new URL("../../", import.meta.url)));

function productionSources(directory: string): string[] {
  const found: string[] = [];
  for (const entry of NodeFS.readdirSync(directory, { withFileTypes: true })) {
    const path = NodePath.join(directory, entry.name);
    if (entry.isDirectory()) {
      found.push(...productionSources(path));
      continue;
    }
    if (!entry.name.endsWith(".ts") || entry.name.endsWith(".test.ts")) continue;
    found.push(path);
  }
  return found;
}

test("gogoke production TypeScript never constructs node:sqlite", () => {
  const offenders: string[] = [];
  for (const file of productionSources(gogokeRoot)) {
    const source = NodeFS.readFileSync(file, "utf8");
    if (
      source.includes("node:sqlite") ||
      source.includes("DatabaseSync") ||
      source.includes("nodeSqliteClient") ||
      source.includes("persistence/Layers/Sqlite")
    ) {
      offenders.push(NodePath.relative(gogokeRoot, file).replaceAll("\\", "/"));
    }
  }
  assert.deepEqual(offenders, []);
});
