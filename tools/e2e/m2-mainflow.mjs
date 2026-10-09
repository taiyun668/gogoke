// V11's existing installed-product merge step, extracted for the M2 runner.
// Importing this module has no effects. The caller owns the original H lifecycle.
import { id } from './product-cdp.mjs';

export async function runV11Merge(product, config, journal, {
  lead, captured, check, leadTurn, sessionOp, stopUser,
}) {
  check(config.repositoryId === 'gogokeSeatTestbed' &&
    captured.worktree?.id && captured.worktree?.state === 'REGISTERED' &&
    captured.controllerMergeDecision?.decision === 'MERGE_EXACT_PRIVATE_TEST_MARKER_ONLY' &&
    captured.controllerMergeDecision.scope === config.repositoryId &&
    captured.controllerMergeDecision.childHead === captured.worktree.childHeadBeforeSeal &&
    captured.controllerMergeDecision.markerSha256 === captured.worktree.markerSha256 &&
    JSON.stringify(captured.controllerMergeDecision.changedPaths) === JSON.stringify([journal.markerFile]) &&
    /^[1-9][0-9]*$/.test(captured.controllerMergeDecision.policyRevision),
  'V11 requires the normally closed original F/H testbed capture and narrow merge decision');

  const tree = captured.worktree.id;
  const revision = String(captured.worktree.revision);
  const reason = `M2 original child file ${journal.markerFile}`;
  const record = { state: 'RUNNING', acceptance: false, worktreeId: tree,
    captureRef: journal.readbacks.find(row => row.phase === 'capture') ?? null,
    userMergeBoundary: null, grantRequestId: null, modelMergeTurnId: null,
    graphRequestId: null, graph: null };
  journal.v11Merge = record; product.save();
  check(record.captureRef && /^[a-f0-9]{64}$/.test(record.captureRef.sha256),
    'V11 original normally closed capture artifact must be in the journal');

  // A User K-WORKTREE merge request cannot supply an H-authenticated caller.
  // This is a control against accidentally accepting a User substitute for
  // the one model-authorized merge. It must not create an F MERGE intent.
  const before = await product.operation('K-WORKTREE', 'graph-query', tree, {}, revision);
  check(before.result.state === 'REGISTERED' && before.result.mergeTargetCommit === null,
    'V11 original F tree remains unmerged before User-only control');
  const user = await product.operation('K-WORKTREE', 'merge', tree,
    { decision: 'MERGE', reason }, revision, ['UNSUPPORTED']);
  record.userMergeBoundary = { requestId: journal.operations.at(-1).request.requestId,
    receipt: user, authority: 'ORIGINAL_USER_INGRESS_ONLY' }; product.save();
  check(user.status === 'UNSUPPORTED' && user.revision === revision &&
    JSON.stringify(user.result) === '{}',
  'V11 original User-only merge has no Model authority or effect');
  const after = await product.operation('K-WORKTREE', 'graph-query', tree, {}, revision);
  check(after.result.state === before.result.state &&
    after.result.mergeTargetCommit === before.result.mergeTargetCommit &&
    JSON.stringify(after.result.members) === JSON.stringify(before.result.members),
  'V11 User-only control left the original F graph unchanged');

  const grant = { schema: 'gogoke.37.owner-configuration.v1', command: 'policy-call-grant',
    domainId: config.domainId, requestId: id('m2MergeGrant'), callerSeatId: config.seatId,
    targetId: 'MAIN', action: 'MERGE', expiresAtMs: String(Date.now() + 3600000),
    expectedRevision: captured.controllerMergeDecision.policyRevision };
  const entry = { kind: 'CONTROLLER_TESTBED_MERGE_CONFIGURATION', request: grant,
    rawFrame: JSON.stringify(grant), decision: captured.controllerMergeDecision, receipt: null };
  journal.operations.push(entry); product.save();
  entry.rawReceipt = await product.evaluate(
    `window.__TAURI_INTERNALS__.invoke('gogoke_design37_user_operation',{frame:${JSON.stringify(entry.rawFrame)}})`);
  entry.receipt = JSON.parse(entry.rawReceipt); product.save();
  check(entry.receipt.status === 'APPLIED' && entry.receipt.requestId === grant.requestId &&
    entry.receipt.command === grant.command && entry.receipt.revision ===
    (BigInt(captured.controllerMergeDecision.policyRevision) + 1n).toString(),
  'V11 original narrow MERGE grant CAS receipt');
  record.grantRequestId = grant.requestId; product.save();

  const originalThread = lead.threadId;
  const resumed = await sessionOp(lead, 'resume');
  check(resumed.result.state === 'RUNNING' && resumed.result.newGeneration &&
    (!resumed.result.threadId || resumed.result.threadId === originalThread),
  'V11 original lead H session resumes its own logical thread');
  lead.cursor = '0'; product.save();
  const events = await leadTurn(lead,
    `Owner-authorized M2 merge of the exact registered testbed worktree ${tree}. ` +
    `Call native gogoke_worktree merge with targetId ${tree}, expectedRevision ${revision}, ` +
    `payload decision MERGE and reason ${JSON.stringify(reason)}. ` +
    'Only this original worktree; no User substitute, new create, push or remote operation.', false);
  const completed = events.find(event => event._meta?.codexMethod === 'turn/completed' &&
    event._meta?.turnStatus === 'completed');
  // The immutable reader later binds the exact A call/turn to the F intent,
  // child commit, native reply and source merge; this is only live progress.
  record.modelMergeTurnId = completed?._meta?.turnId ?? null;
  product.save();
  let graph = await product.operation('K-WORKTREE', 'graph-query', tree, {}, revision,
    ['APPLIED', 'STALE']);
  if (graph.status === 'STALE') graph = await product.operation('K-WORKTREE', 'graph-query',
    tree, {}, graph.revision);
  record.graphRequestId = journal.operations.at(-1).request.requestId;
  check(graph.result.state === 'MERGED' && /^[a-f0-9]{40}$/.test(graph.result.mergeTargetCommit),
  'V11 actual F graph reports merged original worktree');
  record.graph = graph; record.state = 'LIVE_MERGE_REQUIRES_NORMAL_CLOSE_DIRECT_READBACK'; product.save();
  await stopUser(lead, true);
  return { graph, record };
}
