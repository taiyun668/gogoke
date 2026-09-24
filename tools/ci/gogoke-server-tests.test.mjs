import assert from "node:assert/strict";
import { join } from "node:path";
import { test } from "node:test";
import { parseNodeTap, parseVitestReport } from "./gogoke-server-tests.mjs";

const target = join(process.cwd(), "fixture.test.ts");
const viteReport = (status = "passed") => ({
  numTotalTests: 2,
  numPassedTests: 2,
  numFailedTests: 0,
  numPendingTests: 0,
  numTodoTests: 0,
  testResults: [{ name: target, status }],
});
const tap = (tests, passed, failed) =>
  `1..${tests}\n# tests ${tests}\n# pass ${passed}\n# fail ${failed}\n# cancelled 0\n# skipped 0\n# todo 0\n`;

test("Vitest result requires exact files and framework-owned nonzero assertions", () => {
  assert.equal(parseVitestReport(viteReport(), [target], 0).state, "PASS");
  assert.equal(parseVitestReport(viteReport(), [], 0).state, "FAIL_INSTRUMENT");
  assert.equal(parseVitestReport(viteReport(), [target], null).state, "FAIL_INSTRUMENT");
  assert.equal(parseVitestReport({ ...viteReport(), numTotalTests: 0 }, [target], 0).state, "FAIL_INSTRUMENT");
  assert.equal(parseVitestReport({ ...viteReport(), numPassedTests: 0 }, [target], 0).state, "FAIL_INSTRUMENT");
  assert.equal(parseVitestReport({ ...viteReport(), numPassedTests: 1 }, [target], 0).state, "FAIL_INSTRUMENT");
  assert.equal(parseVitestReport({ ...viteReport(), numTodoTests: undefined }, [target], 0).state, "FAIL_INSTRUMENT");
  assert.equal(parseVitestReport({ ...viteReport(), numPendingTests: 1, numPassedTests: 1 }, [target], 0).state, "FAIL");
  assert.equal(parseVitestReport({ ...viteReport(), numTodoTests: 1, numPassedTests: 1 }, [target], 0).state, "FAIL");
  assert.equal(parseVitestReport(viteReport("failed"), [target], 0).state, "FAIL");
});

test("Node TAP result rejects missing summaries, zero tests and real failure", () => {
  assert.equal(parseNodeTap(tap(2, 2, 0), 1, 0).state, "PASS");
  assert.equal(parseNodeTap("", 1, 0).state, "FAIL_INSTRUMENT");
  assert.equal(parseNodeTap(tap(2, 2, 0), 1, null).state, "FAIL_INSTRUMENT");
  assert.equal(parseNodeTap(tap(0, 0, 0), 1, 0).state, "FAIL_INSTRUMENT");
  assert.equal(parseNodeTap(tap(2, 1, 1), 1, 1).state, "FAIL");
});
