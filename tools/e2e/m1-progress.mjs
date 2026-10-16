// Summarize a settled journal only. Observe a live run from its stdout log:
// opening the atomic journal target can prevent its replacement on Windows.
import { readJson } from './product-cdp.mjs';

const journal = readJson(process.argv[2]);
console.log(JSON.stringify({
  state: journal.state,
  error: journal.error ?? null,
  lastOperation: journal.operations?.at(-1)?.request?.operation ?? null,
  assertions: [...new Set(journal.assertions ?? [])],
  launches: journal.launches?.length ?? 0,
  closes: journal.closes?.length ?? 0,
  sessions: (journal.sessions ?? []).map(({ id, generation, turns }) => ({
    id, generation, turns: turns?.length ?? 0,
  })),
}, null, 2));
