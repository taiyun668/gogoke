#!/usr/bin/env node
// Generate reviewable production npm license/copyright text from the locked install.
import { createHash } from "node:crypto";
import { readFileSync, readdirSync, statSync, mkdirSync, writeFileSync } from "node:fs";
import { dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const NOTICE_NAME = /^(LICEN[CS]E|COPYING|NOTICE|COPYRIGHT|AUTHORS)([._-].*)?$/iu;
const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "../../apps/desktop");

function escapeHtml(value) {
  return String(value).replaceAll("&", "&amp;").replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;").replaceAll('"', "&quot;");
}

function lruMapLicense(packageDir, entry) {
  if (entry.name !== "lru_map" || entry.versionInfo !== "0.4.1") return null;
  const readme = readFileSync(join(packageDir, "README.md"), "utf8");
  const marker = "# MIT license";
  const offset = readme.indexOf(marker);
  return offset < 0 ? null : { name: "README.md#MIT license", text: readme.slice(offset).trim() };
}

export function collectProductionNotices(sbom, desktopRoot = ROOT) {
  if (!Array.isArray(sbom?.packages) || sbom.packages.length < 2) {
    throw new Error("FAIL_INSTRUMENT: production SPDX package set is empty");
  }
  const rootPackage = JSON.parse(readFileSync(join(desktopRoot, "package.json"), "utf8"));
  if (rootPackage.name !== "gogoke" || typeof rootPackage.version !== "string") {
    throw new Error("FAIL_INSTRUMENT: gogoke root package identity is invalid");
  }
  const nodeModules = resolve(desktopRoot, "node_modules");
  const seen = new Set();
  const notices = [];
  let rootCount = 0;
  for (const entry of sbom.packages) {
    if (entry.name === rootPackage.name && entry.versionInfo === rootPackage.version &&
        entry.packageFileName === "") {
      rootCount += 1;
      continue;
    }
    if (typeof entry.name !== "string" || typeof entry.versionInfo !== "string" ||
        typeof entry.licenseDeclared !== "string" ||
        !entry.licenseDeclared || entry.licenseDeclared === "NOASSERTION" ||
        typeof entry.packageFileName !== "string" || isAbsolute(entry.packageFileName)) {
      throw new Error(`FAIL_INSTRUMENT: incomplete production package metadata: ${entry.name ?? "unknown"}`);
    }
    const packageDir = resolve(desktopRoot, entry.packageFileName);
    if (!packageDir.startsWith(nodeModules + sep) || !entry.packageFileName.includes("node_modules")) {
      throw new Error(`FAIL_INSTRUMENT: package path escapes installed dependency root: ${entry.name}`);
    }
    if (seen.has(packageDir)) throw new Error(`FAIL_INSTRUMENT: duplicate package path: ${entry.name}`);
    seen.add(packageDir);
    const candidates = readdirSync(packageDir).filter((name) => NOTICE_NAME.test(name))
      .map((name) => ({ name, path: join(packageDir, name) }))
      .filter(({ path }) => statSync(path).isFile())
      .map(({ name, path }) => ({ name, text: readFileSync(path, "utf8").trim() }));
    if (candidates.length === 0) {
      const fallback = lruMapLicense(packageDir, entry);
      if (fallback) candidates.push(fallback);
    }
    if (candidates.length === 0 || candidates.some(({ text }) => !text || text.includes("\uFFFD"))) {
      throw new Error(`FAIL_INSTRUMENT: license/copyright text missing or unreadable: ${entry.name}@${entry.versionInfo}`);
    }
    notices.push({ name: entry.name, version: entry.versionInfo,
      license: entry.licenseDeclared, packagePath: relative(desktopRoot, packageDir).replaceAll("\\", "/"),
      sources: candidates });
  }
  if (rootCount !== 1) throw new Error("FAIL_INSTRUMENT: gogoke root SPDX identity is missing or duplicate");
  if (notices.length < 100) throw new Error(`FAIL_INSTRUMENT: only ${notices.length} production npm packages`);
  return notices.sort((a, b) => a.name.localeCompare(b.name) || a.version.localeCompare(b.version));
}

export function renderNotices(notices, lockHash) {
  return `<!doctype html>\n<html lang="en"><meta charset="utf-8"><title>gogoke npm production notices</title>\n` +
    `<h1>gogoke npm production dependency notices</h1>\n` +
    `<p>Generated from the locked production dependency tree. package-lock SHA-256: <code>${escapeHtml(lockHash)}</code>. ` +
    `This inventory is not a legal conclusion.</p>\n` +
    `<p>Packages: ${notices.length}</p>\n` +
    notices.map((item) => `<section><h2>${escapeHtml(item.name)}@${escapeHtml(item.version)}</h2>\n` +
      `<p>Declared license: ${escapeHtml(item.license)}; installed path: ${escapeHtml(item.packagePath)}</p>\n` +
      item.sources.map((source) => `<h3>${escapeHtml(source.name)}</h3><pre>${escapeHtml(source.text)}</pre>\n`).join("") +
      `</section>\n`).join("") + `</html>\n`;
}

function main() {
  const [sbomPath, outputPath] = process.argv.slice(2);
  if (!sbomPath || !outputPath) throw new Error("usage: generate-npm-notices.mjs <spdx-json> <output-html>");
  const sbom = JSON.parse(readFileSync(sbomPath, "utf8"));
  const notices = collectProductionNotices(sbom);
  const lockHash = createHash("sha256").update(readFileSync(join(ROOT, "package-lock.json"))).digest("hex");
  mkdirSync(dirname(outputPath), { recursive: true });
  writeFileSync(outputPath, renderNotices(notices, lockHash), "utf8");
  console.log(JSON.stringify({ state: "PASS", package_count: notices.length, lock_sha256: lockHash,
    notice_sources: notices.reduce((sum, item) => sum + item.sources.length, 0) }));
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main();
