// Shared three-observer/13-field capture and comparison; no work on import.
import fs from 'node:fs';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
const here = path.dirname(fileURLToPath(import.meta.url));
const fieldsPath = path.join(here, 'installed-protection-fields.json');
const digest = file => createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const read = file => JSON.parse(fs.readFileSync(file, 'utf8').replace(/^\uFEFF/, ''));
const fieldBytes = fs.readFileSync(fieldsPath);
const fields = JSON.parse(fieldBytes.toString('utf8'));
const loadedSources = { module: { path: fileURLToPath(import.meta.url), sha256: digest(fileURLToPath(import.meta.url)) },
  fieldMap: { path: fieldsPath, sha256: createHash('sha256').update(fieldBytes).digest('hex') } };
const require = (value, reason) => { if (!value) throw Error(reason); };
const canonical = value => JSON.stringify(sort(value));
function sort(value) {
  if (Array.isArray(value)) return value.map(sort);
  if (value && typeof value === 'object') return Object.fromEntries(Object.keys(value).sort().map(key => [key, sort(value[key])]));
  return value;
}
export function protectionSources() {
  for (const source of Object.values(loadedSources)) require(digest(source.path) === source.sha256, 'Loaded protection source changed');
  return structuredClone(loadedSources);
}
export function validateProtectionObservers(observers) {
  protectionSources();
  require(Array.isArray(observers) && observers.length === 3, 'Three original protection observers required');
  require(Object.keys(fields).sort().join(',') === 'formal,ledger,memory' &&
    fields.formal.length === 5 && fields.memory.length === 5 && fields.ledger.length === 3,
  'Shared protection field definition differs');
  require(new Set(observers.map(row => row.name)).size === 3, 'Duplicate protection observer');
  for (const row of observers) require(Array.isArray(row.equalFields) && fields[row.name] &&
    new Set(row.equalFields).size === row.equalFields.length &&
    canonical([...row.equalFields].sort()) === canonical([...fields[row.name]].sort()),
  'Exact original thirteen protection fields required');
}
export async function captureProtection(observers, selectedNames, fileFor, onSnapshot) {
  validateProtectionObservers(observers);
  for (const row of observers.filter(observer => selectedNames === null || selectedNames.includes(observer.name))) {
    const output = fileFor(row.name);
    require(path.isAbsolute(output) && !fs.existsSync(output), 'Fresh absolute protection snapshot required');
    require(typeof row.runtime === 'string' && path.isAbsolute(row.runtime) && Array.isArray(row.args) &&
      row.args.filter(arg => arg === '{output}').length === 1, 'Original observer runtime/output arguments required');
    const args = row.args.map(arg => arg === '{output}' ? output : arg);
    await new Promise((resolve, reject) => {
      const child = spawn(row.runtime, args, { windowsHide: true, stdio: ['ignore', 'ignore', 'pipe'] });
      let stderr = '';
      child.stderr.on('data', bytes => { stderr = (stderr + bytes).slice(-8192); });
      child.once('error', reject);
      child.once('exit', code => code === 0 ? resolve() : reject(Error(`Original ${row.name} observer exit=${code}: ${stderr}`)));
    });
    onSnapshot(row.name, { file: path.basename(output), sha256: digest(output) });
  }
  protectionSources();
}
export function readProtectionSnapshot(directory, reference) {
  require(reference && path.basename(reference.file) === reference.file && /^[a-f0-9]{64}$/.test(reference.sha256),
    'Original protection snapshot reference required');
  const file = path.join(directory, reference.file);
  require(digest(file) === reference.sha256, 'Original protection snapshot changed');
  return read(file);
}
export function compareProtection(observers, valueFor) {
  validateProtectionObservers(observers);
  const measured = [];
  for (const row of observers) {
    const before = valueFor(row.name, 'before'), after = valueFor(row.name, 'after');
    for (const field of fields[row.name]) {
      require(Object.hasOwn(before, field) && Object.hasOwn(after, field), `Missing original ${row.name}.${field}`);
      measured.push({ group: row.name, field, equal: canonical(before[field]) === canonical(after[field]) });
    }
  }
  return { acceptance: false, measuredFieldCount: measured.length,
    allEqual: measured.every(row => row.equal), fields: measured, loadedSources: protectionSources() };
}
