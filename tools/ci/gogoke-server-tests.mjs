#!/usr/bin/env node
// Cloud construction evidence for every Gogoke server test file, not a due-check runner.
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const donorRoot = join(repoRoot, "third_party/t3code");
const serverRoot = join(donorRoot, "apps/server");
const testRoot = join(serverRoot, "src/gogoke");

function testFiles(directory) {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = join(directory, entry.name);
    return entry.isDirectory() ? testFiles(path) : entry.isFile() && entry.name.endsWith(".test.ts") ? [path] : [];
  }).sort();
}

function count(value) {
  return Number.isSafeInteger(value) && value >= 0 ? value : null;
}

export function parseVitestReport(report, expectedFiles, exitCode) {
  const tests = count(report.numTotalTests);
  const passed = count(report.numPassedTests);
  const failed = count(report.numFailedTests);
  const pending = count(report.numPendingTests);
  const todo = count(report.numTodoTests ?? 0);
  const actualFiles = Array.isArray(report.testResults) ? report.testResults : [];
  const expected = new Set(expectedFiles.map((path) => resolve(path).toLowerCase()));
  const actual = new Set(actualFiles.map((entry) => resolve(entry.name ?? "").toLowerCase()));
  const failedFiles = actualFiles.filter((entry) => entry.status !== "passed").map((entry) => entry.name);
  const filesMatch = actualFiles.length === expectedFiles.length && expected.size === actual.size &&
    [...expected].every((path) => actual.has(path));
  const instrumentOk = Number.isSafeInteger(exitCode) && filesMatch && tests !== null && passed !== null && failed !== null &&
    pending !== null && todo !== null && tests > 0 && passed + failed + pending + todo <= tests;
  const skipped = pending === null || todo === null ? null : pending + todo;
  return {
    test_files: actualFiles.length,
    discovered: tests,
    executed: passed === null || failed === null ? null : passed + failed,
    passed,
    failed,
    skipped,
    failed_files: failedFiles,
    exit_code: exitCode,
    state: !instrumentOk ? "FAIL_INSTRUMENT" : exitCode !== 0 || failed > 0 || skipped > 0 ||
      failedFiles.length > 0 ? "FAIL" : "PASS",
  };
}

function lastSummary(text, key) {
  const values = [...text.matchAll(new RegExp(`^# ${key} (\\d+)\\r?$`, "gm"))];
  return values.length ? Number(values.at(-1)[1]) : null;
}

export function parseNodeTap(text, fileCount, exitCode) {
  const discovered = lastSummary(text, "tests");
  const passed = lastSummary(text, "pass");
  const failed = lastSummary(text, "fail");
  const skipped = lastSummary(text, "skipped");
  const cancelled = lastSummary(text, "cancelled");
  const todo = lastSummary(text, "todo");
  const instrumentOk = Number.isSafeInteger(exitCode) && fileCount > 0 && [discovered, passed, failed, skipped, cancelled, todo]
    .every((value) => count(value) !== null) && discovered > 0 &&
    passed + failed + skipped + cancelled + todo === discovered;
  return {
    test_files: fileCount,
    discovered,
    executed: passed === null || failed === null ? null : passed + failed,
    passed,
    failed,
    skipped: skipped === null || cancelled === null || todo === null ? null : skipped + cancelled + todo,
    exit_code: exitCode,
    state: !instrumentOk ? "FAIL_INSTRUMENT" : exitCode !== 0 || failed > 0 ||
      skipped + cancelled + todo > 0 ? "FAIL" : "PASS",
  };
}

function run(args, cwd, logPath, env = process.env) {
  const timeoutMs = 120_000;
  const result = spawnSync(process.execPath, args, { cwd, env, encoding: "utf8", maxBuffer: 64 * 1024 * 1024,
    timeout: timeoutMs });
  writeFileSync(logPath, `${result.stdout ?? ""}${result.stderr ?? ""}`, "utf8");
  return { exitCode: result.status, error: result.error?.message ?? null,
    timedOut: result.error?.code === "ETIMEDOUT", timeoutMs };
}

function main() {
  const evidenceRoot = process.env.GOGOKE_SERVER_EVIDENCE_ROOT;
  if (!evidenceRoot) throw new Error("GOGOKE_SERVER_EVIDENCE_ROOT is required");
  mkdirSync(evidenceRoot, { recursive: true });
  const files = testFiles(testRoot);
  const nodeFiles = files.filter((path) => readFileSync(path, "utf8").includes("node:test"));
  const viteFiles = files.filter((path) => !nodeFiles.includes(path));
  const reportPath = join(evidenceRoot, "vitest-report.json");
  writeFileSync(reportPath, "", "utf8");
  const viteLog = join(evidenceRoot, "vitest.log");
  const viteBin = join(donorRoot, "node_modules/vite-plus/bin/vp");
  const viteRun = run([viteBin, "test", "run", ...viteFiles.map((path) => relative(serverRoot, path)),
    "--reporter=json", `--outputFile=${reportPath}`], serverRoot, viteLog);
  let vite;
  try {
    vite = parseVitestReport(JSON.parse(readFileSync(reportPath, "utf8")), viteFiles, viteRun.exitCode);
  } catch (error) {
    vite = { test_files: viteFiles.length, discovered: 0, executed: 0, passed: 0, failed: 0,
      skipped: 0, exit_code: viteRun.exitCode, state: "FAIL_INSTRUMENT", error: String(error) };
  }
  vite.command = "node vite-plus/bin/vp test run <all Vite test files> --reporter=json";
  vite.log = "vitest.log";
  vite.timeout_ms = viteRun.timeoutMs;
  vite.timed_out = viteRun.timedOut;
  if (viteRun.error) vite.spawn_error = viteRun.error;

  const nodeLog = join(evidenceRoot, "node-tap.log");
  const host = process.env.GOGOKE_NATIVE_HOST ?? "";
  const nodeRun = run(["--experimental-strip-types", "--experimental-sqlite", "--test",
    "--test-reporter=tap", ...nodeFiles], donorRoot, nodeLog);
  const node = parseNodeTap(readFileSync(nodeLog, "utf8"), nodeFiles.length, nodeRun.exitCode);
  node.command = "node --experimental-strip-types --experimental-sqlite --test --test-reporter=tap <all node:test files>";
  node.log = "node-tap.log";
  node.timeout_ms = nodeRun.timeoutMs;
  node.timed_out = nodeRun.timedOut;
  node.native_host_bound = host.length > 0 && existsSync(host);
  if (nodeRun.error) node.spawn_error = nodeRun.error;
  if (!node.native_host_bound) node.state = "FAIL_INSTRUMENT";

  const selectionOk = files.length === 51 && nodeFiles.length === 10 && viteFiles.length === 41;
  const categories = [vite, node];
  const result = {
    schema: "gogoke.server-cloud-tests.v1",
    commit_sha: process.env.GITHUB_SHA ?? "LOCAL_DIAGNOSTIC",
    run_id: process.env.GITHUB_RUN_ID ?? "LOCAL_DIAGNOSTIC",
    run_attempt: process.env.GITHUB_RUN_ATTEMPT ?? "LOCAL_DIAGNOSTIC",
    runner_os: process.env.RUNNER_OS ?? process.platform,
    evidence_scope: "WINDOWS_SERVER_REAL_RUN_NOT_WINDOWS_11_DESKTOP",
    node_version: process.version,
    selected_files: { total: files.length, vite: viteFiles.length, node: nodeFiles.length,
      paths: files.map((path) => relative(repoRoot, path).replaceAll("\\", "/")) },
    vite,
    node,
    totals: {
      discovered: categories.reduce((sum, item) => sum + (item.discovered ?? 0), 0),
      executed: categories.reduce((sum, item) => sum + (item.executed ?? 0), 0),
      passed: categories.reduce((sum, item) => sum + (item.passed ?? 0), 0),
      failed: categories.reduce((sum, item) => sum + (item.failed ?? 0), 0),
      skipped: categories.reduce((sum, item) => sum + (item.skipped ?? 0), 0),
    },
    state: !selectionOk || categories.some((item) => item.state === "FAIL_INSTRUMENT") ?
      "FAIL_INSTRUMENT" : categories.some((item) => item.state !== "PASS") ? "FAIL" : "PASS",
  };
  writeFileSync(join(evidenceRoot, "machine-result.json"), `${JSON.stringify(result, null, 2)}\n`, "utf8");
  console.log(JSON.stringify({ state: result.state, selected_files: result.selected_files.total,
    totals: result.totals, vite: vite.state, node: node.state }));
  if (result.state !== "PASS") process.exitCode = 1;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main();
