// V11's existing installed-product merge step, extracted for the M2 runner.
// Importing this module has no effects. The caller owns the original H lifecycle.
import { id } from './product-cdp.mjs';
import fs from 'node:fs';
import path from 'node:path';

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

// Original User F graph setup for V11's two-project layout case. Both
// repositories and both IDLE seats must already be Owner-registered in the
// private testbed. Physical separation remains for a normal-close reader.
export async function runV11GraphFacts(product, config, journal) {
  const c = config.v11Graph;
  const atom = value => typeof value === 'string' && /^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/.test(value);
  const requireFact = (condition, message) => { if (!condition) throw Error(message); };
  requireFact(process.platform === 'win32' && config.repositoryId === 'gogokeSeatTestbed' &&
    c?.ownership === 'EXCLUSIVE_V11_GRAPH_TWO_PROJECTS' &&
    atom(c.secondaryRepositoryId) && c.secondaryRepositoryId !== config.repositoryId &&
    [config.stateRoot, config.testbedSource, c.secondarySource].every(value =>
      typeof value === 'string' && path.isAbsolute(value) && fs.existsSync(value) &&
      fs.statSync(value).isDirectory()) &&
    path.resolve(c.secondarySource) !== path.resolve(config.testbedSource) &&
    Array.isArray(c.groups) && c.groups.length === 2 && !journal.v11Graph,
  'V11 graph requires two exclusive project domains and an Owner-registered second test repository');
  const identities = new Set(), trees = new Set();
  for (const group of c.groups) {
    requireFact(atom(group.domainId) && atom(group.seatId) && atom(group.instanceId) &&
      group.domainId !== config.domainId && !identities.has(group.domainId) &&
      ['singleTreeId', 'mixedPrimaryId', 'mixedSiblingId'].every(key => atom(group[key]) && !trees.has(group[key])),
    'V11 graph groups require distinct fresh domains and worktree IDs outside mainflow');
    identities.add(group.domainId);
    for (const key of ['singleTreeId', 'mixedPrimaryId', 'mixedSiblingId']) trees.add(group[key]);
  }
  requireFact(c.groups[0].seatId !== c.groups[1].seatId &&
    ![config.seatId, config.childSeatId].includes(c.groups[0].seatId) &&
    ![config.seatId, config.childSeatId].includes(c.groups[1].seatId),
  'V11 graph must not reuse a mainflow seat');
  const record = { state: 'RUNNING', acceptance: false,
    sourceCommit: config.sourceCommit, candidateVersion: config.version,
    installedSha256: config.installedSha256, stateRoot: config.stateRoot,
    sources: { [config.repositoryId]: config.testbedSource,
      [c.secondaryRepositoryId]: c.secondarySource },
    repositoryIds: [config.repositoryId, c.secondaryRepositoryId],
    operations: [], graphs: [], notRun: ['PHYSICAL_SEPARATION_CLOSED_READER', 'MODEL_H_WRITE_SCOPE'] };
  journal.v11Graph = record; product.save();
  const operation = async (domainId, family, verb, targetId, payload, revision, allowed = ['APPLIED']) => {
    const request = { schema: 'gogoke.37.operations.v1', family, operation: verb,
      requestId: id('v11Graph'), domainId, targetId, expectedRevision: revision, payload };
    const rawFrame = JSON.stringify(request);
    const entry = { kind: 'V11_ORIGINAL_USER_GRAPH', request, rawFrame, receipt: null };
    journal.operations.push(entry); record.operations.push(request.requestId); product.save();
    try {
      entry.rawReceipt = await product.evaluate(
        `window.__TAURI_INTERNALS__.invoke('gogoke_design37_user_operation',{frame:${JSON.stringify(rawFrame)}})`);
      entry.receipt = JSON.parse(entry.rawReceipt); product.save();
      requireFact(entry.receipt.schema === request.schema && entry.receipt.requestId === request.requestId &&
        entry.receipt.domainId === domainId && entry.receipt.family === family &&
        entry.receipt.operation === verb && entry.receipt.targetId === targetId &&
        allowed.includes(entry.receipt.status),
      `V11 original ${domainId}/${family}/${verb} result=${entry.receipt.status}; preserve without replay`);
      return entry.receipt;
    } catch (error) { entry.originalError = String(error.stack ?? error); product.save(); throw error; }
  };
  const graph = async (domainId, tree) => {
    let receipt = await operation(domainId, 'K-WORKTREE', 'graph-query', tree, {}, '0', ['APPLIED', 'STALE']);
    if (receipt.status === 'STALE') receipt = await operation(domainId, 'K-WORKTREE', 'graph-query', tree, {}, receipt.revision);
    return receipt;
  };
  for (const group of c.groups) {
    let card = await operation(group.domainId, 'K-SEAT', 'state-card', group.seatId, {}, '0', ['APPLIED', 'STALE']);
    if (card.status === 'STALE') card = await operation(group.domainId, 'K-SEAT', 'state-card', group.seatId, {}, card.revision);
    requireFact(card.result.state === 'IDLE' && card.result.instanceId === group.instanceId,
      'V11 original graph seat must be IDLE on its bound instance');
    for (const [tree, repository, layout] of [
      [group.singleTreeId, config.repositoryId, 'single'],
      [group.mixedPrimaryId, config.repositoryId, 'mixed'],
      [group.mixedSiblingId, c.secondaryRepositoryId, 'mixed'],
    ]) {
      const created = await operation(group.domainId, 'K-WORKTREE', 'create', tree,
        { repositoryId: repository, seatId: group.seatId, layout }, '0');
      requireFact(created.result.worktreeId === tree && created.result.repositoryId === repository &&
        created.result.seatId === group.seatId && created.result.state === 'CREATED' &&
        created.result.classification === layout.toUpperCase(),
      'V11 original F create receipt differs from requested layout');
      const registered = await operation(group.domainId, 'K-WORKTREE', 'register', tree, {}, '1');
      requireFact(registered.result.worktreeId === tree, 'V11 original F registration differs');
      const current = await graph(group.domainId, tree);
      requireFact(current.result.state === 'REGISTERED' &&
        current.result.classification === layout.toUpperCase() &&
        current.result.members?.some(row => row.worktreeId === tree && row.repositoryId === repository &&
          row.domainId === group.domainId && row.seatId === group.seatId &&
          row.instanceId === group.instanceId),
      'V11 original F graph did not report its actual member');
      record.graphs.push({ domainId: group.domainId, seatId: group.seatId,
        instanceId: group.instanceId, worktreeId: tree, repositoryId: repository,
        layout, spaceId: current.result.spaceId, graphRequestId: journal.operations.at(-1).request.requestId,
        graph: current.result }); product.save();
    }
    // The first MIXED graph was observed before its second member existed.
    // Read both original members again after registration; never treat that
    // earlier one-member page as the final native space state.
    for (const tree of [group.mixedPrimaryId, group.mixedSiblingId]) {
      const current = await graph(group.domainId, tree);
      const fact = record.graphs.find(row => row.worktreeId === tree);
      requireFact(fact && current.result.spaceId === fact.spaceId &&
        current.result.state === 'REGISTERED' && current.result.members?.length === 2,
      'V11 original final MIXED graph lacks its registered second member');
      fact.graphRequestId = journal.operations.at(-1).request.requestId;
      fact.graph = current.result; product.save();
    }
  }
  const [a, b] = c.groups.map(group => ({
    single: record.graphs.find(row => row.worktreeId === group.singleTreeId),
    primary: record.graphs.find(row => row.worktreeId === group.mixedPrimaryId),
    sibling: record.graphs.find(row => row.worktreeId === group.mixedSiblingId),
  }));
  for (const row of [a, b]) requireFact(row.primary.spaceId === row.sibling.spaceId &&
    row.single.spaceId !== row.primary.spaceId && row.primary.graph.members.length === 2 &&
    row.sibling.graph.members.length === 2,
  'V11 each project must have one distinct SINGLE and a two-repository MIXED group');
  requireFact(a.primary.spaceId !== b.primary.spaceId && a.single.spaceId !== b.single.spaceId &&
    [a.primary, b.primary].every(row => row.repositoryId === config.repositoryId) &&
    [a.primary, a.sibling].every(row => row.graph.members.every(member => member.domainId === a.primary.domainId)) &&
    [b.primary, b.sibling].every(row => row.graph.members.every(member => member.domainId === b.primary.domainId)),
  'V11 one repository serves two projects without crossing their MIXED spaces');
  record.state = 'ORIGINAL_USER_F_GRAPHS_RECORDED_PHYSICAL_READBACK_REQUIRED'; product.save();
  return record;
}
