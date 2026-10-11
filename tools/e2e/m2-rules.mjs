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
      ...(!c.foreignProject ? [{ caseId: 'V08_MODEL_CROSS_PROJECT', reason: 'No actual NativeUser-created foreign domain/gate and immutable baseline supplied; an invented target cannot prove the boundary.' }] : []),
      { caseId: 'V08_MODEL_SUBORDINATE_OWNER', reason: 'The four dynamic tools and H allowlist expose no Model MESSAGE/Owner operation; CallAction::Message alone is not reachable, and User K-INBOX fixes sender to User.' },
      { caseId: 'V08_REJECT_CAP_DELIVERY', reason: 'Host recipient/checkpoint context not supplied; gate state alone cannot prove original E/C/H delivery.' },
      { caseId: 'V08_STALL_CHAIN', reason: 'No original failed WORK with contextWindowExceeded/typed retry followed by original compact -32601 Unsupported, same custody and no successor work was observed. Interrupted turns and synthetic cloud associations cannot substitute.' },
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
    if (c.selection === 'CROSS_PROJECT_ONLY') {
      const qualification = before.sameScopeQualification;
      requireFact(qualification?.state === 'QUALIFIED_SAME_SCOPE_GATE_SUBMIT' &&
        qualification.domainId === config.domainId && qualification.callerSeatId === c.submitterSession.seatId &&
        qualification.callerLayer === 'USER' && qualification.callerState === 'IDLE' &&
        qualification.gate?.submitter_seat_id === c.submitterSession.seatId &&
        qualification.gate.revision === before.foreignProject?.gate?.revision &&
        qualification.reviewGrant?.caller_seat_id === c.submitterSession.seatId &&
        qualification.reviewGrant.target_id === qualification.reviewerSeatId &&
        qualification.reviewGrant.action === 'REVIEW' && qualification.reviewGrant.expires_at_ms === 0,
      'Cross-project DENIED requires an actual same-scope A gate-submit/reviewer-REVIEW qualification');
      record.sameScopeQualification = qualification;
      requireFact(before.sameScopeOwnerPolicy?.configuration === c.sameScopePolicy &&
        before.sameScopeOwnerPolicy.gate?.submitter_seat_id === c.submitterSession.seatId &&
        before.sameScopeOwnerPolicy.gate?.reviewer_seat_id === c.reviewerSession.seatId,
      'Cross-project qualification must retain actual A NativeUser head/gate/grant frames for these two seats');
      record.sameScopePolicy = c.sameScopePolicy;
    }
    const foreign = c.foreignProject ?? null;
    requireFact(JSON.stringify(foreign) === JSON.stringify(journal.foreignProject ?? null) &&
      JSON.stringify(foreign) === JSON.stringify(before.foreignProject?.configuration ?? null),
    'Foreign fixture must match the original normally closed reader baseline');
    if (foreign) {
      requireFact(foreign.domainId !== config.domainId && before.foreignProject.policy.head.length === 1 &&
        before.foreignProject.gate.gate_id === foreign.gateId &&
        !before.policy.gates.some(row => row.gate_id === foreign.gateId),
      'Foreign target must actually exist in B and be absent from A');
      record.foreignProject = foreign;
    }
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

    if (c.selection === 'CROSS_PROJECT_ONLY') {
      const foreign = c.foreignProject;
      requireFact(foreign && foreign.ownerHead && foreign.ownerGate &&
        Object.keys(foreign.ownerHead).sort().join(',') === 'rawFrame,rawReceipt' &&
        Object.keys(foreign.ownerGate).sort().join(',') === 'rawFrame,rawReceipt' &&
        before.foreignProject?.configuration?.domainId === foreign.domainId &&
        before.foreignProject?.configuration?.gateId === foreign.gateId &&
        before.foreignProject?.policy?.head?.length === 1 &&
        before.foreignProject?.gate?.state === 'READY' && before.foreignProject.gate.revision === 1 &&
        !before.policy.gates.some(row => row.gate_id === foreign.gateId),
      'CROSS_PROJECT_ONLY requires the existing closed B head/gate baseline and original bytes; it never initializes or changes Owner policy');
      const headRequest = JSON.parse(foreign.ownerHead.rawFrame);
      const headReceipt = JSON.parse(foreign.ownerHead.rawReceipt);
      const gateRequest = JSON.parse(foreign.ownerGate.rawFrame);
      const gateReceipt = JSON.parse(foreign.ownerGate.rawReceipt);
      const keysEqual = (value, names) => JSON.stringify(Object.keys(value).sort()) === JSON.stringify([...names].sort());
      requireFact(keysEqual(headRequest, ['schema','command','domainId','requestId','stage','expectedRevision']) &&
        headRequest.schema === 'gogoke.37.owner-configuration.v1' && headRequest.command === 'policy-initialize' &&
        headRequest.domainId === foreign.domainId && headRequest.stage === before.foreignProject.policy.head[0].current_stage &&
        headRequest.expectedRevision === '0' && headReceipt.schema === headRequest.schema &&
        headReceipt.command === headRequest.command && headReceipt.requestId === headRequest.requestId &&
        headReceipt.status === 'APPLIED' && headReceipt.revision === '1' &&
        keysEqual(gateRequest, ['schema','command','domainId','requestId','gateId','submitterSeatId',
          'reviewerSeatId','fromStage','toStage','rejectCap','expectedRevision']) &&
        gateRequest.schema === 'gogoke.37.owner-configuration.v1' && gateRequest.command === 'policy-gate' &&
        gateRequest.domainId === foreign.domainId && gateRequest.gateId === foreign.gateId &&
        gateRequest.expectedRevision === headReceipt.revision && gateReceipt.schema === gateRequest.schema &&
        gateReceipt.command === gateRequest.command && gateReceipt.requestId === gateRequest.requestId &&
        gateReceipt.status === 'APPLIED' && gateReceipt.revision === '2',
      'Foreign fixture must carry its original new-test-domain NativeUser head/gate frames and receipts');
      requireFact(c.host === undefined, 'CROSS_PROJECT_ONLY cannot include Host cases');

      record.selection = 'CROSS_PROJECT_ONLY';
      record.selectedCase = 'V08_MODEL_CROSS_PROJECT';
      record.foreignProject = foreign;
      record.initialSessions = [binding(submitter), binding(reviewer)];
      record.submitterSession = submitter.id; record.reviewerSession = reviewer.id;
      record.worktreeCards = []; record.capabilityProbes = [];
      record.notRun = [
        ...['V08_SUBMIT_REJECT_GATE','V08_REJECT_WITH_REASON','V08_MODEL_BYPASS_GATE','V08_RESUBMIT',
          'V08_REJECT_CAP_STATE_ONLY','V08_MODEL_CAP_BLOCKS_SUBMIT','V08_MODEL_EXPIRED_GRANT',
          'V08_SUBMIT_PASS_GATE','V08_MODEL_FORGED_SENDER','V08_MODEL_WRONG_REVIEWER',
          'V08_MODEL_EMPTY_REJECT_REASON','V08_APPROVE','V08_LEGAL_STAGE'].map(caseId =>
            ({ caseId, reason: 'NOT_RUN_IN_CROSS_PROJECT_ONLY_SELECTION; preserved as an explicit original V08 requirement.' })),
        { caseId: 'V08_USER_POLICY_UNSUPPORTED', reason: 'NOT_RUN_IN_CROSS_PROJECT_ONLY_SELECTION; this run makes no User policy request.' },
        { caseId: 'V08_MODEL_SUBORDINATE_OWNER', reason: 'No advertised/reachable Model MESSAGE/Owner producer exists in the current H tool path.' },
        { caseId: 'V08_STALL_CHAIN', reason: 'No qualifying original failed WORK plus original compact -32601 Unsupported under the same custody.' },
        { caseId: 'V08_REJECT_CAP_DELIVERY', reason: 'Host recipient/checkpoint case is outside CROSS_PROJECT_ONLY.' },
        { caseId: 'V08_HOST_DELIVERED', reason: 'Host case is outside CROSS_PROJECT_ONLY.' },
        { caseId: 'V08_HOST_BUSY_QUEUED', reason: 'Host case is outside CROSS_PROJECT_ONLY.' },
        { caseId: 'V08_HOST_ROUTE_CHANGED', reason: 'Host case is outside CROSS_PROJECT_ONLY.' },
        { caseId: 'V08_HOST_CANCELLED', reason: 'Host case is outside CROSS_PROJECT_ONLY.' },
        { caseId: 'V08_HOST_BUSY_TO_IDLE_DELIVERY', reason: 'Host case is outside CROSS_PROJECT_ONLY.' },
        { caseId: 'V08_HOST_LATE_ACK_AFTER_ROUTE_CHANGE', reason: 'Late-ACK host case is outside CROSS_PROJECT_ONLY.' },
      ];
      for (const session of [submitter, reviewer]) {
        const tree = await product.operation('K-WORKTREE', 'graph-query', session.worktreeId, {}, '0', ['APPLIED', 'STALE']);
        const observedTree = tree.status === 'STALE' ?
          await product.operation('K-WORKTREE', 'graph-query', session.worktreeId, {}, tree.revision) : tree;
        requireFact(observedTree.result.state === 'REGISTERED' && observedTree.result.members?.some(member =>
          member.domainId === config.domainId && member.repositoryId === config.repositoryId &&
          member.seatId === session.seatId && member.instanceId === session.instanceId &&
          member.worktreeId === session.worktreeId),
        'Cross-project A session requires its actual registered E/F worktree membership');
        record.worktreeCards.push({ sessionId: session.id,
          requestId: journal.operations.at(-1).request.requestId,
          result: observedTree.result }); product.save();
        const capability = await product.operation('K-SESSION', 'capability-probe', session.id,
          { generation: session.generation }, session.revision);
        session.revision = capability.revision; product.save();
        requireFact(capability.result.driverId === 'codex' && capability.result.version &&
          /^sha256:[a-f0-9]{64}$/.test(capability.result.binaryDigest),
        'Cross-project H requires the actual current Codex CLI capability pin');
        record.capabilityProbes.push({ sessionId: session.id,
          requestId: journal.operations.at(-1).request.requestId, receipt: capability }); product.save();
      }
      const args = { operation: 'gate-submit', targetId: foreign.gateId,
        expectedRevision: String(before.foreignProject.gate.revision), payload: {} };
      const action = { caseId: 'V08_MODEL_CROSS_PROJECT', state: 'PREPARED', acceptance: false,
        sessionId: submitter.id, seatId: submitter.seatId, binding: binding(submitter),
        arguments: args, expected: { status: 'DENIED', state: null, reason: '',
          policyRevision: String(before.policy.head[0].revision) }, readbackRequired: true };
      const body = `Owner-authorized V08 cross-project test under the existing test scope. Make exactly one real gogoke_policy call with these exact arguments: ${JSON.stringify(args)}. No retries or other tool calls. Do not edit files, invoke processes, dispatch seats, contact anyone, request permission, or infer success from prose. Preserve the original native result and finish this turn.`;
      action.askBytes = body; record.actions.push(action); product.save();
      const start = submitter.events.length;
      const readOutput = async () => {
        let reply = await product.operation('K-SESSION', 'output-stream', submitter.id,
          { generation: submitter.generation, afterCursor: submitter.cursor }, submitter.revision,
          ['APPLIED', 'STALE']);
        if (reply.status === 'STALE') {
          submitter.revision = reply.revision; product.save();
          reply = await product.operation('K-SESSION', 'output-stream', submitter.id,
            { generation: submitter.generation, afterCursor: submitter.cursor }, submitter.revision);
        }
        requireFact(reply.result.generation === submitter.generation && decimal(reply.result.cursor) &&
          BigInt(reply.result.cursor) >= BigInt(submitter.cursor),
        'Cross-project original H output cursor must remain monotonic');
        submitter.cursor = reply.result.cursor; submitter.revision = reply.revision;
        submitter.events.push(...reply.result.events); product.save();
        requireFact(!reply.result.sourceError, 'Cross-project original A source error requires preservation without retry');
        return reply.result;
      };
      const sent = await sessionOp(submitter, 'send', { body });
      action.sendRequestId = journal.operations.at(-1).request.requestId;
      action.sendReceipt = sent; action.state = 'SENT';
      requireFact(sent.result.createdTurn === true && sent.result.turnId,
        'Cross-project test requires one actual original H model turn');
      action.turnId = sent.result.turnId; product.save();
      const deadline = Date.now() + 600000;
      let completed;
      while (Date.now() < deadline) {
        await readOutput();
        completed = submitter.events.slice(start).find(row => row._meta?.codexMethod === 'turn/completed' &&
          row._meta.threadId === submitter.threadId && row._meta.turnId === action.turnId);
        if (completed) break;
        await delay(300);
      }
      requireFact(completed?._meta?.turnStatus === 'completed',
        'Original cross-project CLI turn did not complete; the request is never resent');
      const events = submitter.events.slice(start).filter(row => row._meta?.turnId === action.turnId);
      const tools = events.filter(row => row._meta?.codexMethod === 'item/completed' &&
        ['dynamicToolCall','mcpToolCall','commandExecution','fileChange'].includes(row._meta.codexItemType));
      requireFact(tools.length === 1 && tools[0]._meta.codexItemType === 'dynamicToolCall' &&
        tools[0].status === 'failed', 'Cross-project requires exactly one original failed dynamic tool completion');
      const content = Array.isArray(tools[0].rawOutput) ? tools[0].rawOutput : tools[0].rawOutput?.contentItems;
      requireFact(Array.isArray(content) && content.length === 1 && content[0].type === 'inputText' &&
        typeof content[0].text === 'string', 'Original cross-project native tool receipt text is missing');
      action.cliToolStatus = tools[0].status; action.rawToolReceipt = content[0].text;
      action.receipt = JSON.parse(action.rawToolReceipt);
      requireFact(action.receipt.schema === 'gogoke.37.operations.v1' &&
        action.receipt.family === 'K-POLICY' && action.receipt.operation === 'gate-submit' &&
        action.receipt.targetId === foreign.gateId && action.receipt.status === 'DENIED' &&
        action.receipt.previousRevision === args.expectedRevision &&
        action.receipt.revision === args.expectedRevision && JSON.stringify(action.receipt.result) === '{}',
      'Cross-project model call did not return the exact original native DENIED result');
      submitter.turns.push({ turnId: action.turnId, sendRequestId: action.sendRequestId });
      action.state = 'ORIGINAL_DENIED_RECEIPT_OBSERVED_REQUIRES_CLOSED_READBACK';
      record.state = 'FLOW_COMPLETE_DIRECT_LEDGER_READBACK_REQUIRED';
      product.save();
      return record;
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
      requireFact(reply.result.generation === session.generation && decimal(reply.result.cursor) &&
        BigInt(reply.result.cursor) >= BigInt(session.cursor),
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
      action.rawToolReceipt = content[0].text; product.save();
      if (caseId === 'V08_MODEL_EMPTY_REJECT_REASON') {
        requireFact(status === 'INVALID_INPUT' && tools[0].status === 'failed' &&
          content[0].text === 'Native host operation failed: Invalid("reason")',
        'Empty reason must preserve the exact original native input refusal');
        action.nativeRefusal = { kind: 'INVALID_INPUT', original: content[0].text };
        action.state = 'OBSERVED_NATIVE_REFUSAL_READBACK_REQUIRED';
        session.turns.push({ turnId: action.turnId, sendRequestId: action.sendRequestId });
        product.save(); return action;
      }
      action.receipt = JSON.parse(content[0].text); product.save();
      const receipt = action.receipt;
      requireFact(tools[0].status === (status === 'APPLIED' ? 'completed' : 'failed') &&
        (status !== 'DENIED' || JSON.stringify(receipt.result) === '{}'),
      'Original CLI tool outcome must match the native policy result');
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
    await call('V08_MODEL_FORGED_SENDER', submitter, 'gate-decide', pass, '2',
      { decision: 'PASS', callerSeatId: reviewer.seatId }, 'DENIED');
    await call('V08_MODEL_WRONG_REVIEWER', submitter, 'gate-decide', pass, '2', { decision: 'PASS' }, 'DENIED');
    if (foreign) await call('V08_MODEL_CROSS_PROJECT', submitter, 'gate-submit', foreign.gateId,
      String(before.foreignProject.gate.revision), {}, 'DENIED');
    await call('V08_MODEL_EMPTY_REJECT_REASON', reviewer, 'gate-decide', pass, '2',
      { decision: 'REJECT', reason: '' }, 'INVALID_INPUT');
    await call('V08_APPROVE', reviewer, 'gate-decide', pass, '2', { decision: 'PASS' }, 'APPLIED', 'PASSED');
    policyRevision = (BigInt(policyRevision) + 1n).toString();
    await call('V08_LEGAL_STAGE', submitter, 'stage-transition', pass, '3', {}, 'APPLIED', 'ADVANCED', toStage);
    if (c.host) {
      const h = c.host;
      requireFact(h.lifecycleOwnership === 'EXCLUSIVE_V08_HOST_RECIPIENTS' &&
        (h.busyToIdleOnly === undefined || typeof h.busyToIdleOnly === 'boolean') &&
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
        const target = host.kind === 'DELIVERED' ?
          journal.sessions.find(row => row.id === host.autoObservation?.sessionId) :
          journal.sessions.find(row => row.id === host.busy?.binding.id);
        requireFact(target, 'Checkpoint requires the original live observed recipient before any close');
        const stopped = [submitter, reviewer, target];
        host.checkpointStops = []; product.save();
        for (const session of stopped) {
          await output(session, Boolean(host.busy && session.id === target.id));
          const readRequestId = journal.operations.at(-1).request.requestId;
          const revision = session.revision;
          const receipt = await c.stopRulesSession(session);
          const entry = journal.operations.at(-1);
          requireFact(entry.request.family === 'K-SESSION' && entry.request.operation === 'stop' &&
            entry.request.targetId === session.id && entry.request.expectedRevision === revision &&
            entry.request.payload.generation === session.generation &&
            entry.request.payload.seatId === session.seatId && receipt === entry.receipt &&
            receipt.status === 'APPLIED' && receipt.previousRevision === revision &&
            receipt.revision === (BigInt(revision) + 1n).toString() &&
            receipt.revision === session.revision && typeof receipt.result.stopFact === 'string' &&
            receipt.result.stopFact.length > 0,
          'Checkpoint needs one original H stop-only receipt and genuine StopFact at the observed revision');
          host.checkpointStops.push({ binding: binding(session), revision: session.revision,
            readRequestId, stopRequestId: entry.request.requestId, stopFact: receipt.result.stopFact });
          product.save();
        }
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
          host.kind === 'BUSY_TO_IDLE_DELIVERY' ?
          original.message.state === 'DELIVERED' && original.recipient === null &&
            original.deliveries.length === 1 && original.sends.length === 1 &&
            original.commands.length === 1 && original.message.turn_id === host.existingObservation.turnId &&
            original.message.generation === host.busy.binding.generation :
          original.message.state === (host.kind === 'CANCELLED' ? 'CANCELLED' : 'PENDING') &&
            original.message.turn_id === '' &&
            original.message.generation === '' && original.deliveries.length === 0 &&
            original.sends.length === 0 && original.recipient === null),
        'Actual Host checkpoint must contain the original automatic delivery or genuinely blocked queue');
        for (const session of stopped) {
          const claim = snapshot.stoppedClaims.find(row => row.session_id === session?.id);
          requireFact(claim?.state === 'STOPPED' && claim.generation === session.generation &&
            claim.instance_id === session.instanceId &&
            claim.stop_fact_id === host.checkpointStops.find(row => row.binding.id === session.id).stopFact &&
            String(claim.revision) === session.revision,
          'Immutable checkpoint must read the original H stop-only claim/StopFact without inventing a stop');
        }
        host.checkpoint = reference; host.messageId = original.message.message_id;
        host.enqueueRequestId = original.enqueue.request_id;
        host.triggerId = original.intent.trigger_id; host.escalationRequestId = original.intent.request_id;
        host.queuedRevision = original.message.revision; product.save();
        if (host.kind === 'DELIVERED') {
          requireFact(target.seatId === h.destination.seatId &&
            target.instanceId === h.destination.instanceId && target.worktreeId === h.destination.worktreeId &&
            target.id === host.autoObservation.sessionId &&
            target.threadId === host.autoObservation.threadId &&
            original.message.turn_id === host.autoObservation.turnId &&
            ['id', 'seatId', 'instanceId', 'worktreeId', 'generation', 'threadId'].every(name =>
              original.autoBinding[name] === target[name]) && original.autoBinding.revision === target.revision,
          'Automatic recipient must be the unique original Host-created H session');
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
          if (card) {
            host.busy.cardId = card.cardId; host.busy.cardRevision = card.revision;
            host.busy.eventStart = session.events.length; product.save(); break;
          }
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
        return reply;
      };
      const completeBusy = async (host, session, recovered) => {
        const answered = await product.operation('K-QCARD', 'answer', host.busy.cardId,
          { generation: session.generation,
            answers: { [h.busyQuestion.questionId]: [h.busyQuestion.optionLabel] } }, recovered.revision);
        host.busy.answerRequestId = journal.operations.at(-1).request.requestId;
        requireFact(answered.status === 'APPLIED' && answered.result.state === 'ANSWERED' &&
          answered.result.deliveryBasis === 'NATIVE_EXACT_WRITE_RECEIPT',
        'Original nonsecret K-QCARD answer must have its exact native write receipt');
        const deadline = Date.now() + 600000;
        while (Date.now() < deadline) {
          const page = await output(session);
          const readRequestId = journal.operations.at(-1).request.requestId;
          const completed = session.events.find(row => row._meta?.codexMethod === 'turn/completed' &&
            row._meta.threadId === session.threadId && row._meta.turnId === host.busy.turnId);
          const idle = session.events.slice(host.busy.eventStart).some(row =>
            row._meta?.codexMethod === 'thread/status/changed' &&
            row._meta.threadId === session.threadId && row._meta.threadStatus?.type === 'idle');
          if (!host.busy.completedReadRequestId && page.events.some(row =>
            row._meta?.codexMethod === 'turn/completed' && row._meta.threadId === session.threadId &&
            row._meta.turnId === host.busy.turnId)) host.busy.completedReadRequestId = readRequestId;
          if (!host.busy.idleReadRequestId && page.events.some(row =>
            row._meta?.codexMethod === 'thread/status/changed' &&
            row._meta.threadId === session.threadId && row._meta.threadStatus?.type === 'idle'))
            host.busy.idleReadRequestId = readRequestId;
          product.save();
          if (completed && idle && host.busy.completedReadRequestId && host.busy.idleReadRequestId) {
            requireFact(completed._meta.turnStatus === 'completed', 'Original answered CLI turn must complete');
            host.busy.completedTurnId = completed._meta.turnId;
            product.save(); return;
          }
          await delay(300);
        }
        throw Error('Original answered H/A turn did not reach positive idle; preserve without stop/resume');
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
          const page = await output(target);
          requireFact(page.generation === target.generation, 'Automatic output must bind the actual live H generation');
          host.autoObservation.outputReadRequestId = journal.operations.at(-1).request.requestId;
          const inputs = page.nativeInputReceipts.filter(row => row.phase === 'RECEIPTED' &&
            row.receipt?.family === 'K-SESSION' && row.receipt.operation === 'send' &&
            row.receipt.targetId === target.id && row.receipt.status === 'APPLIED' &&
            row.receipt.result?.createdTurn === true);
          if (inputs.length === 0) {
            await delay(300); continue; // Observe this original Host send; never resend or substitute.
          }
          if (!(inputs.length === 1 && inputs[0].receipt.result.generation === target.generation &&
              decimal(inputs[0].receipt.revision))) {
            host.state = 'NOT_RUN_NO_ORIGINAL_LIVE_AUTOMATIC_BINDING'; product.save();
            throw Error('V08 automatic recipient NOT_RUN: no unique original live H send ACK; preserve without stop or close');
          }
          host.autoObservation.nativeInputReceipt = inputs[0];
          completed = target.events.find(row => row._meta?.codexMethod === 'turn/completed' &&
            row._meta.turnStatus === 'completed' && row._meta.turnId === inputs[0].receipt.result.turnId);
          if (completed) break;
          await delay(300);
        }
        if (!(completed?._meta?.turnId && completed._meta.threadId)) {
          host.state = 'NOT_RUN_NO_ORIGINAL_LIVE_AUTOMATIC_COMPLETION'; product.save();
          throw Error('V08 automatic recipient NOT_RUN: original live H binding/CLI completion unavailable; preserve without stop or close');
        }
        host.autoObservation = { ...host.autoObservation, generation: target.generation,
          threadId: completed._meta.threadId, turnId: completed._meta.turnId };
        target.threadId = completed._meta.threadId; target.turns = [];
        requireFact(!journal.sessions.some(row => row.id === target.id), 'Automatic live recipient must be unique');
        journal.sessions.push(target);
        product.save();
      };
      const observeExisting = async (host, session) => {
        const deadline = Date.now() + 600000;
        while (Date.now() < deadline) {
          const page = await output(session);
          const matches = page.nativeInputReceipts.filter(row => row.phase === 'RECEIPTED' &&
            row.receipt?.family === 'K-SESSION' && row.receipt.operation === 'send' &&
            row.receipt.targetId === session.id && row.receipt.status === 'APPLIED' &&
            row.receipt.result?.createdTurn === true && row.receipt.result?.generation === session.generation &&
            row.requestId?.startsWith('hostsend-'));
          if (matches.length === 0) { await delay(300); continue; }
          requireFact(matches.length === 1, 'One original Host H send to existing idle recipient required');
          const turnId = matches[0].receipt.result.turnId;
          host.existingObservation = { sessionId: session.id, generation: session.generation,
            threadId: session.threadId, turnId, nativeInputReceipt: matches[0],
            outputReadRequestId: journal.operations.at(-1).request.requestId };
          product.save();
          const completed = session.events.find(row => row._meta?.codexMethod === 'turn/completed' &&
            row._meta.threadId === session.threadId && row._meta.turnId === turnId);
          if (completed) {
            requireFact(completed._meta.turnStatus === 'completed', 'Original Host-delivered CLI turn failed');
            host.existingObservation.completedReadRequestId = journal.operations.at(-1).request.requestId;
            product.save(); return;
          }
          await delay(300);
        }
        throw Error('Original Host delivery to the same idle H turn did not complete; preserve without resend');
      };
      const hostKinds = h.busyToIdleOnly === true ? ['BUSY_TO_IDLE_DELIVERY'] :
        ['DELIVERED', 'BUSY_QUEUED', 'ROUTE_CHANGED', 'CANCELLED'];
      record.hostKinds = hostKinds; product.save();
      for (const kind of hostKinds) {
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
        const latestBusy = busySession ? await readBusy(host, busySession) : null;
        if (kind === 'DELIVERED') await observeAutomatic(host);
        if (kind === 'ROUTE_CHANGED') {
          await configure('policy-escalation-route', { fromSeatId: submitter.seatId,
            reason: 'REJECT_CAP', toSeatId: h.alternateDestination.seatId });
          host.changedRouteRevision = policyRevision; product.save();
        }
        if (kind === 'CANCELLED' || kind === 'BUSY_TO_IDLE_DELIVERY') {
          host.messageId = hostSessionLocator(config.domainId, host.causeEventId, host.policyRevision).messageId;
          host.queuedRevision = '1'; product.save();
          if (kind === 'CANCELLED') await cancel(host);
          else {
            const pending = await inbox(host);
            requireFact(pending.result.state === 'PENDING', 'Original C notice must remain pending while native question is unanswered');
            host.pendingReadRequestId = host.readRequestIds.at(-1); product.save();
            await completeBusy(host, busySession, latestBusy);
            await observeExisting(host, busySession);
          }
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
      record.notRun.push({ caseId: 'V08_HOST_LATE_ACK_AFTER_ROUTE_CHANGE', reason: 'No real deterministic UNKNOWN/late-ACK occurrence is available here; synthetic ACK/faults and input replay are not used.' });
      if (!h.busyToIdleOnly) record.notRun.push({ caseId: 'V08_HOST_BUSY_TO_IDLE_DELIVERY', reason: 'The optional answered-question flow was not selected; stopped busy controls do not prove idle.' });
      else for (const skipped of ['DELIVERED', 'BUSY_QUEUED', 'ROUTE_CHANGED', 'CANCELLED'])
        record.notRun.push({ caseId: `V08_HOST_${skipped}`, reason: 'The optional busy-to-idle-only flow did not execute this separate Host case.' });
    }
    record.state = 'FLOW_COMPLETE_DIRECT_LEDGER_READBACK_REQUIRED'; product.save();
    return record;
  } catch (error) {
    record.state = 'FAILED_OR_NOT_RUN_PRESERVE_ORIGINAL'; record.originalError = String(error.stack ?? error);
    product.save(); throw error;
  }
}
