// Embedded in the signed shell and preloaded before the product service entry.
import { registerHooks, isBuiltin } from 'node:module';
import { readFileSync, writeSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { isAbsolute, resolve } from 'node:path';

function installGuard(policyPath, expectedHash) {
  const bytes = readFileSync(policyPath);
  if (createHash('sha256').update(bytes).digest('hex') !== expectedHash) {
    throw new Error('GOGOKE_MODULE_POLICY_IDENTITY_MISMATCH');
  }
  const entries = JSON.parse(bytes.toString('utf8'));
  if (!Array.isArray(entries) || entries.length === 0 ||
      entries.some((entry) => typeof entry !== 'string' || !isAbsolute(entry))) {
    throw new Error('GOGOKE_MODULE_POLICY_INVALID');
  }
  const key = (path) => resolve(path).toLowerCase();
  const allowed = new Set(entries.map(key));
  if (allowed.size !== entries.length) {
    throw new Error('GOGOKE_MODULE_POLICY_DUPLICATE');
  }
  function deny() {
    writeSync(2, 'GOGOKE_MODULE_NOT_LISTED\n');
    process.exit(78);
  }
  function authorize(url) {
    if (isBuiltin(url)) return;
    let parsed;
    try { parsed = new URL(url); } catch { deny(); }
    if (parsed.protocol !== 'file:' || parsed.search || parsed.hash) deny();
    let path;
    try { path = fileURLToPath(parsed); } catch { deny(); }
    if (!allowed.has(key(path))) deny();
  }
  const originalDlopen = process.dlopen;
  process.dlopen = function guardedDlopen(module, filename, ...args) {
    if (typeof filename !== 'string' || !isAbsolute(filename) || !allowed.has(key(filename))) {
      deny();
    }
    return originalDlopen.call(this, module, filename, ...args);
  };
  registerHooks({
    resolve(specifier, context, nextResolve) {
      const result = nextResolve(specifier, context);
      authorize(result.url);
      return result;
    },
    load(url, context, nextLoad) {
      authorize(url);
      return nextLoad(url, context);
    },
  });
}
