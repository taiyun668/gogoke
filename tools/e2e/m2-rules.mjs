// V08 installed-product preparation. Importing this module performs no operation.
import fs from 'node:fs';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { id, delay } from './product-cdp.mjs';

const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const decimal = value => typeof value === 'string' && /^(0|[1-9][0-9]*)$/.test(value);
const requireFact = (condition, message) => { if (!condition) throw Error(message); };

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
    ownership: { lifecycle: c.lifecycleOwnership, policy: c.policyOwnership },
    ownerConfigurations: [], seatCards: [], userBoundaries: [], actions: [], notRun: [
      { caseId: 'V08_MODEL_FORGED_SENDER', reason: 'The advertised tool accepts no sender/caller field; a real malformed caller A frame is required. User bytes cannot substitute.' },
      { caseId: 'V08_MODEL_CROSS_PROJECT', reason: 'The native tool derives domain from H; it exposes no cross-domain selector. A genuine reachable cross-project model call is required.' },
      { caseId: 'V08_MODEL_SUBORDINATE_OWNER', reason: 'A distinct admitted subordinate and its real reachable MESSAGE/Owner operation are required; gate target text is not that operation.' },
      { caseId: 'V08_REJECT_CAP_DELIVERY', reason: 'ESCALATION_REQUIRED is a gate state. Native K-POLICY escalate/trigger dispatch is UNSUPPORTED; original coordinator trigger and C/H delivery are required.' },
      { caseId: 'V08_STALL_CHAIN', reason: 'Requires an actual stalled seat health event, configured chain and original coordinator/C/H delivery; none may be synthesized.' },
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
      before.databaseWrites === false && before.credentialReads === false && before.policy.head.length === 1,
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

    const output = async session => {
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
      requireFact(!reply.result.nativeCardRefs.some(row => row.state === 'OPEN'),
        'V08 unexpected question card; preserve without answering');
    };
    const call = async (caseId, session, operation, gate, revision, payload, status, state = null, reason = '') => {
      const args = { operation, targetId: gate, expectedRevision: revision, payload };
      const action = { caseId, sessionId: session.id, seatId: session.seatId,
        arguments: args, expected: { status, state, reason, policyRevision }, state: 'PREPARED' };
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
    record.state = 'FLOW_COMPLETE_DIRECT_LEDGER_READBACK_REQUIRED'; product.save();
    return record;
  } catch (error) {
    record.state = 'FAILED_OR_NOT_RUN_PRESERVE_ORIGINAL'; record.originalError = String(error.stack ?? error);
    product.save(); throw error;
  }
}
