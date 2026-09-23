import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, writeFileSync, rmSync, unlinkSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { collectProductionNotices, renderNotices } from "./generate-npm-notices.mjs";

test("production notice generator rejects missing text, unknown license and path escape", () => {
  const root = mkdtempSync(join(tmpdir(), "gogoke-npm-notice-test-"));
  try {
    const rootId = "SPDXRef-Package-gogoke-current";
    writeFileSync(join(root, "package.json"), JSON.stringify({ name: "gogoke", version: "9.9.9" }), "utf8");
    const packages = [{ name: "gogoke", versionInfo: "9.9.9", SPDXID: rootId, packageFileName: "" }];
    for (let index = 0; index < 100; index += 1) {
      const name = `fixture-${index}`;
      const packageFileName = `node_modules/${name}`;
      const dir = join(root, packageFileName);
      mkdirSync(dir, { recursive: true });
      writeFileSync(join(dir, "LICENSE"), `Copyright fixture ${index}\nMIT license text`, "utf8");
      packages.push({ name, versionInfo: "1.0.0", licenseDeclared: "MIT", packageFileName });
    }
    const sbom = { documentDescribes: [rootId], packages };
    const notices = collectProductionNotices(sbom, root);
    assert.equal(notices.length, 100);
    assert.match(renderNotices(notices, "abc"), /Copyright fixture 99/);
    assert.throws(() => collectProductionNotices({ ...sbom, documentDescribes: ["wrong"] }, root), /root package disagrees/);
    assert.throws(() => collectProductionNotices({ ...sbom, documentDescribes: [] }, root), /exactly one root/);

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
