import { readFileSync, readdirSync, statSync } from "node:fs";
import { extname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("..", import.meta.url));
const allowedProvenance = new Set(["LICENSE", "THIRD_PARTY_NOTICES.md"]);
const ignoredDirectories = new Set(["node_modules", "dist", "target", ".git"]);
const textExtensions = new Set([
  ".css", ".html", ".js", ".json", ".md", ".mjs", ".nix", ".plist",
  ".rs", ".sh", ".toml", ".ts", ".tsx", ".xml", ".yml", ".yaml",
]);
const oldOwner = ["Dim", "illian"].join("");
const oldProduct = ["Codex", "Monitor"].join("");
const forbidden = [
  `github.com/${oldOwner}/${oldProduct}`,
  `api.github.com/repos/${oldOwner}/${oldProduct}`,
  `com.${oldOwner.toLowerCase()}.${oldProduct.toLowerCase()}`,
  oldProduct,
  "Codex" + " Monitor",
  "@" + "sentry/react",
];

const failures = [];
function visit(directory) {
  for (const name of readdirSync(directory)) {
    if (ignoredDirectories.has(name)) continue;
    const path = join(directory, name);
    const stat = statSync(path);
    if (stat.isDirectory()) {
      visit(path);
      continue;
    }
    const rel = relative(root, path).replaceAll("\\", "/");
    if (allowedProvenance.has(rel) || !textExtensions.has(extname(name))) continue;
    const source = readFileSync(path, "utf8");
    for (const needle of forbidden) {
      if (source.includes(needle)) failures.push(`${rel}: contains ${needle}`);
    }
  }
}

visit(root);
const packageJson = JSON.parse(readFileSync(join(root, "package.json"), "utf8"));
const tauriConfig = JSON.parse(readFileSync(join(root, "src-tauri", "tauri.conf.json"), "utf8"));
if (packageJson.name !== "gogoke") failures.push("package.json: wrong package identity");
if (tauriConfig.productName !== "gogoke") failures.push("tauri.conf.json: wrong product identity");
if (tauriConfig.identifier !== "app.gogoke.desktop") failures.push("tauri.conf.json: wrong app identifier");
if (tauriConfig.plugins?.updater) failures.push("tauri.conf.json: inherited updater is still configured");
if (failures.length > 0) {
  console.error(failures.join("\n"));
  process.exit(1);
}
console.log("PASS localization identity: gogoke owns product, telemetry, and release-channel strings");
