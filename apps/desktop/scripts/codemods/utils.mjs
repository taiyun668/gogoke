import fs from "node:fs";
import path from "node:path";

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

export function parseCodemodArgs(argv = process.argv.slice(2)) {
  const files = [];
  let dryRun = false;

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--dry-run") {
      dryRun = true;
      continue;
    }
    if (arg === "--files") {
      const raw = argv[index + 1] ?? "";
      index += 1;
      files.push(...raw.split(",").map((entry) => entry.trim()).filter(Boolean));
      continue;
    }
    if (arg.startsWith("--files=")) {
      files.push(
        ...arg
          .slice("--files=".length)
          .split(",")
          .map((entry) => entry.trim())
          .filter(Boolean),
      );
    }
  }

  return { dryRun, files: Array.from(new Set(files)) };
}

export function importPathToDesignSystem(fromFilePath, designSystemModulePath) {
  const normalizedFromFilePath = fromFilePath.replace(/\\/g, "/");
  const normalizedTargetPath = designSystemModulePath.replace(/\\/g, "/");
  const fromDir = path.posix.dirname(normalizedFromFilePath);
  let relative = path.posix.relative(fromDir, normalizedTargetPath);
  if (!relative.startsWith(".")) {
    relative = `./${relative}`;
  }
  return relative;
}

export function ensureNamedImport(source, modulePath, requiredNames) {
  if (!requiredNames.length) {
    return source;
  }
  const escapedModule = escapeRegExp(modulePath);
  const importPattern = new RegExp(
    `^import\\s*\\{([^}]*)\\}\\s*from\\s*["']${escapedModule}["'];?`,
    "m",
  );
  const existingImport = source.match(importPattern);

  if (existingImport) {
    const existingNames = existingImport[1]
      .split(",")
      .map((entry) => entry.replace(/\s+/g, " ").trim())
      .filter(Boolean);
    const mergedNames = [...existingNames];
    for (const name of requiredNames) {
      if (!mergedNames.includes(name)) {
        mergedNames.push(name);
      }
    }
    if (mergedNames.length === existingNames.length) {
      return source;
    }
    const replacement = `import {\n  ${mergedNames.join(",\n  ")},\n} from "${modulePath}";`;
    return source.replace(importPattern, replacement);
  }

  const importLine = `import { ${requiredNames.join(", ")} } from "${modulePath}";\n`;
  const importStatementPattern = /^import[\s\S]*?;\n?/gm;
  let lastImport = null;
  for (const match of source.matchAll(importStatementPattern)) {
    lastImport = match;
  }
  if (!lastImport) {
    return `${importLine}${source}`;
  }
  const insertIndex = (lastImport.index ?? 0) + lastImport[0].length;
  return `${source.slice(0, insertIndex)}${importLine}${source.slice(insertIndex)}`;
}

export function normalizeClassTokens(source, resolveToken) {
  return source.replace(/className="([^"]+)"/g, (fullMatch, classValue) => {
    const tokens = classValue.split(/\s+/).filter(Boolean);
    const additionalTokens = [];
    for (const token of tokens) {
      const normalizedToken = resolveToken(token);
      if (!normalizedToken) {
        continue;
      }
      if (!tokens.includes(normalizedToken) && !additionalTokens.includes(normalizedToken)) {
        additionalTokens.push(normalizedToken);
      }
    }
    if (!additionalTokens.length) {
      return fullMatch;
    }
    return `className="${[...additionalTokens, ...tokens].join(" ")}"`;
  });
}

export function runCodemod({ name, defaultFiles, transform }) {
  const { dryRun, files } = parseCodemodArgs();
  const targetFiles = files.length ? files : defaultFiles;
  let changedCount = 0;
  const changedPaths = [];
  const missingPaths = [];

  for (const relativePath of targetFiles) {
    const filePath = path.resolve(relativePath);
    if (!fs.existsSync(filePath)) {
      missingPaths.push(relativePath);
      continue;
    }

    const original = fs.readFileSync(filePath, "utf8");
    const updated = transform(original, relativePath);
    if (updated === original) {
      continue;
    }

    changedCount += 1;
    changedPaths.push(relativePath);
    if (!dryRun) {
      fs.writeFileSync(filePath, updated, "utf8");
    }
  }

  const modeLabel = dryRun ? "dry-run" : "write";
  console.log(`[${name}] mode=${modeLabel} files=${targetFiles.length} changed=${changedCount}`);
  for (const changedPath of changedPaths) {
    console.log(`  - ${dryRun ? "would update" : "updated"} ${changedPath}`);
  }
  for (const missingPath of missingPaths) {
    console.warn(`  - missing ${missingPath}`);
  }

  if (missingPaths.length) {
    process.exitCode = 1;
  }
}
