// Pure business-fixture controls. No installed product, Native API, DB or model runs.
import test from 'node:test';
import assert from 'node:assert/strict';
import { hostSessionLocator, qualifyHistoricalHostBaseline,
  qualifyHostPolicyContinuation, qualifyHostPolicyContinuationReferences } from './m2-rules.mjs';

const domain = 'domainA', submitter = 'submitterA', reviewer = 'reviewerA';
const destination = 'destinationA', gateId = 'gateA', stage = 'stageA';
const sourceCommit = 'a'.repeat(40), fingerprint = `sha256:${'b'.repeat(64)}`;
const same = value => structuredClone(value);
const event = (eventId, operation, target, revision, state, detail = '') => ({
  domain_id: domain, event_id: eventId, operation, fingerprint,
  target_id: target, policy_revision: revision, state, detail,
});
const owner = (command, requestId, previous, revision, fields) => {
  const request = { schema: 'gogoke.37.owner-configuration.v1', command,
    domainId: domain, requestId, ...fields, expectedRevision: previous };
  const receipt = { command, requestId, revision,
    schema: request.schema, status: 'APPLIED' };
  return { kind: 'V08_OWNER_CONFIGURATION', request, rawFrame: JSON.stringify(request),
    receipt, rawReceipt: JSON.stringify(receipt) };
};
const action = (operation, requestId, previous, revision, state, seatId, payload, reason = '') => {
  const receipt = { family: 'K-POLICY', operation, previousRevision: previous, requestId,
    result: { policyRevision: '39', reason, state }, revision,
    schema: 'gogoke.37.operations.v1', status: 'APPLIED', targetId: gateId };
  return { binding: { seatId }, cliToolStatus: 'completed',
    state: 'OBSERVED_NATIVE_RECEIPT_READBACK_REQUIRED',
    arguments: { operation, targetId: gateId, expectedRevision: previous, payload },
    receipt, rawToolReceipt: JSON.stringify(receipt) };
};

function fixture({ existingRoute = false } = {}) {
  const route = owner('policy-escalation-route', 'ownerRoute', '37', '38', {
    fromSeatId: submitter, reason: 'REJECT_CAP', toSeatId: destination });
  const gate = owner('policy-gate', 'ownerGate', '38', '39', {
    gateId, submitterSeatId: submitter, reviewerSeatId: reviewer,
    fromStage: stage, toStage: 'hostStage', rejectCap: 1 });
  const submit = action('gate-submit', 'modelSubmit', '1', '2', 'SUBMITTED', submitter, {});
  const decide = action('gate-decide', 'modelReject', '2', '3',
    'ESCALATION_REQUIRED', reviewer, { decision: 'REJECT', reason: 'reasonA' }, 'reasonA');
  const locator = hostSessionLocator(domain, decide.receipt.requestId, '39');
  const oldRoute = existingRoute
    ? { domain_id: domain, from_seat_id: submitter, reason: 'REJECT_CAP',
      to_seat_id: 'oldDestination', revision: 20 }
    : { domain_id: domain, from_seat_id: 'anotherSubmitter', reason: 'REJECT_CAP',
      to_seat_id: 'olderDestination', revision: 20 };
  const prior = { schema: 'gogoke.37.private-m2-rules-readback.v1',
    state: 'IMPLEMENTED_CASES_HAVE_DIRECT_EVIDENCE_V08_INCOMPLETE',
    directCaseEvidence: true, domainId: domain, databasePath: 'C:/root/state.sqlite',
    rootIdentity: { device: '1', inode: '2' },
    policy: { head: [{ domain_id: domain, revision: 37, current_stage: stage }],
      grants: [{ id: 'grantA' }], gates: [{ gate_id: 'oldGate', state: 'OLD' }],
      routes: [oldRoute], triggers: [],
      escalations: [{ request_id: 'oldEscalation', state: 'INTENT' }],
      events: [event('oldEvent', 'old-operation', 'oldTarget', 37, 'APPLIED')] },
    inbox: { messages: [{ message_id: 'oldMessage', state: 'CANCELLED' }],
      operations: [{ message_id: 'oldMessage', result_state: 'CANCELLED' }] } };
  const sealed = { schema: prior.schema, phase: 'before',
    state: 'BASELINE_ONLY_NOT_A_V08_RESULT', directCaseEvidence: false,
    domainId: domain, sourceCommit, readerSha256: 'c'.repeat(64),
    databasePath: prior.databasePath, rootIdentity: same(prior.rootIdentity),
    measurementPreservedDatabaseBytes: true, databaseWrites: false, credentialReads: false,
    normalClose: { exitCode: 0, forceKill: false },
    capacityQualification: { state: 'CAPACITY_4_WITHIN_REFERENCE_OWNER_AND_HOST',
      referenceDomainId: 'referenceA', referenceOwnerCap: 4, hostMachineLimit: 16,
      testDomainId: domain, testDomainOwnerCap: 4, effectiveTestDomainCap: 4 },
    policy: same(prior.policy), inbox: same(prior.inbox) };
  sealed.policy.head[0].revision = 39;
  sealed.policy.gates.push({ domain_id: domain, gate_id: gateId,
    submitter_seat_id: submitter, reviewer_seat_id: reviewer,
    from_stage: stage, to_stage: 'hostStage', reject_cap: 1,
    reject_count: 1, state: 'ESCALATION_REQUIRED', reason: 'reasonA', revision: 3 });
  const newRoute = { domain_id: domain, from_seat_id: submitter, reason: 'REJECT_CAP',
    to_seat_id: destination, revision: 38 };
  if (existingRoute) sealed.policy.routes[0] = newRoute;
  else sealed.policy.routes.push(newRoute);
  sealed.policy.escalations.push({ domain_id: domain, trigger_id: locator.trigger,
    request_id: locator.escalation, from_seat_id: submitter, to_seat_id: destination,
    reason: 'REJECT_CAP', state: 'INTENT', delivery_receipt_id: null, revision: 1 });
  sealed.policy.events.push(
    event(route.request.requestId, 'policy-escalation-route', domain, 38, 'APPLIED'),
    event(gate.request.requestId, 'policy-gate', domain, 39, 'APPLIED'),
    event(submit.receipt.requestId, 'gate-submit', gateId, 39, 'SUBMITTED'),
    event(decide.receipt.requestId, 'gate-decide', gateId, 39, 'ESCALATION_REQUIRED', 'reasonA'),
    event(locator.escalation, 'escalate', locator.trigger, 39, 'INTENT', decide.receipt.requestId));
  sealed.inbox.messages.push({ message_id: locator.messageId, state: 'PENDING',
    revision: '1', seat_id: destination, sender_seat_id: 'HOST_RULE' });
  sealed.inbox.operations.push({ message_id: locator.messageId,
    result_state: 'PENDING', phase: 'APPLIED' });
  const producer = { state: 'FAIL_ORIGINAL_REQUESTS_RETAINED', domainId: domain,
    closes: [{ exitCode: 0, forceKill: false }, { exitCode: 0, forceKill: false }],
    protectionComparison: { allEqual: true }, operations: [route, gate],
    rulesCases: [{ selection: 'HOST_BUSY_TO_IDLE_ONLY',
      state: 'FAILED_OR_NOT_RUN_PRESERVE_ORIGINAL', actions: [submit, decide],
      hostCases: [{ kind: 'BUSY_TO_IDLE_DELIVERY', state: 'ORIGINAL_CAP_OBSERVED',
        gateId, toStage: 'hostStage', sourceSeatId: submitter,
        causeEventId: decide.receipt.requestId, policyRevision: '39', routeRevision: '38',
        messageId: locator.messageId, queuedRevision: '1', destination: { seatId: destination } }] }] };
  const scope = { domainId: domain, sourceCommit,
    submitter: { seatId: submitter }, reviewer: { seatId: reviewer },
    capReferenceDomainId: 'referenceA' };
  return { before: same(sealed), prior, sealed, producer, scope };
}

test('sealed same-Root continuation preserves historical core13 and exact later Host effects', () => {
  for (const existingRoute of [false, true]) {
    const f = fixture({ existingRoute });
    assert.equal(qualifyHostPolicyContinuation(f.before, f.prior, f.sealed,
      f.producer, f.scope).state, 'EXACT_CLOSED_CURRENT_POLICY_BASELINE');
  }
});

test('without continuation the original historical-policy equality remains strict', () => {
  const f = fixture();
  const historicalBefore = { policy: same(f.prior.policy),
    capacityQualification: { state: 'CAPACITY_4_WITHIN_REFERENCE_OWNER_AND_HOST' } };
  assert.doesNotThrow(() => qualifyHistoricalHostBaseline(historicalBefore, f.prior, domain));
  assert.throws(() => qualifyHistoricalHostBaseline(f.before, f.prior, domain), /Host-only baseline/);
});

test('continuation accepts only the three original sealed source hashes', () => {
  const prior = { finalSha256: '0aacd901483e12de4a4de022acc748f6f8a693075d82d5d5fdf753c11e363adb' };
  const refs = { closedBaseline: { sha256: '574ec77d1ed9b12535f32d198a274d660cda179edf8162e3f12b5d0110caad2f' },
    producerJournal: { sha256: '76069651c7bda35b6535c74e0f3af69da3ae5f6c1be15bb80dbcbba905c3cfbd' } };
  assert.doesNotThrow(() => qualifyHostPolicyContinuationReferences(prior, refs));
  assert.throws(() => qualifyHostPolicyContinuationReferences({ finalSha256: 'f'.repeat(64) }, refs));
  assert.throws(() => qualifyHostPolicyContinuationReferences(prior,
    { ...refs, producerJournal: { sha256: 'f'.repeat(64) } }));
  assert.throws(() => qualifyHostPolicyContinuationReferences(prior,
    { ...refs, closedBaseline: { sha256: 'f'.repeat(64) } }));
});

test('unknown drift, wrong source and wrong native receipts all fail before Host actions', () => {
  const changes = [
    f => { f.before.policy.events.push(event('unknown', 'unknown', 'other', 39, 'APPLIED')); },
    f => { f.before.policy.events.push(event('unknown', 'unknown', 'other', 39, 'APPLIED'));
      f.sealed.policy.events.push(event('unknown', 'unknown', 'other', 39, 'APPLIED')); },
    f => { f.before.policy.gates[0].state = f.sealed.policy.gates[0].state = 'CHANGED'; },
    f => { f.before.policy.grants[0].id = f.sealed.policy.grants[0].id = 'CHANGED'; },
    f => { f.before.rootIdentity.inode = f.sealed.rootIdentity.inode = 'otherRoot'; },
    f => { f.producer.operations[0].rawFrame = '{}'; },
    f => { f.producer.rulesCases[0].actions[1].receipt.status = 'DENIED'; },
    f => { f.before.inbox.messages.at(-1).state = f.sealed.inbox.messages.at(-1).state = 'CANCELLED'; },
    f => { f.before.capacityQualification.testDomainOwnerCap =
      f.sealed.capacityQualification.testDomainOwnerCap = 3; },
    f => { f.before.policy.routes.at(-1).to_seat_id =
      f.sealed.policy.routes.at(-1).to_seat_id = 'unknownDestination'; },
    f => { f.before.sourceCommit = f.sealed.sourceCommit = 'd'.repeat(40); },
  ];
  for (const mutate of changes) {
    const f = fixture(); mutate(f);
    assert.throws(() => qualifyHostPolicyContinuation(f.before, f.prior, f.sealed,
      f.producer, f.scope));
  }
});
