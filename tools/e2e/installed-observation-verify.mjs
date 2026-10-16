// One runner for pinned, read-only instrument checks over retained real scenes.
import fs from 'node:fs';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const digest = file => createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const pin = ref => {
  if (!ref || !path.isAbsolute(ref.path) || !/^[a-f0-9]{64}$/.test(ref.sha256) || digest(ref.path) !== ref.sha256) {
    throw Error('Instrument/source reference changed');
  }
};
const inputPath = process.argv[2];
if (!inputPath || !path.isAbsolute(inputPath)) throw Error('Absolute pinned observation job required');
const input = JSON.parse(fs.readFileSync(inputPath, 'utf8').replace(/^\uFEFF/, ''));
if (input.schema !== 'gogoke.37.installed-observation-job.v1' || input.acceptance !== false ||
    input.runnerSha256 !== digest(fileURLToPath(import.meta.url)) || !Array.isArray(input.checks) ||
    !input.checks.length || !path.isAbsolute(input.outputDirectory)) throw Error('Invalid observation job');
const directory = input.outputDirectory;
fs.mkdirSync(directory, { recursive: true });
const resultPath = path.join(directory, 'instrument-validation.json');
if (fs.existsSync(resultPath)) throw Error('Observation job already ran; no implicit replay');
const results = [];
for (const check of input.checks) {
  if (!/^[a-z0-9-]+$/.test(check.id) || !['OBSERVED', 'REFUSED_ORIGINAL_SOURCE'].includes(check.expected) ||
      !Array.isArray(check.args) || !check.args.every(arg => typeof arg === 'string') ||
      !Array.isArray(check.sources) || !check.sources.length) throw Error('Invalid instrument check');
  pin(check.runtime); pin(check.instrument);
  const executable = path.basename(check.runtime.path).toLowerCase();
  const prefixes = {
    'node-test': ['node.exe', ['--test', '--test-reporter=tap', check.instrument.path]],
    'python-file': ['python.exe', ['-E', '-B', check.instrument.path]],
    'powershell-file': ['pwsh.exe', ['-NoProfile', '-NonInteractive', '-File', check.instrument.path]],
  };
  const route = prefixes[check.commandKind];
  if (!route || executable !== route[0] ||
      !route[1].every((arg, index) => check.args[index] === arg) ||
      (check.commandKind === 'node-test' && check.args.length !== route[1].length) ||
      (check.commandKind === 'python-file' && path.basename(check.instrument.path) !== 'installed_native_readback.py') ||
      (check.commandKind === 'powershell-file' && path.basename(check.instrument.path) !== 'capture-protection.ps1')) {
    throw Error('Pinned instrument is not the actual executable entry');
  }
  for (const source of check.sources) pin(source);
  const env = { ...process.env, TEMP: directory, TMP: directory, PYTHONDONTWRITEBYTECODE: '1' };
  delete env.NODE_OPTIONS;
  for (const [name, value] of Object.entries(check.environment ?? {})) {
    if (!name.startsWith('GOGOKE_') || typeof value !== 'string') throw Error('Invalid instrument environment');
    env[name] = value;
  }
  const execution = spawnSync(check.runtime.path, check.args, {
    cwd: directory, env, windowsHide: true, encoding: 'utf8', maxBuffer: 8 * 1024 * 1024,
  });
  const stdout = path.join(directory, `${check.id}.stdout.txt`);
  const stderr = path.join(directory, `${check.id}.stderr.txt`);
  fs.writeFileSync(stdout, execution.stdout ?? '', { flag: 'wx' });
  fs.writeFileSync(stderr, execution.stderr ?? '', { flag: 'wx' });
  const testTotals = check.commandKind === 'node-test'
    ? { pass: Number((execution.stdout ?? '').match(/# pass (\d+)/)?.[1]),
      skipped: Number((execution.stdout ?? '').match(/# skipped (\d+)/)?.[1]) } : null;
  const observed = execution.status === 0 && !execution.error &&
    (!testTotals || (Number.isInteger(check.expectedPassCount) && check.expectedPassCount > 0 &&
      testTotals.pass === check.expectedPassCount && testTotals.skipped === 0));
  const originalRefusal = Number.isInteger(execution.status) && execution.status !== 0 &&
    !execution.error && typeof check.originalFailureText === 'string' && check.originalFailureText &&
    (execution.stderr ?? '').includes(check.originalFailureText);
  const matched = check.expected === 'OBSERVED' ? observed : originalRefusal;
  results.push({ id: check.id, expected: check.expected, matched, exitCode: execution.status,
    error: execution.error ? String(execution.error) : null, testTotals, instrument: check.instrument,
    sources: check.sources, stdout: { path: stdout, sha256: digest(stdout) },
    stderr: { path: stderr, sha256: digest(stderr) } });
  pin(check.instrument);
  for (const source of check.sources) pin(source);
}
const report = { schema: 'gogoke.37.installed-instrument-validation.v1', acceptance: false,
  state: 'COMMON_INSTRUMENTS_EXISTING_SCENES_REVIEW_REQUIRED',
  runnerSha256: digest(fileURLToPath(import.meta.url)), inputSha256: digest(inputPath),
  allChecksMatched: results.every(row => row.matched), checks: results };
fs.writeFileSync(resultPath, JSON.stringify(report, null, 2) + '\n', { flag: 'wx' });
console.log(JSON.stringify({ state: report.state, allChecksMatched: report.allChecksMatched,
  checkCount: results.length, acceptance: false }));
if (!report.allChecksMatched) process.exitCode = 1;
