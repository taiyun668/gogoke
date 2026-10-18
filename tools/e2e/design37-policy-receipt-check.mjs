// Security boundary checks against the actual shared USER bridge, with no native calls.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import crypto from 'node:crypto';
import vm from 'node:vm';
import { createRequire } from 'node:module';

const sourcePath = new URL('../../apps/desktop/src/services/tauri.ts', import.meta.url);
const source = fs.readFileSync(sourcePath, 'utf8');
const require = createRequire(new URL('../../apps/desktop/package.json', import.meta.url));
const ts = require('typescript');
const compiled = ts.transpileModule(source, { compilerOptions: {
  module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022,
} }).outputText;
let count = 0;
function bridge(command, mutation, present = false) {
  const calls = [];
  const module = { exports: {} };
  const context = vm.createContext({ module, exports: module.exports,
    crypto: { randomUUID: () => 'security-case' },
    require: (name) => name === '@tauri-apps/api/core' ? {
      invoke: async (route, { frame }) => {
        assert.equal(route, 'gogoke_design37_user_operation');
        const input = JSON.parse(frame); calls.push(input);
        if (input.command === 'policy-head-read') {
          return JSON.stringify({ schema: 'gogoke.37.project-policy-head.v1', domainId: input.domainId,
            ...(command === 'policy-stage-set-initial' || present || calls.length > 1
              ? { state: 'PRESENT', revision: command === 'policy-stage-set-initial' ? '7' : '1', currentStage: null }
              : { state: 'ABSENT' }) });
        }
        assert.equal(input.command, command);
        return JSON.stringify(mutation({ schema: input.schema, command: input.command,
          requestId: input.requestId, status: 'APPLIED', revision: command === 'policy-stage-set-initial' ? '8' : '1' }));
      },
    } : {},
  });
  vm.runInContext(compiled, context, { filename: sourcePath.pathname });
  return { api: module.exports, calls };
}
const invalid = [
  () => null, () => ({}),
  (reply) => ({ ...reply, schema: 'wrong' }),
  (reply) => ({ ...reply, command: 'wrong' }),
  (reply) => ({ ...reply, requestId: 'other' }),
  (reply) => ({ ...reply, status: ['APPLIED'] }),
  (reply) => ({ ...reply, status: 'UNKNOWN' }),
  (reply) => ({ ...reply, status: undefined }),
  (reply) => ({ ...reply, revision: '999' }),
];
for (const command of ['policy-metadata-initialize', 'policy-stage-set-initial']) {
  const run = (api) => command === 'policy-metadata-initialize'
    ? api.initializeDesign37ProjectPolicyMetadata('projectA')
    : api.setDesign37ProjectInitialStage('projectA', 'REVIEW');
  for (const mutation of invalid) {
    const { api, calls } = bridge(command, mutation);
    await assert.rejects(() => run(api));
    assert.equal(calls.filter((row) => row.command === command).length, 1, 'rejected receipt must not retry a write');
    count++;
  }
  for (const status of ['APPLIED', 'REPLAYED']) {
    const { api, calls } = bridge(command, (reply) => ({ ...reply, status }));
    await run(api);
    assert.equal(calls.filter((row) => row.command === command).length, 1);
    count++;
  }
}
const concurrent = bridge('policy-metadata-initialize', (reply) => ({
  schema: reply.schema, command: reply.command, requestId: reply.requestId,
  status: 'CONFLICT', reason: 'Conflict',
}));
await concurrent.api.initializeDesign37ProjectPolicyMetadata('projectA');
assert.equal(concurrent.calls.length, 3, 'proven conflict must only reread, not rewrite'); count++;
const existing = bridge('policy-metadata-initialize', () => { throw Error('read must never write'); }, true);
await existing.api.readDesign37ProjectPolicyHead('projectA');
await existing.api.initializeDesign37ProjectPolicyMetadata('projectA');
assert.equal(existing.calls.filter((row) => row.command !== 'policy-head-read').length, 0); count++;
console.log(JSON.stringify({ state: 'ACTUAL_USER_BRIDGE_RECEIPT_CHECKS_PASS', passed: count,
  sourceSha256: crypto.createHash('sha256').update(source).digest('hex'), native: 'NOT_RUN', acceptance: false }));
