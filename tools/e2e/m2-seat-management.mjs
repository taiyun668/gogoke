// Real installed-product K-SEAT cases. Importing this module starts nothing.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { id, sha256 } from './product-cdp.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const atom = value => typeof value === 'string' && /^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/.test(value);
const check = (value, message) => { if (!value) throw Error(message); };

export async function runSeatManagementCases(product, config, journal) {
  const c = config.seatManagement;
  const notRun = (reason = 'No private M2 seat-management fixture was configured.') =>
    ({ state: 'NOT_RUN', reason, acceptance: false });
  if (!c) return notRun();
  check(process.platform === 'win32' && config.repositoryId === 'gogokeSeatTestbed' &&
    c.lifecycleOwnership === 'EXCLUSIVE_M2_SEAT_MANAGEMENT' &&
    /^[a-f0-9]{40}$/.test(config.sourceCommit) && config.domainId === product.config.domainId &&
    product.tester && product.tester.page.url() === product.endpoint.url &&
    journal.connectionBackend?.agentActs === 0 && journal.connectionBackend?.telemetryDisabled === true &&
    !journal.seatManagement && Array.isArray(c.projects) && c.projects.length === 2 &&
    typeof c.normalCloseReadbackRestart === 'function',
  'Seat management requires two exclusive real test domains, original User ingress and close/readback callback');
  const [a, b] = c.projects;
  const seats = new Set(), domains = new Set();
  for (const row of c.projects) {
    check(['domainId', 'templateId', 'seatId'].every(key => atom(row[key])) &&
      row.repositoryId === config.repositoryId && !domains.has(row.domainId) && !seats.has(row.seatId),
    'Two actual project domains require distinct exact template and new seat identities');
    domains.add(row.domainId); seats.add(row.seatId);
  }
  check(a.domainId !== b.domainId && a.seatId !== b.seatId &&
    a.templateId === b.templateId && typeof c.setting === 'string' && atom(c.setting) &&
    typeof c.value === 'string' && atom(c.value) &&
    !/credential|password|token|secret|api.?key/i.test(`${c.setting} ${c.value}`),
  'Cases require the same actual template identity in two domains and a non-secret test edit');
  const forbidden = new Set([config.seatId, config.childSeatId, config.sideChat?.sourceSeatId,
    config.sideChat?.sideSeatId, ...config.providerCases.map(row => row.seatId)]);
  check([...seats].every(value => !forbidden.has(value)), 'Seat-management targets overlap another M2 case');
  if (c.leadSeat) check(domains.has(c.leadSeat.domainId) && atom(c.leadSeat.seatId) &&
    atom(c.leadSeat.parentSeatId) && /^[a-f0-9]{32}$/.test(c.leadSeat.incarnation) &&
    /^(0|[1-9][0-9]*)$/.test(String(c.leadSeat.generation)) &&
    /^[1-9][0-9]*$/.test(String(c.leadSeat.revision)) && !forbidden.has(c.leadSeat.seatId) &&
    !seats.has(c.leadSeat.seatId) && typeof c.leadSeat.value === 'string' && atom(c.leadSeat.value) &&
    !/credential|password|token|secret|api.?key/i.test(c.leadSeat.value),
  'LEAD fixture must be an explicit disjoint producer-bound idle target with safe test value');

  const record = { schema: 'gogoke.37.m2-seat-management.v1', state: 'RUNNING', acceptance: false,
    sourceCommit: config.sourceCommit, domainId: config.domainId, repositoryId: config.repositoryId,
    stateRoot: path.resolve(config.stateRoot), evidenceDirectory: path.resolve(config.evidenceDirectory),
    candidateVersion: config.version, candidateInstalledSha256: config.installedSha256,
    driverSha256: sha256(path.join(here, 'm2-seat-management.mjs')),
    readerSha256: sha256(path.join(here, 'm2-seat-management-readback.py')),
    projects: c.projects.map(row => ({ ...row, createRequestId: null, createReceipt: null,
      tuneRequestId: null, tuneReceipt: null, cardRequestId: null, cardReceipt: null })),
    edit: { setting: c.setting, value: c.value }, refusals: [],
    leadSeat: c.leadSeat ? { ...c.leadSeat, setting: c.setting, tuneRequestId: null, tuneReceipt: null,
      cardRequestId: null, cardReceipt: null, reclaimRequestId: null, reclaimReceipt: null } : null,
    modelAttempt: null, notRun: [], readbacks: [] };
  journal.seatManagement = record; product.save();
  const invoke = async (domainId, operation, targetId, payload, revision = '0', allowed = ['APPLIED']) => {
    const request = { schema: 'gogoke.37.operations.v1', family: 'K-SEAT', operation,
      requestId: id('m2Seat'), domainId, targetId, expectedRevision: String(revision), payload };
    const rawFrame = JSON.stringify(request);
    const entry = { request, rawFrame, startedAt: new Date().toISOString(), receipt: null };
    journal.operations.push(entry); product.save();
    try {
      entry.rawReceipt = await product.evaluate(`window.__TAURI_INTERNALS__.invoke('gogoke_design37_user_operation',{frame:${JSON.stringify(rawFrame)}})`);
      entry.receipt = JSON.parse(entry.rawReceipt); entry.finishedAt = new Date().toISOString(); product.save();
    } catch (error) { entry.originalError = String(error.stack ?? error); product.save(); throw error; }
    check(entry.receipt.schema === request.schema && entry.receipt.requestId === request.requestId &&
      entry.receipt.domainId === domainId && entry.receipt.targetId === targetId &&
      entry.receipt.family === 'K-SEAT' && entry.receipt.operation === operation &&
      allowed.includes(entry.receipt.status),
    `Original ${domainId}/K-SEAT/${operation} returned ${entry.receipt.status}; no replay`);
    return entry;
  };
  const readSeat = async row => {
    let entry = await invoke(row.domainId, 'state-card', row.seatId, {}, '0', ['APPLIED', 'STALE']);
    if (entry.receipt.status === 'STALE') entry = await invoke(row.domainId, 'state-card', row.seatId, {}, entry.receipt.revision);
    check(entry.receipt.result.layer === 'USER' && entry.receipt.result.templateId === row.templateId,
      'Original state-card does not identify this copied USER template seat');
    row.cardRequestId = entry.request.requestId; row.cardReceipt = entry.receipt; product.save();
    return entry.receipt;
  };
  await product.custody(); product.verifyBytes();
  const ui = await product.evaluate('({url:location.href,home:!!document.querySelector(".home-product-entry"),tauri:!!window.__TAURI_INTERNALS__})');
  check(ui.url === product.endpoint.url && ui.home && ui.tauri, 'Actual installed Home/User bridge identity missing');

  await product.custody(); product.verifyBytes();
  const baseline = await c.normalCloseReadbackRestart('baseline');
  const baselineProof = readProof(baseline, 'baseline');
  check(baselineProof.directCaseEvidence === false && baselineProof.projects.length === 2 &&
    baselineProof.projects.every(row => row.targetAbsent === true),
  'Original baseline does not prove two existing templates and absent case-owned targets');
  if (record.leadSeat) check(baselineProof.leadSeat &&
    ['domainId', 'seatId', 'parentSeatId', 'incarnation'].every(key =>
      baselineProof.leadSeat[key] === record.leadSeat[key]) &&
    String(baselineProof.leadSeat.generation) === String(record.leadSeat.generation) &&
    String(baselineProof.leadSeat.revision) === String(record.leadSeat.revision),
  'Original idle LEAD target identities differ from its registered baseline');
  record.baseline = baseline; record.baselineProof = baselineProof; product.save();
  await product.custody(); product.verifyBytes();
  for (const row of record.projects) {
    const created = await invoke(row.domainId, 'create-from-template', row.seatId,
      { layer: 'USER', templateId: row.templateId });
    check(created.receipt.result.layer === 'USER' && created.receipt.result.kind === 'LONG' &&
      created.receipt.result.state === 'IDLE' && created.receipt.result.templateId === row.templateId &&
      created.receipt.revision === '1' && created.receipt.result.settings &&
      typeof created.receipt.result.settings === 'object', 'Original User template copy receipt differs');
    row.createRequestId = created.request.requestId; row.createReceipt = created.receipt; product.save();
  }
  check(JSON.stringify(a.createReceipt.result.settings) === JSON.stringify(b.createReceipt.result.settings),
    'Same-ID source templates did not produce equal initial independent copies');

  const model = c.originalModelAttempt;
  const modelProject = record.projects.find(row => row.domainId === config.domainId);
  if (typeof model === 'function' && atom(c.originalSessionId) && modelProject) {
    const modelValue = `${c.value}-model`;
    const prompt = `Use exactly one native gogoke_seat tool call: operation tune, targetId ${JSON.stringify(modelProject.seatId)}, ` +
      `expectedRevision ${JSON.stringify(modelProject.createReceipt.revision)}, payload {setting:${JSON.stringify(c.setting)},value:${JSON.stringify(modelValue)}}. ` +
      'Do not call another tool, use another seat, or claim a result without the native tool response.';
    const observed = await model({ prompt, domainId: modelProject.domainId, seatId: modelProject.seatId,
      expectedRevision: modelProject.createReceipt.revision, sessionId: c.originalSessionId });
    check(observed && observed.sessionId === c.originalSessionId && observed.domainId === modelProject.domainId &&
      atom(observed.sendRequestId) && atom(observed.turnId),
      'Original-session callback must return its exact H send request and completed turn IDs');
    record.modelAttempt = { sessionId: observed.sessionId, sendRequestId: observed.sendRequestId,
      turnId: observed.turnId, domainId: modelProject.domainId, seatId: modelProject.seatId,
      expectedRevision: modelProject.createReceipt.revision, setting: c.setting, value: modelValue, prompt };
    product.save();
  } else record.notRun.push({ caseId: 'MODEL_DENIAL', reason: 'No original-session callback for either configured project domain.' });

  const tuned = await invoke(a.domainId, 'tune', a.seatId,
    { setting: c.setting, value: c.value }, a.createReceipt.revision);
  check(tuned.receipt.result.layer === 'USER' && tuned.receipt.result.settings?.[c.setting] === c.value &&
    tuned.receipt.revision === '2', 'Original User tune did not change only its copied seat');
  a.tuneRequestId = tuned.request.requestId; a.tuneReceipt = tuned.receipt; product.save();
  const afterA = await readSeat(a);
  const afterB = await readSeat(b);
  check(afterA.result.settings?.[c.setting] === c.value &&
    JSON.stringify(afterB.result.settings) === JSON.stringify(b.createReceipt.result.settings),
  'Actual project B template copy changed with project A edit');

  if (record.leadSeat) {
    const lead = record.leadSeat;
    const tuneLead = await invoke(lead.domainId, 'tune', lead.seatId,
      { setting: c.setting, value: lead.value }, lead.revision);
    check(tuneLead.receipt.result.layer === 'LEAD' && tuneLead.receipt.result.settings?.[c.setting] === lead.value,
      'Original User tune did not update the exact LEAD seat');
    lead.tuneRequestId = tuneLead.request.requestId; lead.tuneReceipt = tuneLead.receipt;
    const leadCard = await invoke(lead.domainId, 'state-card', lead.seatId, {}, tuneLead.receipt.revision);
    check(leadCard.receipt.result.layer === 'LEAD' && leadCard.receipt.result.state === 'IDLE' &&
      leadCard.receipt.result.settings?.[c.setting] === lead.value,
    'Original LEAD state-card did not retain the User edit');
    lead.cardRequestId = leadCard.request.requestId; lead.cardReceipt = leadCard.receipt;
    const reclaimed = await invoke(lead.domainId, 'reclaim', lead.seatId, {}, leadCard.receipt.revision);
    check(reclaimed.receipt.result.layer === 'LEAD' && reclaimed.receipt.result.state === 'RECLAIMED',
      'Original User reclaim receipt did not reclaim the exact LEAD seat');
    lead.reclaimRequestId = reclaimed.request.requestId; lead.reclaimReceipt = reclaimed.receipt;
  } else record.notRun.push({ caseId: 'USER_LEAD_TUNE_AND_RECLAIM',
    reason: 'NOT_RUN_NOT_CONFIGURED: no exclusive registered IDLE LEAD target with producer-bound parent, incarnation, generation and revision.' });
  product.save();
  record.state = 'FLOW_COMPLETE_FINAL_READBACK_REQUIRED'; product.save();
  await product.custody(); product.verifyBytes();
  const final = await c.normalCloseReadbackRestart('final');
  const proof = readProof(final, 'final');
  check(proof.directCaseEvidence === true && proof.projects.length === 2 &&
    proof.projects.every(row => row.targetAbsent === false) &&
    JSON.stringify(proof.rootIdentity) === JSON.stringify(baselineProof.rootIdentity) &&
    JSON.stringify(proof.candidateIdentity) === JSON.stringify(baselineProof.candidateIdentity) &&
    JSON.stringify(proof.candidateInstalledSha256) === JSON.stringify(baselineProof.candidateInstalledSha256),
  'Final readback differs from the baseline physical root or installed candidate');
  await product.custody(); product.verifyBytes();
  record.readbacks.push(final); record.state = 'READBACK_COMPLETE_REVIEW_REQUIRED'; product.save();
  return { state: record.state, acceptance: false, notRun: record.notRun };

  function readProof(reference, phase) {
    check(reference && path.basename(reference.file) === reference.file && /^[a-f0-9]{64}$/.test(reference.sha256),
      `Normal-close immutable ${phase} readback required`);
    const file = path.resolve(config.evidenceDirectory, reference.file);
    check(path.dirname(file) === path.resolve(config.evidenceDirectory) && sha256(file) === reference.sha256,
      `Original ${phase} readback file/hash does not match its private reference`);
    const proof = JSON.parse(fs.readFileSync(file, 'utf8').replace(/^\uFEFF/, ''));
    check(proof.schema === 'gogoke.37.private-m2-seat-management-readback.v1' && proof.phase === phase &&
      proof.caseId === journal.caseId && proof.sourceCommit === config.sourceCommit &&
      proof.domainId === config.domainId && proof.readerSha256 === record.readerSha256 &&
      proof.measurementPreservedDatabaseBytes === true && proof.acceptance === false &&
      proof.stateRoot === record.stateRoot && proof.evidenceDirectory === record.evidenceDirectory &&
      proof.normalClose?.exitCode === 0 && proof.normalClose.forceKill === false &&
      proof.launch?.pid === proof.normalClose.pid && proof.rootIdentity?.observer === 'python-stat' &&
      ['device', 'inode', 'databaseDevice', 'databaseInode'].every(key =>
        typeof proof.rootIdentity[key] === 'string') &&
      proof.candidateIdentity?.sourceCommit === config.sourceCommit &&
      proof.candidateIdentity.version === config.version && proof.candidateIdentity.setId &&
      proof.candidateIdentity.generationId && proof.launch.sourceCommit === config.sourceCommit &&
      JSON.stringify(proof.candidateInstalledSha256) === JSON.stringify(config.installedSha256),
    `Original ${phase} readback is not bound to the current closed candidate and physical root`);
    return proof;
  }
}
