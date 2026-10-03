import assert from "node:assert/strict";
import { test } from "node:test";
import { join } from "node:path";
import { effectiveMemoryOff, isolatedProbeEnvironment } from "./processProbe.ts";

test("probe child environment excludes inherited credential variables", () => {
  const windows = process.env.SystemRoot ?? process.env.WINDIR;
  assert.ok(windows);
  const home = join(windows, "Temp", "isolated-codex-probe");
  const env = isolatedProbeEnvironment(home);
  assert.equal(env.CODEX_HOME, home);
  assert.equal(env.USERPROFILE, home);
  assert.equal(env.APPDATA, home);
  assert.equal(env.OPENAI_API_KEY, undefined);
  assert.equal(env.CODEX_API_KEY, undefined);
  assert.equal(env.GITHUB_TOKEN, undefined);
  assert.equal(env.PATH, join(windows, "System32"));
});

test("memory-off proof requires all three effective config values", () => {
  const config = { config: { features: { memories: false }, memories: {
    generate_memories: false, use_memories: false,
  } } };
  assert.equal(effectiveMemoryOff(config), true);
  assert.equal(effectiveMemoryOff({ config: { features: { memories: false } } }), false);
  assert.equal(effectiveMemoryOff({ config: { features: { memories: true }, memories: {
    generate_memories: false, use_memories: false,
  } } }), false);
  assert.equal(effectiveMemoryOff({}), false);
});
