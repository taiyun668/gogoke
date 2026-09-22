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
function isObject(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

// Tauri applies each --config file with JSON Merge Patch (RFC 7396): arrays
// replace the whole value, so a short app.windows override drops window fields.
function mergePatch(target, patch) {
  if (!isObject(patch)) return patch;
  const result = isObject(target) ? { ...target } : {};
  for (const [key, value] of Object.entries(patch)) {
    if (value === null) {
      delete result[key];
    } else {
      result[key] = mergePatch(result[key], value);
    }
  }
  return result;
}

function assert(condition, message) {
  if (!condition) failures.push(message);
}

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
const windowsConfig = JSON.parse(readFileSync(join(root, "src-tauri", "tauri.windows.conf.json"), "utf8"));
const unsignedConfig = JSON.parse(readFileSync(join(root, "src-tauri", "tauri.gogoke.unsigned.conf.json"), "utf8"));
const cargo = readFileSync(join(root, "src-tauri", "Cargo.toml"), "utf8");
const cargoVersion = cargo.match(/^version\s*=\s*"([^"]+)"/m)?.[1];
if (packageJson.name !== "gogoke") failures.push("package.json: wrong package identity");
if (tauriConfig.productName !== "gogoke") failures.push("tauri.conf.json: wrong product identity");
if (tauriConfig.identifier !== "app.gogoke.desktop") failures.push("tauri.conf.json: wrong app identifier");
if (packageJson.version !== tauriConfig.version || packageJson.version !== cargoVersion) {
  failures.push("package.json, tauri.conf.json, and Cargo.toml versions differ");
}
if (tauriConfig.plugins?.updater) failures.push("tauri.conf.json: inherited updater is still configured");

const mergedWindowsConfig = mergePatch(mergePatch(tauriConfig, windowsConfig), unsignedConfig);
const [mergedWindow] = mergedWindowsConfig.app?.windows ?? [];
assert(mergedWindowsConfig.productName === "gogoke", "merged Tauri config: wrong product identity");
assert(mergedWindowsConfig.identifier === "app.gogoke.desktop", "merged Tauri config: wrong app identifier");
assert(mergedWindow?.title === "gogoke", "merged Windows window: wrong title");
assert(mergedWindow?.titleBarStyle === "Visible", "merged Windows window: titleBarStyle must remain Visible");
assert(mergedWindow?.transparent === true, "merged Windows window: transparent must remain true");
assert(mergedWindow?.windowEffects === null, "merged Windows window: windowEffects must remain null");
assert(mergedWindow?.hiddenTitle === false, "merged Windows window: hiddenTitle must remain false");

if (failures.length > 0) {
  console.error(failures.join("\n"));
  process.exit(1);
}
console.log("PASS product identity: gogoke owns telemetry and release-channel strings");
