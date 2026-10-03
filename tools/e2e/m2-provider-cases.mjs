// Boundaries for already logged-in real providers. Importing this module
// starts no process, model turn, login, browser or credential inspection.
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { id, sha256 } from './product-cdp.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const drivers = ['claude', 'opencode', 'grok'];
const atom = value => typeof value === 'string' && /^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/.test(value);

export async function runProviderBoundaryCases(product, config, journal) {
  const plan = config.providerBoundary;
  journal.sessions ??= [];
  journal.operations ??= [];
  journal.providerBoundaryCases ??= [];
  journal.providerBoundarySummary ??= { V03b: 'NOT_RUN_NOT_CONFIGURED',
    V04b: 'NOT_RUN_NOT_CONFIGURED', V10: 'NOT_RUN_NOT_CONFIGURED', acceptance: false };
  if (!plan) { product.save(); return journal.providerBoundarySummary; }
  if (process.platform !== 'win32' || config.repositoryId !== 'gogokeSeatTestbed' ||
      config.domainId !== product.config.domainId || config.instanceId !== 'codexTestM1' ||
      !product.tester || product.tester.page.url() !== product.endpoint.url ||
      journal.connectionBackend?.agentActs !== 0 ||
      journal.connectionBackend?.telemetryDisabled !== true ||
      !Array.isArray(plan.cases) || plan.cases.length !== 3 ||
      new Set(plan.cases.map(row => row.driverId)).size !== 3 ||
      plan.cases.some(row => !drivers.includes(row.driverId) ||
        ![row.instanceId, row.seatId, row.worktreeId].every(atom) ||
        typeof row.version !== 'string' || !/^[a-f0-9]{64}$/.test(row.sha256)) ||
      journal.providerBoundaryCases.length !== 0) {
    throw Error('Fresh same-domain real provider boundary preflight requires exact installed product and three fixed CLI cases');
  }
  journal.driverBytes ??= {};
  journal.driverBytes['m2-provider-cases.mjs'] = sha256(path.join(here, 'm2-provider-cases.mjs'));
  journal.driverBytes['m2-provider-readback.py'] = sha256(path.join(here, 'm2-provider-readback.py'));
  const checked = (condition, reason) => { if (!condition) throw Error(reason); };
  const operation = (family, action, target, payload, revision, allowed) =>
    product.operation(family, action, target, payload, revision, allowed);
  async function card(seatId) {
    let reply = await operation('K-SEAT', 'state-card', seatId, {}, '0', ['APPLIED', 'STALE']);
    if (reply.status === 'STALE') reply = await operation('K-SEAT', 'state-card', seatId, {}, reply.revision);
    return reply;
  }
  async function graph(worktreeId, seatId, instanceId) {
    let reply = await operation('K-WORKTREE', 'graph-query', worktreeId, {}, '0', ['APPLIED', 'STALE']);
    if (reply.status === 'STALE') reply = await operation('K-WORKTREE', 'graph-query', worktreeId, {}, reply.revision);
    checked(reply.result.state === 'REGISTERED' && reply.result.members?.some(row =>
      row.worktreeId === worktreeId && row.domainId === config.domainId &&
      row.repositoryId === config.repositoryId && row.seatId === seatId &&
      row.instanceId === instanceId), 'Original F graph does not bind provider seat/worktree');
    return reply;
  }
  const instances = await product.instances();
  for (const driverId of drivers) {
    const row = plan.cases.find(value => value.driverId === driverId);
    const observed = instances.instances.find(value => value.instanceId === row.instanceId);
    const record = { caseId: id('m2Provider'), driverId, instanceId: row.instanceId,
      seatId: row.seatId, worktreeId: row.worktreeId, domainId: config.domainId,
      expectedVersion: row.version, expectedDigest: `sha256:${row.sha256}`,
      observedInstance: observed ?? null, state: 'PREFLIGHT', acceptance: false,
      checks: { V03b: 'NOT_RUN', V04b: 'NOT_RUN', V10: 'NOT_RUN' },
      reasons: [], requestIds: [] };
    journal.providerBoundaryCases.push(record); product.save();
    if (!observed || observed.driverId !== driverId || observed.state !== 'LOGGED_IN') {
      record.state = 'NOT_RUN_NOT_LOGGED_IN';
      record.reasons.push('The original fixed instance is not already logged in; no login or substitute CLI is started.');
      product.save(); continue;
    }
    checked(observed.version === row.version, 'Actual instance version differs from private fixed pin');
    const seat = await card(row.seatId);
    checked(seat.result.state === 'IDLE' && seat.result.instanceId === row.instanceId,
      'Preconfigured real provider seat is not Idle on its pinned instance');
    record.graph = await graph(row.worktreeId, row.seatId, row.instanceId);
    const session = { id: id('m2ProviderSession'), caseOwner: record.caseId, seatId: row.seatId,
      instanceId: row.instanceId, worktreeId: row.worktreeId,
      generation: (BigInt(seat.result.generation) + 1n).toString(), revision: '0',
      cursor: '0', events: [], turns: [] };
    record.sessionId = session.id; journal.sessions.push(session); product.save();
    const step = async (action, payload = {}) => {
      const reply = await operation('K-SESSION', action, session.id,
        { generation: session.generation, ...payload }, session.revision);
      record.requestIds.push({ action, requestId: journal.operations.at(-1).request.requestId });
      session.revision = reply.revision; product.save(); return reply;
    };
    await step('admission-reserve', { seatId: row.seatId });
    await step('admission-commit', { seatId: row.seatId });
    const opened = await step('open', { seatId: row.seatId,
      repositoryId: config.repositoryId, worktreeId: row.worktreeId });
    session.threadId = opened.result.threadId ?? null; product.save();
    const capability = await step('capability-probe');
    checked(capability.result.driverId === driverId && capability.result.version === row.version &&
      capability.result.binaryDigest === record.expectedDigest &&
      capability.result.evidenceBasis !== undefined,
    'Original provider capability does not match the F/H fixed executable');
    record.capability = capability; product.save();
    const cardMode = capability.result.capabilities?.nativeQuestionCard;
    if (cardMode === 'UNSUPPORTED_REPLY_ENCODER') {
      record.reasons.push('V03b: this fixed provider has no native answer encoder; current User K-QCARD only answers Codex-native cards and has no provider-bound host-card raise path.');
    } else {
      record.reasons.push(`V03b: unqualified card capability ${JSON.stringify(cardMode)}; no provider question turn is induced.`);
    }
    if (capability.result.capabilities?.memoryOffLaunch !== 'LOADED_THREAD_MEMORIES_FALSE') {
      record.reasons.push('V04b: no fixed-provider effective memory-off proof, vendor memory-store readback, second-project original input, or instruction-load manifest.');
    } else {
      record.reasons.push('V04b: a launch flag alone cannot prove zero memory writes or cross-project instruction isolation.');
    }
    record.reasons.push('V10: this fixed-provider case has no independently observed same-seat SIDE_CHAT source and private marker. A fresh FORMAL_REVIEW open alone cannot prove input isolation; no WORK substitute or side marker is invented.');
    const stopped = await step('stop', { seatId: row.seatId });
    checked(typeof stopped.result.stopFact === 'string' && stopped.result.stopFact.length > 0,
      'Original provider process has no durable H stop fact');
    record.stopFact = stopped.result.stopFact;
    await step('admission-release', { seatId: row.seatId });
    record.state = 'REAL_FIXED_CAPABILITY_PREFLIGHT_ONLY'; product.save();
  }
  journal.providerBoundaryCases.push({ caseId: id('m2Provider'), driverId: 'antigravity',
    state: 'NOT_RUN_OWNER_DECISION_PENDING', acceptance: false,
    checks: { V03b: 'NOT_RUN', V04b: 'NOT_RUN', V10: 'NOT_RUN' },
    reasons: ['Fixed 1.2.11 still has shared Windows credential and no proven memory-off/instance login contract; no CLI, auth or model operation started.'] });
  journal.providerBoundarySummary = { V03b: 'NOT_RUN_NO_PROVIDER_BOUND_HOST_CARD',
    V04b: 'NOT_RUN_NO_EFFECTIVE_MEMORY_AND_CROSS_PROJECT_INPUT_PROOF',
    V10: 'NOT_RUN_NO_SAME_SEAT_PROVIDER_SIDE_SOURCE', acceptance: false,
    directReadbackRequired: true };
  product.save(); return journal.providerBoundarySummary;
}
