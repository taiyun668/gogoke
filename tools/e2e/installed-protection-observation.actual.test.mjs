import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { compareProtection, validateProtectionObservers } from './installed-protection-observation.mjs';
const archive = process.env.GOGOKE_PROTECTION_ACTUAL_D;
if (!archive) throw Error('Actual protection scene required; cannot skip');
const digest = file => createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const read = ref => {
  assert.equal(digest(ref.path), ref.sha256);
  return JSON.parse(fs.readFileSync(ref.path, 'utf8').replace(/^\uFEFF/, ''));
};
const manifest = read({ path: path.join(archive, 'g37-v07-partial-first-close-capture-v2-prep-20261013/partial-first-close-capture-manifest.json'),
  sha256: '27db41d5e8d02cb4f06c186cdef85c2b11c05ee8cbe18448e7b329cc52891e7f' });
const before = read(manifest.caseBefore);
const observers = read({ path: path.join(archive, 'g37-v11-host-next-candidate-prep-20261013/v11.binding.json'),
  sha256: '847280d7f5c22d524947600376a861644360fd4ff23b6d22ddbe4490b4534dc7' }).observers;
const values = (name, phase) => read((phase === 'before' ? before.snapshots : manifest.snapshots)[name]);
test('shared Node protection compares all 13 actual retained measurements', () => {
  const result = compareProtection(observers, values);
  assert.equal(result.measuredFieldCount, 13);
  assert.equal(result.allEqual, true);
  assert.equal(result.acceptance, false);
});
test('missing and duplicated fields cannot become a successful protection result', () => {
  const wrong = structuredClone(observers);
  wrong[0].equalFields[1] = wrong[0].equalFields[0];
  assert.throws(() => validateProtectionObservers(wrong), /Exact original thirteen/);
  assert.throws(() => compareProtection(observers, (name, phase) => {
    const value = values(name, phase);
    if (name === 'formal' && phase === 'after') delete value.formal;
    return value;
  }), /Missing original formal.formal/);
});
