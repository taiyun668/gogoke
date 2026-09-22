import { expect, it } from "@effect/vitest";
import * as fs from "node:fs";
import * as path from "node:path";
import * as url from "node:url";

const root = path.dirname(url.fileURLToPath(import.meta.url));
const implementationFiles = ["types.ts", "ports.ts", "fake.ts", "registry.ts", "index.ts"];

it("keeps the native adapter boundary free of product authority and direct side effects", () => {
  const source = implementationFiles
    .map((name) => fs.readFileSync(path.join(root, name), "utf8"))
    .join("\n");
  expect(source).not.toMatch(
    /from\s+["'][^"']*(?:room|persistentseat|persistence|store|account|network)[^"']*["']/iu,
  );
  expect(source).not.toMatch(/\b(?:spawn|fork|exec|fetch|createServer)\s*\(/u);
  expect(source).not.toMatch(/\bprocess\.env\b/u);
  expect(source).not.toMatch(/from\s+["']node:(?:child_process|net|http|https|sqlite)["']/u);
});

it("keeps the public boundary provider-neutral", () => {
  const index = fs.readFileSync(path.join(root, "index.ts"), "utf8");
  expect(index).toContain('"./types.ts"');
  expect(index).toContain('"./ports.ts"');
  expect(index).toContain('"./registry.ts"');
  expect(index).toContain('"./fake.ts"');
  expect(index).not.toMatch(/codex|claude|grok|pi|antigravity|gemini/iu);
});
