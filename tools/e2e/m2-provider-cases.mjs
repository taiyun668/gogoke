// Boundaries for already logged-in real providers. Importing this module
// starts no process, model turn, login, browser or credential inspection.
import path from 'node:path';
import fs from 'node:fs';
import { fileURLToPath } from 'node:url';
import { id, sha256 } from './product-cdp.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const drivers = ['claude', 'opencode', 'grok'];
const atom = value => typeof value === 'string' && /^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/.test(value);
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const questionText = 'Which non-secret format should this marker use?';
const questionHeader = 'Format';
const questionOptions = [
  { label: 'JSON', description: 'Write the specified JSON marker.' },
  { label: 'Plain', description: 'Write plain text instead.' },
];

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
      !path.isAbsolute(config.testbedSource) || !path.isAbsolute(config.stateRoot) ||
      journal.providerBoundaryCases.length !== 0) {
    throw Error('Fresh same-domain real provider boundary preflight requires exact installed product and three fixed CLI cases');
  }
  journal.providerBoundaryConfig = plan.cases.map(row => ({driverId:row.driverId,
    crossProject:row.crossProject ? {repositoryId:row.crossProject.repositoryId,
      seatId:row.crossProject.seatId,worktreeId:row.crossProject.worktreeId} : null,
    reviewSource:row.reviewSource ? {repositoryId:config.repositoryId,
      seatId:row.reviewSource.seatId,instanceId:row.reviewSource.instanceId,
      worktreeId:row.reviewSource.worktreeId} : null}));
  product.save();
  journal.driverBytes ??= {};
  journal.driverBytes['m2-provider-cases.mjs'] = sha256(path.join(here, 'm2-provider-cases.mjs'));
  journal.driverBytes['m2-provider-readback.py'] = sha256(path.join(here, 'm2-provider-readback.py'));
  const checked = (condition, reason) => { if (!condition) throw Error(reason); };
  const operation = (family, action, target, payload, revision, allowed) =>
    product.operation(family, action, target, payload, revision, allowed);
  async function readAtRevision(family, action, target, payload, revision = '0') {
    let reply = await operation(family, action, target, payload, revision, ['APPLIED', 'STALE']);
    if (reply.status === 'STALE') reply = await operation(family, action, target, payload, reply.revision);
    return reply;
  }
  async function card(seatId) {
    let reply = await operation('K-SEAT', 'state-card', seatId, {}, '0', ['APPLIED', 'STALE']);
    if (reply.status === 'STALE') reply = await operation('K-SEAT', 'state-card', seatId, {}, reply.revision);
    return reply;
  }
  async function graph(worktreeId, seatId, instanceId, repositoryId = config.repositoryId) {
    let reply = await operation('K-WORKTREE', 'graph-query', worktreeId, {}, '0', ['APPLIED', 'STALE']);
    if (reply.status === 'STALE') reply = await operation('K-WORKTREE', 'graph-query', worktreeId, {}, reply.revision);
    checked(reply.result.state === 'REGISTERED' && reply.result.members?.some(row =>
      row.worktreeId === worktreeId && row.domainId === config.domainId &&
      row.repositoryId === repositoryId && row.seatId === seatId &&
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
    if (row.crossProject) {
      const other = row.crossProject;
      checked([other.repositoryId, other.seatId, other.worktreeId].every(atom) &&
        other.repositoryId !== config.repositoryId && other.seatId !== row.seatId &&
        other.worktreeId !== row.worktreeId,
      'V04b needs a second original F project and distinct seat on this same instance');
      const otherSeat = await card(other.seatId);
      checked(otherSeat.result.state === 'IDLE' && otherSeat.result.instanceId === row.instanceId,
        'V04b second original E seat is not Idle on the identical instance');
      record.crossProjectGraph = await graph(other.worktreeId, other.seatId,
        row.instanceId, other.repositoryId);
      record.reasons.push('V04b: two distinct original F project bindings on one instance are ready; effective vendor memory/instruction provenance and simultaneous original H inputs remain NOT_RUN.');
    } else {
      record.reasons.push('V04b: no second F registered testbed project, separate E seat and worktree are supplied for this same instance.');
    }
    if (row.reviewSource) {
      const source = row.reviewSource;
      checked([source.seatId, source.instanceId, source.worktreeId].every(atom) &&
        source.seatId !== row.seatId && source.worktreeId !== row.worktreeId,
      'V10 K-SIDE composition needs a distinct original source seat/worktree');
      checked(instances.instances.some(instance => instance.instanceId === source.instanceId &&
        instance.state === 'LOGGED_IN'), 'V10 source instance is not already logged in');
      const sourceSeat = await card(source.seatId);
      checked(sourceSeat.result.state === 'IDLE' && sourceSeat.result.instanceId === source.instanceId,
        'V10 original source seat is not ready for K-SIDE composition');
      record.reviewSourceGraph = await graph(source.worktreeId, source.seatId,source.instanceId);
      record.reasons.push('V10: F/E source and provider side-seat bindings are ready; no original K-SIDE create, same-seat private marker, or fresh FORMAL_REVIEW H/A turn has run.');
    } else {
      record.reasons.push('V10: no separate original F/E source seat/worktree is supplied for K-SIDE creation before the same provider seat is reopened as FORMAL_REVIEW.');
    }
    const session = { id: id('m2ProviderSession'), caseOwner: record.caseId, seatId: row.seatId,
      instanceId: row.instanceId, worktreeId: row.worktreeId,
      generation: (BigInt(seat.result.generation) + 1n).toString(), revision: '0',
      cursor: '0', events: [], turns: [] };
    record.sessionId = session.id; journal.sessions.push(session); product.save();
    const step = async (action, payload = {}, allowed = ['APPLIED']) => {
      let reply;
      try {
        reply = await operation('K-SESSION', action, session.id,
          { generation: session.generation, ...payload }, session.revision, allowed);
      } finally {
        const request = journal.operations.at(-1)?.request;
        if (request?.family === 'K-SESSION' && request.operation === action &&
            request.targetId === session.id &&
            !record.requestIds.some(item => item.requestId === request.requestId)) {
          record.requestIds.push({ action, requestId: request.requestId }); product.save();
        }
      }
      session.revision = reply.revision; product.save(); return reply;
    };
    const output = async () => {
      const reply = await readAtRevision('K-SESSION', 'output-stream', session.id,
        { generation: session.generation, afterCursor: session.cursor }, session.revision);
      checked(reply.result.generation === session.generation &&
        BigInt(reply.result.cursor) >= BigInt(session.cursor), 'Original provider output cursor/generation');
      session.revision = reply.revision; session.cursor = reply.result.cursor;
      session.events.push(...reply.result.events); product.save();
      if (reply.result.sourceError) throw Error(`Original ${driverId} source error: ${JSON.stringify(reply.result.sourceError)}`);
      return reply.result;
    };
    const observe = async (predicate, label) => {
      const deadline = Date.now() + 600000;
      while (Date.now() < deadline) {
        const page = await output();
        if (predicate(page)) return page;
        await delay(300); // Original read only. No second send or answer.
      }
      throw Error(`${label}: original observation deadline; retain H process and request for Controller`);
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
    if (driverId === 'claude') {
      checked(cardMode === 'SOURCE_PRESENT_NATIVE_ASK_USER_BEHAVIOUR_NOT_RUN',
        'Claude source capability changed; this is not a behaviour PASS');
      checked(typeof row.worktreeRoot === 'string' && path.isAbsolute(row.worktreeRoot) &&
        fs.existsSync(row.worktreeRoot) && fs.statSync(row.worktreeRoot).isDirectory() &&
        fs.realpathSync(row.worktreeRoot).toLowerCase() === path.resolve(row.worktreeRoot).toLowerCase() &&
        path.relative(path.resolve(config.stateRoot), path.resolve(row.worktreeRoot)) !== '..' &&
        !path.relative(path.resolve(config.stateRoot), path.resolve(row.worktreeRoot)).startsWith(`..${path.sep}`),
      'Claude worktree root must be the pre-existing local F testbed path');
      const markerFile = `m2-claude-${record.caseId}.json`;
      checked(atom(markerFile) && !fs.existsSync(path.join(config.testbedSource, markerFile)) &&
        !fs.existsSync(path.join(row.worktreeRoot, markerFile)),
        'Unique non-secret marker must be absent from original private source');
      const marker = { caseId: record.caseId, format: 'JSON', question: questionText };
      record.claudeQuestion = { markerFile, marker, text: questionText,
        header: questionHeader, options: questionOptions, selected: 'JSON',
        worktreeRoot: row.worktreeRoot, markerAbsentBeforeSend: true };
      const prompt = `Private non-secret M2 test in this one fixed Claude session. ` +
        `Call the real AskUserQuestion tool with exactly one question: ${JSON.stringify({
          question: questionText, header: questionHeader, options: questionOptions, multiSelect: false,
        })}. Wait for its answer. If the User selects JSON, create only the new relative file ` +
        `${JSON.stringify(markerFile)} in this bound worktree, containing the compact UTF-8 ` +
        `JSON object ${JSON.stringify(marker)} followed by one newline. ` +
        `Do not create it before the question is answered. ` +
        `Do not read credentials or files outside this worktree, use the network, open a browser, ` +
        `or run any generated text in the host. After the file is written, report its name.`;
      record.prompt = prompt; product.save();
      const sent = await step('send', { body: prompt }, ['APPLIED', 'UNKNOWN']);
      record.sendRequestId = record.requestIds.at(-1).requestId;
      record.sendStatus = sent.status; product.save();
      const asking = await observe(page => {
        checked(page.nativeCardRefsIncomplete === false, 'Native question reference page incomplete');
        return page.nativeCardRefs?.some(card => card.state === 'OPEN' &&
          card.turnId === record.sendRequestId);
      }, 'Original Claude AskUserQuestion card');
      const openCards = asking.nativeCardRefs.filter(card => card.state === 'OPEN' &&
        card.turnId === record.sendRequestId);
      checked(openCards.length === 1, 'Exactly one original current H send question card');
      const originalCard = openCards[0];
      const recovered = await readAtRevision('K-QCARD', 'recover', originalCard.cardId, {}, originalCard.revision);
      const q = recovered.result.nativeQuestion;
      checked(recovered.result.state === 'OPEN' && recovered.result.availableForAnswer === true &&
        recovered.result.turnId === record.sendRequestId &&
        recovered.result.seatId === row.seatId && recovered.result.generation === session.generation &&
        q?.provider === 'claude' && q.idOrigin === 'HOST_DERIVED_ARRAY_INDEX' &&
        q.questions?.length === 1 && q.questions[0].hostIndex === 0 &&
        q.questions[0].hostQuestionId === 'host0' &&
        q.questions[0].question === questionText && q.questions[0].header === questionHeader &&
        q.questions[0].multiSelect === false &&
        JSON.stringify(q.questions[0].options.map(({ label, description }) => ({ label, description }))) ===
          JSON.stringify(questionOptions) &&
        q.originalInput?.questions?.length === 1 &&
        q.originalInput.questions[0].question === questionText &&
        q.originalInput.questions[0].header === questionHeader &&
        q.originalInput.questions[0].multiSelect === false &&
        JSON.stringify(q.originalInput.questions[0].options.map(({ label, description }) =>
          ({ label, description }))) === JSON.stringify(questionOptions),
      'Full original Claude question, host-derived index and current H turn are required');
      record.card = { cardId: originalCard.cardId, revision: originalCard.revision,
        hostQuestionId: 'host0', turnId: record.sendRequestId,
        requestRef: recovered.result.requestRef, originalInput: q.originalInput };
      product.save();
      const answered = await operation('K-QCARD', 'answer', originalCard.cardId,
        { generation: session.generation, answers: { host0: ['JSON'] } }, recovered.revision,
        ['APPLIED', 'UNKNOWN']);
      record.card.answerRequestId = journal.operations.at(-1).request.requestId;
      record.card.answerStatus = answered.status;
      checked(answered.status === 'UNKNOWN' ||
        (answered.result.state === 'ANSWERED' &&
          answered.result.deliveryBasis === 'NATIVE_EXACT_WRITE_RECEIPT' &&
          answered.result.vendorConsumptionConfirmed === false),
      'C/H exact answer write is not vendor answer acceptance');
      product.save();
      await observe(page => {
        const original = page.nativeInputReceipts?.find(value => value.requestId === record.sendRequestId);
        if (original?.receipt?.status === 'FAILED') {
          throw Error('Original Claude H send failed; preserve raw provider result');
        }
        return original?.receipt?.status === 'APPLIED';
      }, 'Original Claude result after question answer');
      record.checks.V03b = 'DIRECT_CASE_PENDING_IMMUTABLE_READBACK';
      record.state = 'CLAUDE_QUESTION_FLOW_DIRECT_READBACK_REQUIRED'; product.save();
    } else {
      checked(cardMode === 'UNSUPPORTED_REPLY_ENCODER',
        `${driverId}: unsupported question codec boundary changed`);
      record.reasons.push('V03b: no provider-bound native question reply codec; no model question turn is induced.');
    }
    record.reasons.push('V04b: capability memoryOffLaunch is source/static only; no effective vendor memory-store or instruction-load provenance has been observed.');
    record.reasons.push('V10: current run did not create a provider-owned SIDE_CHAT, private marker and subsequent fresh FORMAL_REVIEW; WORK and model prose cannot stand in.');
    const stopped = await step('stop', { seatId: row.seatId });
    checked(typeof stopped.result.stopFact === 'string' && stopped.result.stopFact.length > 0,
      'Original provider process has no durable H stop fact');
    record.stopFact = stopped.result.stopFact;
    await step('admission-release', { seatId: row.seatId });
    if (driverId !== 'claude') record.state = 'REAL_FIXED_CAPABILITY_PREFLIGHT_ONLY';
    product.save();
  }
  journal.providerBoundaryCases.push({ caseId: id('m2Provider'), driverId: 'antigravity',
    state: 'NOT_RUN_OWNER_DECISION_PENDING', acceptance: false,
    checks: { V03b: 'NOT_RUN', V04b: 'NOT_RUN', V10: 'NOT_RUN' },
    reasons: ['Fixed 1.2.11 still has shared Windows credential and no proven memory-off/instance login contract; no CLI, auth or model operation started.'] });
  const claude = journal.providerBoundaryCases.find(caseRow => caseRow.driverId === 'claude');
  journal.providerBoundarySummary = { V03b: claude?.state === 'CLAUDE_QUESTION_FLOW_DIRECT_READBACK_REQUIRED'
    ? 'CLAUDE_DIRECT_CASE_PENDING_NORMAL_CLOSE_READBACK' : 'NOT_RUN_CLAUDE_NOT_LOGGED_IN',
    V04b: 'NOT_RUN_NO_EFFECTIVE_MEMORY_AND_CROSS_PROJECT_INPUT_PROOF',
    V10: 'NOT_RUN_NO_SAME_SEAT_PROVIDER_SIDE_SOURCE', acceptance: false,
    directReadbackRequired: true };
  product.save(); return journal.providerBoundarySummary;
}
