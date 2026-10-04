// V08 installed-product preparation. Importing this module performs no operation.
import fs from 'node:fs';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { id, delay } from './product-cdp.mjs';

const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const decimal = value => typeof value === 'string' && /^(0|[1-9][0-9]*)$/.test(value);
const requireFact = (condition, message) => { if (!condition) throw Error(message); };
const binding = ({ id, seatId, instanceId, worktreeId, generation, threadId }) =>
  ({ id, seatId, instanceId, worktreeId, generation, threadId });
const hostSessionLocator = (domain, cause, policyRevision) => {
  const parts = ['host-reject-cap', domain, cause, ''].map(value => Buffer.from(value));
  const framed = Buffer.concat(parts.flatMap(part => {
    const size = Buffer.alloc(8); size.writeBigUInt64BE(BigInt(part.length)); return [size, part];
  }));
  const identity = `sha256:${hash(framed)}`;
  const suffix = hash(Buffer.from(identity)).slice(0, 40);
  const trigger = `host-reject-cap-${suffix}`;
  const escalation = `host-escalate-${suffix}`;
  const digest = hash(Buffer.from(`${domain}\n${escalation}\n${trigger}\n${cause}\n${policyRevision}`));
  return { sessionId: `hostsession-${digest}`, messageId: `hostmsg-${digest}` };
};

export async function runRulesCase(product, config, journal) {
  const c = config.rules;
  requireFact(process.platform === 'win32' && config.repositoryId === 'gogokeSeatTestbed' &&
    /^[a-f0-9]{40}$/.test(config.sourceCommit ?? '') &&
    c?.lifecycleOwnership === 'EXCLUSIVE_V08_SUBMITTER_AND_REVIEWER' &&
    c.policyOwnership === 'EXCLUSIVE_V08_POLICY_DOMAIN',
  'V08 requires the authorized installed testbed and exclusive policy/session ownership');
  const record = { caseId: id('V08'), state: 'RUNNING', acceptance: false,
    sourceCommit: config.sourceCommit, domainId: config.domainId,
    driverSha256: hash(fs.readFileSync(fileURLToPath(import.meta.url))),
    readerSha256: hash(fs.readFileSync(fileURLToPath(new URL('./m2-rules-readback.py', import.meta.url)))),
    ownership: { lifecycle: c.lifecycleOwnership, policy: c.policyOwnership },
    ownerConfigurations: [], seatCards: [], initialSessions: [], userBoundaries: [], actions: [], hostCases: [], notRun: [
      { caseId: 'V08_MODEL_FORGED_SENDER', reason: 'The advertised tool accepts no sender/caller field; a real malformed caller A frame is required. User bytes cannot substitute.' },
      { caseId: 'V08_MODEL_CROSS_PROJECT', reason: 'The native tool derives domain from H; it exposes no cross-domain selector. A genuine reachable cross-project model call is required.' },
      { caseId: 'V08_MODEL_SUBORDINATE_OWNER', reason: 'A distinct admitted subordinate and its real reachable MESSAGE/Owner operation are required; gate target text is not that operation.' },
      { caseId: 'V08_REJECT_CAP_DELIVERY', reason: 'Host recipient/checkpoint context not supplied; gate state alone cannot prove original E/C/H delivery.' },
      { caseId: 'V08_STALL_CHAIN', reason: 'There is no current STALLED source producer for the Host observer; an enum/table row or scheduled trigger cannot substitute.' },
    ], readbackRequired: true };
  journal.rulesCases ??= []; journal.rulesCases.push(record); product.save();
  try {
    const baseline = c.baselineReadback;
    requireFact(baseline && path.basename(baseline.file) === baseline.file &&
      /^[a-f0-9]{64}$/.test(baseline.sha256), 'V08 normal-close baseline artifact required');
    const bytes = fs.readFileSync(path.join(config.evidenceDirectory, baseline.file));
    requireFact(hash(bytes) === baseline.sha256, 'V08 baseline original bytes changed');
    const before = JSON.parse(bytes.toString('utf8'));
    requireFact(before.schema === 'gogoke.37.private-m2-rules-readback.v1' && before.phase === 'before' &&
      before.caseId === journal.caseId && before.sourceCommit === config.sourceCommit &&
      before.domainId === config.domainId && before.measurementPreservedDatabaseBytes === true &&
      before.databaseWrites === false && before.credentialReads === false && before.policy.head.length === 1 &&
      before.readerSha256 === record.readerSha256,
    'V08 baseline must be the same candidate/domain immutable native readback');
    record.baselineReadback = baseline;
    let policyRevision = String(before.policy.head[0].revision);
    const fromStage = before.policy.head[0].current_stage, toStage = id('v08Stage');
    const submitter = c.submitterSession, reviewer = c.reviewerSession;
    for (const session of [submitter, reviewer]) {
      requireFact(session && journal.sessions.includes(session) && session.threadId &&
        decimal(session.generation) && decimal(session.revision) && decimal(session.cursor) &&
        Array.isArray(session.events) && Array.isArray(session.turns),
      'V08 needs original already opened sessions in this journal');
      const opened = journal.operations.filter(row => row.request?.family === 'K-SESSION' &&
        ['open', 'resume'].includes(row.request.operation) && row.request.targetId === session.id &&
        row.receipt?.status === 'APPLIED' && row.receipt.result?.threadId === session.threadId);
      requireFact(opened.length > 0, 'V08 session must come from actual H open/resume');
    }
    requireFact(submitter.id !== reviewer.id && submitter.seatId !== reviewer.seatId &&
      submitter.worktreeId !== reviewer.worktreeId, 'V08 needs two independent exclusive seats/worktrees');
    record.submitterSession = submitter.id; record.reviewerSession = reviewer.id;
    record.initialSessions = [binding(submitter), binding(reviewer)];
    record.fromStage = fromStage; record.toStage = toStage;
    record.rejectGate = id('v08Reject'); record.passGate = id('v08Pass');
    record.rejectReasons = [id('V08_reason_one'), id('V08_reason_cap')]; product.save();
    for (const session of [submitter, reviewer]) {
      let card = await product.operation('K-SEAT', 'state-card', session.seatId, {}, '0', ['APPLIED', 'STALE']);
      if (card.status === 'STALE') card = await product.operation('K-SEAT', 'state-card', session.seatId, {}, card.revision);
      requireFact(card.result.state === 'BUSY' && card.result.instanceId === session.instanceId &&
        String(card.result.generation) === session.generation, 'Original User E seat card must match current admitted H session');
      record.seatCards.push(journal.operations.at(-1).request.requestId); product.save();
    }

    const configure = async (command, fields) => {
      const request = { schema: 'gogoke.37.owner-configuration.v1', command,
        domainId: config.domainId, requestId: id('v08Owner'), ...fields, expectedRevision: policyRevision };
      const rawFrame = JSON.stringify(request);
      const entry = { kind: 'V08_OWNER_CONFIGURATION', request, rawFrame,
        startedAt: new Date().toISOString(), receipt: null };
      journal.operations.push(entry); record.ownerConfigurations.push(request.requestId); product.save();
      try {
        const raw = await product.evaluate(`window.__TAURI_INTERNALS__.invoke('gogoke_design37_user_operation',{frame:${JSON.stringify(rawFrame)}})`);
        entry.rawReceipt = raw; entry.receipt = JSON.parse(raw); entry.finishedAt = new Date().toISOString(); product.save();
        requireFact(entry.receipt.schema === request.schema && entry.receipt.command === command &&
          entry.receipt.requestId === request.requestId && entry.receipt.status === 'APPLIED' &&
          entry.receipt.revision === (BigInt(policyRevision) + 1n).toString(), 'Original Owner policy CAS receipt');
        policyRevision = entry.receipt.revision;
      } catch (error) { entry.originalError = String(error.stack ?? error); product.save(); throw error; }
    };
    const grant = expiresAtMs => configure('policy-call-grant', { callerSeatId: submitter.seatId,
      targetId: reviewer.seatId, action: 'REVIEW', expiresAtMs });
    await grant(null);
    for (const gateId of [record.rejectGate, record.passGate]) await configure('policy-gate', {
      gateId, submitterSeatId: submitter.seatId, reviewerSeatId: reviewer.seatId,
      fromStage, toStage, rejectCap: 2 });

    // This is solely a User ingress control, never a forged native/model caller.
    const userReply = await product.operation('K-POLICY', 'gate-decide', record.passGate,
      { decision: 'PASS', callerSeatId: reviewer.seatId, role: 'OWNER' }, '1', ['UNSUPPORTED']);
    record.userBoundaries.push({ caseId: 'V08_USER_POLICY_UNSUPPORTED',
      requestId: journal.operations.at(-1).request.requestId, receipt: userReply,
      authority: 'REAL_USER_INGRESS_ONLY' }); product.save();

    const output = async (session, allowQuestion = false) => {
      let reply = await product.operation('K-SESSION', 'output-stream', session.id,
        { generation: session.generation, afterCursor: session.cursor }, session.revision, ['APPLIED', 'STALE']);
      if (reply.status === 'STALE') {
        session.revision = reply.revision; product.save();
        reply = await product.operation('K-SESSION', 'output-stream', session.id,
          { generation: session.generation, afterCursor: session.cursor }, session.revision);
      }
      requireFact(decimal(reply.result.cursor) && BigInt(reply.result.cursor) >= BigInt(session.cursor),
        'V08 original output cursor monotonic');
      session.cursor = reply.result.cursor; session.revision = reply.revision;
      session.events.push(...reply.result.events); product.save();
      if (reply.result.sourceError) throw Error(`V08 original source error: ${JSON.stringify(reply.result.sourceError)}`);
      requireFact(allowQuestion || !reply.result.nativeCardRefs.some(row => row.state === 'OPEN'),
        'V08 unexpected question card; preserve without answering');
      return reply.result;
    };
    const call = async (caseId, session, operation, gate, revision, payload, status, state = null, reason = '') => {
      const args = { operation, targetId: gate, expectedRevision: revision, payload };
      const action = { caseId, sessionId: session.id, seatId: session.seatId,
        binding: binding(session), arguments: args,
        expected: { status, state, reason, policyRevision }, state: 'PREPARED' };
      record.actions.push(action); product.save();
      const body = `Owner-authorized V08 test, ${record.caseId}/${caseId}. Make exactly one real gogoke_policy call with these exact arguments: ${JSON.stringify(args)}. No retries or other tool calls. Do not edit files, invoke processes, dispatch seats, contact anyone, request permission, or infer success from prose. Preserve the original native result and finish this turn.`;
      action.askBytes = body; const start = session.events.length; product.save();
      const sent = await product.operation('K-SESSION', 'send', session.id,
        { generation: session.generation, body }, session.revision);
      action.sendRequestId = journal.operations.at(-1).request.requestId;
      action.sendReceipt = sent; session.revision = sent.revision;
      requireFact(sent.result.createdTurn === true && sent.result.turnId, 'V08 actual H created model turn');
      action.turnId = sent.result.turnId; action.state = 'SENT'; product.save();
      const deadline = Date.now() + 600000;
      let completed;
      while (Date.now() < deadline) {
        await output(session);
        completed = session.events.slice(start).find(row => row._meta?.codexMethod === 'turn/completed' &&
          row._meta.threadId === session.threadId && row._meta.turnId === action.turnId);
        if (completed) break;
        await delay(300); // Readonly wait for this exact original CLI turn, never resend.
      }
      requireFact(completed?._meta?.turnStatus === 'completed', 'V08 original CLI completion required');
      const events = session.events.slice(start).filter(row => row._meta?.turnId === action.turnId);
      const tools = events.filter(row => row._meta?.codexMethod === 'item/completed' &&
        ['dynamicToolCall', 'mcpToolCall', 'commandExecution', 'fileChange'].includes(row._meta.codexItemType));
      requireFact(tools.length === 1 && tools[0]._meta.codexItemType === 'dynamicToolCall' &&
        ['completed', 'failed'].includes(tools[0].status), 'V08 requires one original dynamic tool completion');
      action.cliToolStatus = tools[0].status; product.save();
      const content = Array.isArray(tools[0].rawOutput) ? tools[0].rawOutput : tools[0].rawOutput?.contentItems;
      requireFact(Array.isArray(content) && content.length === 1 && content[0].type === 'inputText' &&
        typeof content[0].text === 'string', 'V08 original native receipt text missing');
      action.rawToolReceipt = content[0].text; action.receipt = JSON.parse(content[0].text); product.save();
      const receipt = action.receipt;
      requireFact(receipt.schema === 'gogoke.37.operations.v1' && receipt.family === 'K-POLICY' &&
        receipt.operation === operation && receipt.targetId === gate && receipt.status === status &&
        receipt.previousRevision === revision && receipt.revision ===
          (BigInt(revision) + (status === 'APPLIED' ? 1n : 0n)).toString(), 'V08 original native policy result');
      if (status === 'APPLIED') requireFact(receipt.result.state === state && receipt.result.reason === reason &&
        receipt.result.policyRevision === policyRevision, 'V08 native state/reason/policy revision');
      action.state = 'OBSERVED_NATIVE_RECEIPT_READBACK_REQUIRED';
      session.turns.push({ turnId: action.turnId, sendRequestId: action.sendRequestId }); product.save();
      return action;
    };
    const reject = record.rejectGate, pass = record.passGate;
    await call('V08_SUBMIT_REJECT_GATE', submitter, 'gate-submit', reject, '1', {}, 'APPLIED', 'SUBMITTED');
    await call('V08_REJECT_WITH_REASON', reviewer, 'gate-decide', reject, '2',
      { decision: 'REJECT', reason: record.rejectReasons[0] }, 'APPLIED', 'REJECTED', record.rejectReasons[0]);
    await call('V08_MODEL_BYPASS_GATE', submitter, 'stage-transition', reject, '3', {}, 'DENIED');
    await call('V08_RESUBMIT', submitter, 'gate-submit', reject, '3', {}, 'APPLIED', 'SUBMITTED');
    await call('V08_REJECT_CAP_STATE_ONLY', reviewer, 'gate-decide', reject, '4',
      { decision: 'REJECT', reason: record.rejectReasons[1] }, 'APPLIED', 'ESCALATION_REQUIRED', record.rejectReasons[1]);
    await call('V08_MODEL_CAP_BLOCKS_SUBMIT', submitter, 'gate-submit', reject, '5', {}, 'DENIED');
    await grant('1'); // An already expired genuine Owner grant; no waiting or clock approximation.
    await call('V08_MODEL_EXPIRED_GRANT', submitter, 'gate-submit', pass, '1', {}, 'DENIED');
    await grant(null);
    await call('V08_SUBMIT_PASS_GATE', submitter, 'gate-submit', pass, '1', {}, 'APPLIED', 'SUBMITTED');
    await call('V08_MODEL_WRONG_REVIEWER', submitter, 'gate-decide', pass, '2', { decision: 'PASS' }, 'DENIED');
    await call('V08_MODEL_EMPTY_REJECT_REASON', reviewer, 'gate-decide', pass, '2',
      { decision: 'REJECT', reason: '' }, 'DENIED');
    await call('V08_APPROVE', reviewer, 'gate-decide', pass, '2', { decision: 'PASS' }, 'APPLIED', 'PASSED');
    policyRevision = (BigInt(policyRevision) + 1n).toString();
    await call('V08_LEGAL_STAGE', submitter, 'stage-transition', pass, '3', {}, 'APPLIED', 'ADVANCED', toStage);
    if (c.host) {
      const h = c.host;
      requireFact(h.lifecycleOwnership === 'EXCLUSIVE_V08_HOST_RECIPIENTS' &&
        ['hostCheckpoint', 'openHostSession', 'resumeRulesSession', 'stopRulesSession',
          'releaseStoppedRulesSession'].every(name => typeof c[name] === 'function'),
      'Host cases require scoped real runner lifecycle/checkpoint callbacks');
      const selections = [h.destination, h.alternateDestination];
      requireFact(selections.every(row => row && ['seatId', 'instanceId', 'worktreeId']
        .every(name => typeof row[name] === 'string' && row[name])) &&
        new Set([submitter.seatId, reviewer.seatId, ...selections.map(row => row.seatId)]).size === 4 &&
        new Set([submitter.worktreeId, reviewer.worktreeId, ...selections.map(row => row.worktreeId)]).size === 4 &&
        typeof h.busyQuestion?.questionId === 'string' && typeof h.busyQuestion?.optionLabel === 'string',
      'Four exclusive actual registered test seats/worktrees and a nonsecret busy question are required');
      record.hostOwnership = h.lifecycleOwnership;
      record.hostRecipients = selections; product.save();
      const open = async selection => {
        const session = await c.openHostSession(selection);
        requireFact(journal.sessions.includes(session) && selections.some(row =>
          row.seatId === session.seatId && row.instanceId === session.instanceId && row.worktreeId === session.worktreeId) &&
          session.threadId && decimal(session.generation), 'Recipient must be the actual case-owned H open');
        return session;
      };
      const checkpoint = async host => {
        const reference = await c.hostCheckpoint();
        requireFact(reference && path.basename(reference.file) === reference.file &&
          /^[a-f0-9]{64}$/.test(reference.sha256), 'Host checkpoint original artifact required');
        const bytes = fs.readFileSync(path.join(config.evidenceDirectory, reference.file));
        requireFact(hash(bytes) === reference.sha256, 'Original host checkpoint bytes changed');
        const snapshot = JSON.parse(bytes.toString('utf8'));
        requireFact(snapshot.schema === before.schema && snapshot.phase === 'checkpoint' &&
          snapshot.caseId === journal.caseId && snapshot.sourceCommit === config.sourceCommit &&
          snapshot.domainId === config.domainId && snapshot.databasePath === before.databasePath &&
          snapshot.readerSha256 === record.readerSha256 &&
          snapshot.rootIdentity.observer === before.rootIdentity.observer &&
          JSON.stringify(snapshot.rootIdentity) === JSON.stringify(before.rootIdentity) &&
          snapshot.measurementPreservedDatabaseBytes && !snapshot.databaseWrites && !snapshot.credentialReads,
        'Host checkpoint must be a normally closed original immutable candidate readback');
        const original = snapshot.hostSnapshots.find(row => row.caseId === host.caseId);
        requireFact(original && (host.kind === 'DELIVERED' ?
          original.message.state === 'DELIVERED' && original.recipient?.mode === 'FRESH' &&
            original.autoBinding && original.deliveries.length === 1 && original.sends.length === 1 &&
            original.commands.length === 1 && original.message.turn_id &&
            original.message.generation === original.autoBinding.generation &&
            original.recipientTerminal?.status === 'completed' &&
            original.recipientTerminal.source.state !== 'PENDING' :
          original.message.state === (host.kind === 'CANCELLED' ? 'CANCELLED' : 'PENDING') &&
            original.message.turn_id === '' &&
            original.message.generation === '' && original.deliveries.length === 0 &&
            original.sends.length === 0 && original.recipient === null),
        'Actual Host checkpoint must contain the original automatic delivery or genuinely blocked queue');
        const stopped = [submitter, reviewer, ...(host.busy ? [journal.sessions.find(row => row.id === host.busy.binding.id)] : [])];
        for (const session of stopped) {
          const claim = snapshot.stoppedClaims.find(row => row.session_id === session?.id);
          requireFact(claim?.state === 'STOPPED' && claim.generation === session.generation &&
            claim.instance_id === session.instanceId && claim.stop_fact_id && Number.isSafeInteger(claim.revision),
          'Normal close must leave this exact case-owned H generation physically stopped');
          session.revision = String(claim.revision);
        }
        host.checkpoint = reference; host.messageId = original.message.message_id;
        host.enqueueRequestId = original.enqueue.request_id;
        host.triggerId = original.intent.trigger_id; host.escalationRequestId = original.intent.request_id;
        host.queuedRevision = original.message.revision; product.save();
        if (host.kind === 'DELIVERED') {
          const target = { ...original.autoBinding, cursor: '0', events: [], turns: [] };
          requireFact(target.seatId === h.destination.seatId &&
            target.instanceId === h.destination.instanceId && target.worktreeId === h.destination.worktreeId &&
            target.id === host.autoObservation.sessionId &&
            target.threadId === host.autoObservation.threadId &&
            original.message.turn_id === host.autoObservation.turnId &&
            !journal.sessions.some(row => row.id === target.id),
          'Automatic recipient must be the unique original Host-created H session');
          journal.sessions.push(target);
          host.targetSessions.push(binding(target)); host.observedTurnId = original.message.turn_id;
          product.save();
        }
      };
      const resumeSources = async () => {
        for (const session of [submitter, reviewer]) await c.resumeRulesSession(session);
        requireFact([submitter, reviewer].every((row, index) => row.threadId && decimal(row.generation) &&
          ['id', 'seatId', 'instanceId', 'worktreeId'].every(name => row[name] === record.initialSessions[index][name])),
          'Sources must return through real H resume, preserving journal identity');
      };
      const inbox = async host => {
        let reply = await product.operation('K-INBOX', 'check-unknown', host.messageId, {},
          host.queuedRevision, ['APPLIED', 'STALE']);
        if (reply.status === 'STALE') reply = await product.operation('K-INBOX', 'check-unknown',
          host.messageId, {}, reply.revision);
        host.readRequestIds.push(journal.operations.at(-1).request.requestId); product.save();
        return reply;
      };
      const cancel = async host => {
        const current = await inbox(host);
        requireFact(current.result.state === 'PENDING', 'Only actual pending Host notices may be cancelled');
        const cancelled = await product.operation('K-INBOX', 'cancel', host.messageId, {}, current.revision);
        requireFact(cancelled.result.state === 'CANCELLED', 'Original User Host cancellation receipt');
        host.cancelRequestId = journal.operations.at(-1).request.requestId; product.save();
      };
      const holdBusy = async (host, session) => {
        const body = `Owner-authorized V08 busy-queue case ${host.caseId}. Ask exactly one non-secret native request_user_input question with id ${JSON.stringify(h.busyQuestion.questionId)}, header "V08 queue", text "Keep this test turn waiting for the Owner", and option label ${JSON.stringify(h.busyQuestion.optionLabel)}. Wait for the answer. Do not call any other tool, edit files, dispatch or contact anyone.`;
        host.busy = { binding: binding(session), question: h.busyQuestion,
          askBytes: body, readRequestIds: [] }; product.save();
        const sent = await product.operation('K-SESSION', 'send', session.id,
          { generation: session.generation, body }, session.revision);
        session.revision = sent.revision;
        requireFact(sent.result.createdTurn && sent.result.turnId, 'Real native busy turn required');
        host.busy.sendRequestId = journal.operations.at(-1).request.requestId;
        host.busy.turnId = sent.result.turnId; product.save();
        const deadline = Date.now() + 600000;
        while (Date.now() < deadline) {
          const page = await output(session, true);
          const card = page.nativeCardRefs.find(row => row.state === 'OPEN');
          if (card) { host.busy.cardId = card.cardId; host.busy.cardRevision = card.revision; break; }
          await delay(300);
        }
        requireFact(host.busy.cardId, 'No actual native question: busy control cannot run');
      };
      const readBusy = async (host, session) => {
        const reply = await product.operation('K-QCARD', 'recover', host.busy.cardId, {}, host.busy.cardRevision);
        requireFact(reply.result.state === 'OPEN' && reply.result.availableForAnswer === true &&
          reply.result.seatId === session.seatId && reply.result.generation === session.generation &&
          reply.result.nativeQuestion.threadId === session.threadId &&
          reply.result.nativeQuestion.turnId === host.busy.turnId &&
          reply.result.nativeQuestion.questions.length === 1 &&
          reply.result.nativeQuestion.questions[0].id === h.busyQuestion.questionId &&
          !reply.result.nativeQuestion.questions[0].isSecret &&
          reply.result.nativeQuestion.questions[0].options.some(row => row.label === h.busyQuestion.optionLabel),
        'Busy precondition is original live native question custody, not a BUSY table claim');
        host.busy.readRequestIds.push(journal.operations.at(-1).request.requestId); product.save();
      };
      const observeAutomatic = async host => {
        const locator = hostSessionLocator(config.domainId, host.causeEventId, host.policyRevision);
        const deadline = Date.now() + 600000;
        let card;
        while (Date.now() < deadline) {
          card = await product.operation('K-SEAT', 'state-card', h.destination.seatId, {}, '0', ['APPLIED', 'STALE']);
          if (card.status === 'STALE') card = await product.operation('K-SEAT', 'state-card',
            h.destination.seatId, {}, card.revision);
          if (card.result.state === 'BUSY' && card.result.instanceId === h.destination.instanceId) break;
          await delay(300);
        }
        requireFact(card?.result.state === 'BUSY' && card.result.instanceId === h.destination.instanceId,
          'Original Host recipient never became a real busy E seat');
        const target = { id: locator.sessionId, seatId: h.destination.seatId,
          instanceId: h.destination.instanceId, worktreeId: h.destination.worktreeId,
          generation: String(card.result.generation), revision: '0', cursor: '0', events: [] };
        host.autoObservation = { sessionId: target.id, seatCardRequestId: journal.operations.at(-1).request.requestId };
        product.save();
        let completed;
        while (Date.now() < deadline) {
          await output(target);
          completed = target.events.find(row => row._meta?.codexMethod === 'turn/completed' &&
            row._meta.turnStatus === 'completed');
          if (completed) break;
          await delay(300);
        }
        requireFact(completed?._meta?.turnId && completed._meta.threadId,
          'Original automatic recipient CLI turn did not complete before normal close');
        host.autoObservation = { ...host.autoObservation, generation: target.generation,
          threadId: completed._meta.threadId, turnId: completed._meta.turnId };
        product.save();
      };
      for (const kind of ['DELIVERED', 'BUSY_QUEUED', 'ROUTE_CHANGED', 'CANCELLED']) {
        const host = { caseId: id(`V08_HOST_${kind}`), kind, gateId: id('v08HostGate'),
          toStage: id('v08HostStage'), reason: id('v08HostReason'), sourceSeatId: submitter.seatId,
          destination: h.destination, readRequestIds: [], targetSessions: [], state: 'PREPARING' };
        record.hostCases.push(host); product.save();
        let busySession;
        if (kind !== 'DELIVERED') {
          busySession = await open(h.destination);
          await holdBusy(host, busySession); await readBusy(host, busySession);
        }
        await configure('policy-escalation-route', { fromSeatId: submitter.seatId,
          reason: 'REJECT_CAP', toSeatId: h.destination.seatId });
        host.routeRevision = policyRevision;
        await configure('policy-gate', { gateId: host.gateId, submitterSeatId: submitter.seatId,
          reviewerSeatId: reviewer.seatId, fromStage: toStage, toStage: host.toStage, rejectCap: 1 });
        host.policyRevision = policyRevision; product.save();
        await call(`${host.caseId}_SUBMIT`, submitter, 'gate-submit', host.gateId, '1', {}, 'APPLIED', 'SUBMITTED');
        const rejected = await call(`${host.caseId}_REJECT`, reviewer, 'gate-decide', host.gateId, '2',
          { decision: 'REJECT', reason: host.reason }, 'APPLIED', 'ESCALATION_REQUIRED', host.reason);
        host.causeEventId = rejected.receipt.requestId; host.state = 'ORIGINAL_CAP_OBSERVED'; product.save();
        if (busySession) await readBusy(host, busySession);
        if (kind === 'DELIVERED') await observeAutomatic(host);
        if (kind === 'ROUTE_CHANGED') {
          await configure('policy-escalation-route', { fromSeatId: submitter.seatId,
            reason: 'REJECT_CAP', toSeatId: h.alternateDestination.seatId });
          host.changedRouteRevision = policyRevision; product.save();
        }
        if (kind === 'CANCELLED') {
          host.messageId = hostSessionLocator(config.domainId, host.causeEventId, host.policyRevision).messageId;
          host.queuedRevision = '1'; product.save();
          await cancel(host);
        }
        await checkpoint(host);
        if (kind === 'BUSY_QUEUED') await cancel(host);
        if (busySession) await c.releaseStoppedRulesSession(busySession);
        if (kind === 'DELIVERED') {
          const target = journal.sessions.find(row => row.id === host.targetSessions[0].id);
          await c.releaseStoppedRulesSession(target);
        }
        await resumeSources();
        host.state = 'FLOW_COMPLETE_DIRECT_LEDGER_READBACK_REQUIRED'; product.save();
      }
      record.notRun = record.notRun.filter(row => row.caseId !== 'V08_REJECT_CAP_DELIVERY');
      record.notRun.push({ caseId: 'V08_HOST_LATE_ACK_AFTER_ROUTE_CHANGE', reason: 'No real deterministic UNKNOWN/late-ACK occurrence is available here; synthetic ACK/faults and input replay are not used.' },
        { caseId: 'V08_HOST_BUSY_TO_IDLE_DELIVERY', reason: 'Busy is proved by the native unanswered question; normal close and User cancellation clean up that separate cause. Its old vendor turn is never presumed idle on resume.' });
    }
    record.state = 'FLOW_COMPLETE_DIRECT_LEDGER_READBACK_REQUIRED'; product.save();
    return record;
  } catch (error) {
    record.state = 'FAILED_OR_NOT_RUN_PRESERVE_ORIGINAL'; record.originalError = String(error.stack ?? error);
    product.save(); throw error;
  }
}
