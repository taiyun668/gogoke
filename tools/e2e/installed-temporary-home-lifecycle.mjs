// Shared per-case ownership and finally flow for temporary SESSION HOME rows.
// This module owns request provenance only; native remains the sole HOME writer.
import fs from 'node:fs';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { isDeepStrictEqual } from 'node:util';
import { createStartupHomeLifecycle } from './installed-startup-home-lifecycle.mjs';
import { requireRecordedNormalClose } from './causal-close/causal-close.mjs';

const digest = value => createHash('sha256').update(value, 'utf8').digest('hex');
const fileDigest = file => createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const nonempty = value => typeof value === 'string' && value.length > 0;
const createHomeFor = request => request?.family === 'K-SESSION' &&
  ['admission-reserve', 'resume'].includes(request.operation);

export function exactAdmissionResubmissionReply(record) {
  const q = record?.request, r = record?.receipt;
  const applied = ['APPLIED', 'REPLAYED'].includes(r?.status);
  return q?.schema === 'gogoke.37.operations.v1' && q.family === 'K-SESSION' &&
    q.operation === 'admission-reserve' && q.expectedRevision === '0' &&
    typeof q.payload?.generation === 'string' && /^(0|[1-9][0-9]*)$/.test(q.payload.generation) &&
    record.rawFrame === JSON.stringify(q) && r?.schema === q.schema &&
    r.requestId === q.requestId && r.targetId === q.targetId &&
    r.family === q.family && r.operation === q.operation &&
    ['APPLIED', 'REPLAYED', 'DENIED', 'UNKNOWN', 'CONFLICT', 'STALE'].includes(r.status) &&
    r.previousRevision === q.expectedRevision && r.revision === (applied ? '1' : '0') &&
    r.result !== null && typeof r.result === 'object' && !Array.isArray(r.result) &&
    (applied ? r.result.generation === q.payload.generation && Object.keys(r.result).join('|') === 'generation'
      : r.result.generation === undefined || r.result.generation === q.payload.generation);
}

export function assertSupportedTemporaryHomeOperation(request) {
  if (request?.family === 'K-SESSION' && ['compact', 'renew-session'].includes(request.operation)) {
    throw Error('Temporary-HOME operation blocked: compact/renew-session creation ownership and closure are not instrumented');
  }
}

function persist(manifestPath, manifest) {
  const temporary = `${manifestPath}.tmp`;
  fs.writeFileSync(temporary, `${JSON.stringify(manifest, null, 2)}\n`, { flag: 'w' });
  fs.renameSync(temporary, manifestPath);
}

function stopProof(record) {
  const request = record?.request;
  const receipt = record?.receipt;
  const exactReply = request?.schema === 'gogoke.37.operations.v1' && receipt?.schema === request.schema &&
    record.rawFrame === JSON.stringify(request) &&
    receipt?.requestId === request.requestId && receipt?.targetId === request.targetId &&
    receipt?.family === request.family && receipt?.operation === request.operation;
  return exactReply && request.family === 'K-SESSION' && request.operation === 'stop' &&
    Object.keys(request.payload ?? {}).sort().join('|') === 'generation|seatId' &&
    ['APPLIED', 'REPLAYED'].includes(receipt.status) &&
    typeof receipt.result?.stopFact === 'string' && /^sha256:[a-f0-9]{64}$/.test(receipt.result.stopFact)
    ? receipt.result.stopFact : null;
}

function candidateStopProof(record, attempt, producer, original) {
  const request = record?.request, receipt = record?.receipt, result = receipt?.result;
  const resume = original?.request, failed = original?.receipt;
  if (attempt.operation !== 'resume' || nonempty(attempt.generation) ||
      !matchesRequestReply(original, 'K-SESSION', 'resume', ['UNKNOWN']) ||
      original.rawFrame !== attempt.originalCreateRawFrame ||
      digest(original.rawFrame) !== attempt.originalCreateRawSha256 ||
      !isDeepStrictEqual(failed, attempt.receipt) ||
      resume.domainId !== attempt.domainId || resume.targetId !== attempt.sessionId ||
      resume.requestId !== attempt.originalCreateRequestId ||
      Object.keys(resume.payload ?? {}).join('|') !== 'generation' ||
      resume.payload.generation !== attempt.producerGeneration ||
      failed.previousRevision !== resume.expectedRevision ||
      !/^(0|[1-9][0-9]*)$/.test(failed.revision ?? '') ||
      request?.schema !== 'gogoke.37.operations.v1' || request.family !== 'K-SESSION' ||
      request.operation !== 'stop' || request.domainId !== attempt.domainId ||
      request.targetId !== attempt.sessionId || request.expectedRevision !== failed.revision ||
      request.payload?.seatId !== producer.seatId ||
      request.payload?.generation !== attempt.producerGeneration ||
      request.payload?.resumeRequestId !== resume.requestId ||
      Object.keys(request.payload ?? {}).sort().join('|') !== 'generation|resumeRequestId|seatId' ||
      record.rawFrame !== JSON.stringify(request) ||
      receipt?.schema !== request.schema || receipt.family !== request.family ||
      receipt.operation !== request.operation || receipt.requestId !== request.requestId ||
      receipt.targetId !== request.targetId || !['APPLIED', 'REPLAYED'].includes(receipt.status) ||
      receipt.previousRevision !== request.expectedRevision ||
      receipt.revision !== request.expectedRevision ||
      result?.stopScope !== 'ORIGINAL_RESUME_CANDIDATE' ||
      result.originalResumeRequestId !== resume.requestId ||
      typeof result.stoppedGeneration !== 'string' ||
      !/^[1-9][0-9]*$/.test(result.stoppedGeneration) ||
      result.stoppedGeneration === attempt.producerGeneration ||
      !/^sha256:[a-f0-9]{64}$/.test(result.stopFact ?? '') ||
      Object.keys(result).sort().join('|') !==
        'originalResumeRequestId|stopFact|stopScope|stoppedGeneration') return null;
  return { requestId: request.requestId, rawFrame: record.rawFrame,
    rawSha256: digest(record.rawFrame), receipt, stoppedGeneration: result.stoppedGeneration };
}

function matchesRequestReply(record, family, operation, allowed) {
  const request = record?.request;
  const receipt = record?.receipt;
  return request?.schema === 'gogoke.37.operations.v1' && receipt?.schema === request.schema &&
    request.family === family && request.operation === operation &&
    record.rawFrame === JSON.stringify(request) && receipt?.requestId === request.requestId &&
    receipt?.targetId === request.targetId && receipt?.family === family &&
    receipt?.operation === operation && allowed.includes(receipt.status);
}

function closeReceipt(record, requestId, targetId) {
  const receipt = record?.receipt;
  const result = receipt?.result;
  return receipt?.schema === 'gogoke.37.operations.v1' &&
    receipt.requestId === requestId && receipt?.targetId === targetId &&
    receipt?.family === 'K-INSTANCE' && receipt?.operation === 'temporary-home-close' &&
    ['APPLIED', 'REPLAYED'].includes(receipt.status) &&
    receipt.previousRevision === '1' && receipt.revision === '2' &&
    nonempty(result?.homeId) && nonempty(result?.nativeReceiptId) &&
    nonempty(result?.proofKind) && nonempty(result?.sourceFingerprint) &&
    result?.state === 'CLOSED';
}

function releaseReceipt(record, attempt, producer, stopRevision) {
  const request = record?.request;
  const receipt = record?.receipt;
  return request?.schema === 'gogoke.37.operations.v1' && receipt?.schema === request.schema &&
    request.family === 'K-SESSION' && request.operation === 'admission-release' &&
    request.domainId === attempt.domainId && request.targetId === attempt.sessionId &&
    typeof stopRevision === 'string' && /^(0|[1-9][0-9]*)$/.test(stopRevision) &&
    request.expectedRevision === stopRevision &&
    request.payload?.generation === String(attempt.generation) &&
    request.payload?.seatId === producer.seatId &&
    Object.keys(request.payload ?? {}).sort().join('|') === 'generation|seatId' &&
    record.rawFrame === JSON.stringify(request) &&
    receipt?.requestId === request.requestId && receipt?.targetId === request.targetId &&
    receipt?.family === request.family && receipt?.operation === request.operation &&
    ['APPLIED', 'REPLAYED'].includes(receipt.status) &&
    receipt.result?.generation === String(attempt.generation) &&
    receipt.previousRevision === request.expectedRevision &&
    receipt.revision === String(BigInt(stopRevision) + 1n);
}

export function createTemporaryHomeLifecycle(product, manifestPath, restore = null) {
  if (!path.isAbsolute(manifestPath) || fs.existsSync(manifestPath) || fs.existsSync(`${manifestPath}.tmp`)) {
    throw Error('Fresh private temporary-HOME attempt manifest path required');
  }
  const journal = product.journal;
  const domainId = product.config.domainId;
  const config = product.config;
  const declaredDomains = config.temporaryHomeDomains;
  const ownedDomains = declaredDomains === undefined ? [domainId] : declaredDomains;
  if (!nonempty(domainId) || !Array.isArray(ownedDomains) || ownedDomains.length === 0 ||
      new Set(ownedDomains).size !== ownedDomains.length || !ownedDomains.includes(domainId) ||
      ownedDomains.some(value => !nonempty(value)) ||
      (declaredDomains !== undefined && ownedDomains.length < 2)) {
    throw Error('Temporary-HOME owned domains must be unique and include the startup domain');
  }
  const readbackConfig = config.temporaryHomeReadback;
  const restored = restore !== null;
  const originalManifestOrigin = restore?.originalManifestOrigin ?? manifestPath;
  const readbackDirectory = restored
    ? path.dirname(path.resolve(originalManifestOrigin)) : config.evidenceDirectory;
  if (restored) {
    const configuredPins = config.temporaryHomeReadback;
    const originalPins = restore.readbackSourcePins;
    if (!path.isAbsolute(originalManifestOrigin) || !fs.existsSync(originalManifestOrigin) ||
        !/^[a-f0-9]{64}$/.test(restore.originalManifestSha256 ?? '') ||
        fileDigest(originalManifestOrigin) !== restore.originalManifestSha256 ||
        path.resolve(originalManifestOrigin) === path.resolve(manifestPath) ||
        path.dirname(path.resolve(originalManifestOrigin)) !== path.dirname(path.resolve(manifestPath)) ||
        !restore.manifest || restore.manifest.schema !== 'gogoke.37.temporary-home-attempts.v1' ||
        restore.manifest.caseId !== journal.caseId || restore.manifest.domainId !== domainId ||
        !isDeepStrictEqual(restore.manifest.ownedDomains ?? [domainId], ownedDomains) ||
        !Array.isArray(restore.manifest.cases) || !Array.isArray(restore.manifest.producers) ||
        !Array.isArray(restore.manifest.attempts) || !Array.isArray(restore.manifest.authorizedRepairs) ||
        !restore.baselineReference?.path || !/^[a-f0-9]{64}$/.test(restore.baselineReference.sha256) ||
        !fs.existsSync(restore.baselineReference.path) ||
        fileDigest(restore.baselineReference.path) !== restore.baselineReference.sha256 ||
        (restore.readbackDirectory && (!path.isAbsolute(restore.readbackDirectory) ||
          path.resolve(restore.readbackDirectory) !== readbackDirectory)) ||
        !originalPins || !configuredPins ||
        configuredPins.path !== originalPins.path || configuredPins.sha256 !== originalPins.sha256 ||
        configuredPins.corePath !== originalPins.corePath || configuredPins.coreSha256 !== originalPins.coreSha256 ||
        !path.isAbsolute(originalPins.path ?? '') || !path.isAbsolute(originalPins.corePath ?? '') ||
        !/^[a-f0-9]{64}$/.test(originalPins.sha256 ?? '') ||
        !/^[a-f0-9]{64}$/.test(originalPins.coreSha256 ?? '') ||
        fileDigest(originalPins.path) !== originalPins.sha256 ||
        fileDigest(originalPins.corePath) !== originalPins.coreSha256) {
      throw Error('Restored temporary-HOME manifest/origin/baseline binding is incomplete');
    }
    if (!isDeepStrictEqual(JSON.parse(fs.readFileSync(originalManifestOrigin, 'utf8')),
      restore.manifest)) throw Error('Restored manifest differs from pinned original content');
    const baseline = JSON.parse(fs.readFileSync(restore.baselineReference.path, 'utf8'));
    const baselineInputPath = path.join(readbackDirectory, 'temporary-home-readback-baseline-input.json');
    if (baseline.schema !== 'gogoke.37.temporary-home-readback.v1' || baseline.mode !== 'baseline' ||
        baseline.state !== 'BASELINE_MEASURED' || baseline.caseId !== journal.caseId ||
        baseline.domainId !== domainId || baseline.source?.manifestOrigin !== originalManifestOrigin ||
        !isDeepStrictEqual(baseline.source.reader, { path: originalPins.path, sha256: originalPins.sha256 }) ||
        !isDeepStrictEqual(baseline.source.core, { path: originalPins.corePath, sha256: originalPins.coreSha256 }) ||
        !fs.existsSync(baselineInputPath) || fileDigest(baselineInputPath) !== baseline.source.inputSha256 ||
        JSON.parse(fs.readFileSync(baselineInputPath, 'utf8')).stateRoot !== config.stateRoot ||
        !baseline.source.manifest?.path ||
        fileDigest(baseline.source.manifest.path) !== baseline.source.manifest.sha256) {
      throw Error('Restored baseline original case/root/reader/source chain differs');
    }
    for (const row of [...restore.manifest.startupAttempts, ...restore.manifest.noStartupHomes]) {
      const source = row.launchSource;
      if (!source?.path || !/^[a-f0-9]{64}$/.test(source.sha256) ||
          !path.isAbsolute(source.path) || !fs.existsSync(source.path) ||
          fileDigest(source.path) !== source.sha256) {
        throw Error('Restored startup source binding is incomplete');
      }
    }
  }
  let activeCaseId = null;
  const manifest = restored ? structuredClone(restore.manifest) : {
    schema: 'gogoke.37.temporary-home-attempts.v1',
    caseId: journal.caseId,
    domainId,
    ...(declaredDomains === undefined ? {} : { ownedDomains: [...ownedDomains] }),
    state: 'RUNNING',
    cases: [],
    producers: [],
    attempts: [],
    notAcceptance: true,
  };
  if (restored) {
    manifest.originalCaseState = restore.manifest.state;
    manifest.restoreOrigin = { path: originalManifestOrigin, sha256: restore.originalManifestSha256 };
    manifest.state = 'RESTORED_CASE_RUNNING';
  }
  persist(manifestPath, manifest);

  const save = () => persist(manifestPath, manifest);
  let baselineReference = restored ? { ...restore.baselineReference } : null;
  const producerFor = request => manifest.producers.find(row =>
    row.domainId === request.domainId && row.sessionId === request.targetId);
  const sessionFor = attempt => journal.sessions.find(row => row.id === attempt.sessionId &&
    (row.domainId ?? domainId) === attempt.domainId);
  const ownedProducerFor = attempt => manifest.producers.find(row =>
    row.domainId === attempt.domainId && row.sessionId === attempt.sessionId);
  const attemptFor = requestId => manifest.attempts.find(row => row.originalCreateRequestId === requestId);
  const candidateAttemptFor = request => request?.family === 'K-SESSION' &&
    request.operation === 'stop' && request.payload?.resumeRequestId
    ? attemptFor(request.payload.resumeRequestId) : null;
  const requireReadbackPins = () => {
    if (!path.isAbsolute(config.python ?? '') || !path.isAbsolute(config.stateRoot ?? '') ||
        !path.isAbsolute(config.evidenceDirectory ?? '') || !readbackConfig ||
        !path.isAbsolute(readbackConfig.path ?? '') || !/^[a-f0-9]{64}$/.test(readbackConfig.sha256 ?? '') ||
        !path.isAbsolute(readbackConfig.corePath ?? '') || !/^[a-f0-9]{64}$/.test(readbackConfig.coreSha256 ?? '') ||
        fileDigest(readbackConfig.path) !== readbackConfig.sha256 ||
        fileDigest(readbackConfig.corePath) !== readbackConfig.coreSha256) {
      throw Error('Pinned temporary-HOME closed-database reader and core instrument are required');
    }
    return { path: readbackConfig.path, sha256: readbackConfig.sha256,
      corePath: readbackConfig.corePath, coreSha256: readbackConfig.coreSha256 };
  };
  const runReadback = async (mode, baseline = null, phase = mode) => {
    const pins = requireReadbackPins();
    if (!/^(baseline|verify|startup-\d+)$/.test(phase)) throw Error('Invalid shared readback phase');
    const inputPath = path.join(readbackDirectory, `temporary-home-readback-${phase}-input.json`);
    const outputPath = path.join(readbackDirectory, `temporary-home-readback-${phase}.json`);
    const snapshotPath = path.join(readbackDirectory, `temporary-home-readback-${phase}-manifest.json`);
    if (fs.existsSync(inputPath) || fs.existsSync(outputPath) || fs.existsSync(snapshotPath)) {
      throw Error(`Temporary-HOME ${phase} readback evidence already exists; no overwrite`);
    }
    // Keep the bytes used by this read even when its failure is subsequently
    // recorded in the live manifest. A pin to that mutable file is not custody.
    fs.writeFileSync(snapshotPath, fs.readFileSync(manifestPath), { flag: 'wx' });
    const input = {
      schema: 'gogoke.37.temporary-home-readback-input.v1', mode, phase,
      stateRoot: config.stateRoot,
      readerSha256: pins.sha256,
      core: { path: pins.corePath, sha256: pins.coreSha256 },
      manifest: { path: snapshotPath, sha256: fileDigest(snapshotPath) },
      manifestOrigin: originalManifestOrigin,
      authorizedRepairs: mode === 'baseline' ? (readbackConfig.authorizedRepairs ?? []) : [],
      baseline,
    };
    fs.writeFileSync(inputPath, `${JSON.stringify(input, null, 2)}\n`, { flag: 'wx' });
    await new Promise((resolve, reject) => {
      const child = spawn(config.python, [pins.path, '--input', inputPath, '--output', outputPath], {
        windowsHide: true, stdio: ['ignore', 'ignore', 'pipe'],
      });
      let stderr = '';
      child.stderr.on('data', bytes => { stderr = (stderr + bytes).slice(-8192); });
      child.once('error', reject);
      child.once('exit', code => code === 0 ? resolve() :
        reject(Error(`Temporary-HOME ${phase} shared readback exit=${code}: ${stderr}`)));
    });
    if (fileDigest(pins.path) !== pins.sha256 || fileDigest(pins.corePath) !== pins.coreSha256 ||
        !fs.existsSync(outputPath)) {
      throw Error(`Temporary-HOME ${phase} reader/core identity changed or output is absent`);
    }
    const report = JSON.parse(fs.readFileSync(outputPath, 'utf8').replace(/^\uFEFF/, ''));
    const expectedState = mode === 'baseline' ? 'BASELINE_MEASURED' :
      mode === 'startup' ? 'STARTUP_SOURCE_MEASURED' :
      (manifest.attempts.length || startup.hasCreatedHomes() ? 'EXACT_OWN_HOMES_CLOSED' : 'NO_HOME_CREATE_DISPATCHED');
    if (report.schema !== 'gogoke.37.temporary-home-readback.v1' || report.mode !== mode ||
        report.acceptance !== false || report.state !== expectedState) {
      throw Error(`Temporary-HOME ${phase} shared readback was not qualified: ${report.state ?? 'missing state'}`);
    }
    const reference = { file: path.basename(outputPath), sha256: fileDigest(outputPath),
      input: path.basename(inputPath), inputSha256: fileDigest(inputPath),
      readerSha256: pins.sha256, coreSha256: pins.coreSha256 };
    journal.temporaryHomeReadback ??= {};
    journal.temporaryHomeReadback[phase] = reference;
    product.save();
    return { report, reference };
  };
  const hEntries = (ownerDomain, sessionId, operation, generation) => journal.operations.filter(row =>
    row.request?.family === 'K-SESSION' && row.request.targetId === sessionId &&
    row.request.domainId === ownerDomain &&
    row.request.operation === operation &&
    (generation === undefined || String(row.request.payload?.generation ?? '') === String(generation)));
  const ordinaryStops = (ownerDomain, sessionId, generation) => hEntries(ownerDomain, sessionId, 'stop', generation)
    .filter(row => Object.keys(row.request.payload ?? {}).sort().join('|') === 'generation|seatId');
  const releaseSourceFor = (attempt, producer, stopped, visited = new Set()) => {
    if (visited.has(attempt.originalCreateRequestId)) return null;
    visited.add(attempt.originalCreateRequestId);
    const revision = stopped.receipt.revision;
    const resumes = hEntries(attempt.domainId, attempt.sessionId, 'resume', attempt.generation);
    if (resumes.length === 0) return { revision, promoted: false };
    if (resumes.length !== 1) return null;
    const original = resumes[0];
    const children = manifest.attempts.filter(row => row.caseId === attempt.caseId &&
      row.domainId === attempt.domainId && row.sessionId === attempt.sessionId &&
      row.instanceId === attempt.instanceId && row.operation === 'resume' &&
      row.producerGeneration === attempt.generation &&
      row.originalCreateRequestId === original.request.requestId);
    if (children.length !== 1 || !originalClosed(children[0]) ||
        typeof revision !== 'string' || !/^(0|[1-9][0-9]*)$/.test(revision) ||
        original.request.expectedRevision !== revision ||
        original.receipt?.previousRevision !== revision ||
        original.receipt.revision !== String(BigInt(revision) + 1n)) return null;
    const child = children[0];
    if (!matchesRequestReply(original, 'K-SESSION', 'resume', ['UNKNOWN', 'APPLIED', 'REPLAYED']) ||
        original.rawFrame !== child.originalCreateRawFrame ||
        digest(original.rawFrame) !== child.originalCreateRawSha256 ||
        !isDeepStrictEqual(original.receipt, child.receipt) ||
        Object.keys(original.request.payload ?? {}).join('|') !== 'generation' ||
        original.request.payload.generation !== attempt.generation) return null;
    if (['APPLIED', 'REPLAYED'].includes(original.receipt.status)) {
      const generation = original.receipt.result?.newGeneration;
      if (typeof generation !== 'string' || !/^[1-9][0-9]*$/.test(generation) ||
          generation === attempt.generation || child.generation !== generation) return null;
      const stops = ordinaryStops(child.domainId, child.sessionId, generation);
      if (stops.length !== 1 || !stopProof(stops[0]) ||
          !isDeepStrictEqual(child.hStopSource, { requestId: stops[0].request.requestId,
            rawFrame: stops[0].rawFrame, rawSha256: digest(stops[0].rawFrame), stopFact: stopProof(stops[0]) })) return null;
      const successor = releaseSourceFor(child, producer, stops[0], visited);
      if (!successor) return null;
      if (!successor.promoted) {
        const releases = journal.operations.filter(row => row.request?.requestId === child.releaseRequestId);
        if (releases.length !== 1 || !releaseReceipt(releases[0], child, producer, successor.revision) ||
            !isDeepStrictEqual(child.hReleaseSource, { requestId: releases[0].request.requestId,
              rawFrame: releases[0].rawFrame, rawSha256: digest(releases[0].rawFrame),
              receipt: releases[0].receipt })) return null;
      }
      // The current claim belongs to the settled descendant. Native closes
      // this old HOME from its original stop and mapped promotion ancestry.
      return { revision: null, promoted: true };
    }
    const candidates = journal.operations.filter(row => row.request?.requestId === child.candidateStopRequestId);
    const source = candidates.length === 1 && candidateStopProof(candidates[0], child, producer, original);
    return source && isDeepStrictEqual(source, child.candidateStopSource)
      ? { revision: original.receipt.revision, promoted: false } : null;
  };
  const originalClosed = attempt => {
    const records = journal.operations.filter(row => row.request?.requestId === attempt.closeRequestId);
    const record = records[0], request = record?.request;
    return attempt.closeDispatchState === 'RECEIPT_RECORDED' && records.length === 1 &&
      matchesRequestReply(record, 'K-INSTANCE', 'temporary-home-close', ['APPLIED', 'REPLAYED']) &&
      request.domainId === attempt.domainId && request.targetId === attempt.instanceId &&
      request.expectedRevision === '1' && request.payload?.sessionId === attempt.sessionId &&
      request.payload?.originalCreateRequestId === attempt.originalCreateRequestId &&
      Object.keys(request.payload ?? {}).sort().join('|') === 'originalCreateRequestId|sessionId' &&
      record.rawFrame === attempt.closeRawFrame && digest(record.rawFrame) === attempt.closeRawSha256 &&
      isDeepStrictEqual(record.receipt, attempt.closeReceipt) &&
      closeReceipt(record, attempt.closeRequestId, attempt.instanceId);
  };

  const startupRestore = restored ? (restore.startupState ?? {
    current: manifest.startupAttempts.length ? manifest.startupAttempts.at(-1) : null,
    expectedNoCreate: manifest.noStartupHomes.length ? manifest.noStartupHomes.at(-1) : null,
    launches: manifest.startupAttempts.length,
  }) : null;
  const startup = createStartupHomeLifecycle(product, manifest, save, runReadback, () => baselineReference,
    restored ? { ...startupRestore, originalManifestOrigin,
      originalManifestSha256: restore.originalManifestSha256 } : null);
  save();
  const lifecycle = {
    manifestPath,
    manifest,
    async captureBaseline() {
      if (restored) throw Error('Restored temporary-HOME lifecycle cannot recapture the original baseline');
      if (manifest.attempts.length !== 0) throw Error('Temporary-HOME baseline must precede every create attempt');
      const { report, reference } = await runReadback('baseline', null);
      baselineReference = { path: path.join(config.evidenceDirectory, reference.file),
        sha256: reference.sha256 };
      startup.initializeRepairs(report);
      return report;
    },
    beforeProductLaunch: () => startup.beforeLaunch(),
    afterProductLaunch: () => startup.afterLaunch(),
    observeRetainedNoStartup() {
      if (!restored || activeCaseId !== journal.caseId)
        throw Error('Retained startup metadata read requires the restored original case');
      return startup.observeRetainedNoStartup();
    },
    beginCase(caseId) {
      if (restored) throw Error('Restored temporary-HOME lifecycle requires restoreCase; beginCase would create a new anchor');
      if (!nonempty(caseId) || activeCaseId !== null || manifest.cases.some(row => row.caseId === caseId)) {
        throw Error('Unique temporary-HOME case anchor required');
      }
      manifest.cases.push({ caseId, state: 'RUNNING', attempts: [] });
      activeCaseId = caseId;
      save();
    },
    restoreCase(caseId) {
      if (!restored || !nonempty(caseId) || activeCaseId !== null) {
        throw Error('Restored temporary-HOME case anchor is not available');
      }
      const rows = manifest.cases.filter(row => row.caseId === caseId);
      if (rows.length !== 1 || activeCaseId !== null) throw Error('Restored temporary-HOME case is not unique');
      activeCaseId = caseId;
      rows[0].originalState = rows[0].originalState ?? rows[0].state;
      rows[0].state = 'RUNNING';
      save();
      return rows[0];
    },
    restoreProducer(session, instanceId, caseId = journal.caseId) {
      if (!restored || !session || !nonempty(session.id) || !nonempty(instanceId) ||
          !Array.isArray(journal.sessions) || !journal.sessions.includes(session) ||
          caseId !== activeCaseId) {
        throw Error('Restored temporary-HOME producer requires the exact existing session');
      }
      const rows = manifest.producers.filter(row => row.sessionId === session.id &&
        row.domainId === (session.domainId ?? domainId) && row.instanceId === instanceId && row.caseId === caseId);
      if (rows.length !== 1) throw Error('Restored temporary-HOME producer is not unique');
      return rows[0];
    },
    beforeAdmissionResubmission(originalRecord, record, continuation = {}) {
      if (!restored) throw Error('Denied admission resubmission requires a restored lifecycle');
      const request = originalRecord?.request;
      const attempt = attemptFor(request?.requestId);
      const producer = request && producerFor(request);
      if (!attempt || !producer || activeCaseId !== attempt.caseId ||
          !manifest.cases.some(row => row.caseId === attempt.caseId) ||
          producer.caseId !== attempt.caseId || producer.instanceId !== attempt.instanceId ||
          request.domainId !== attempt.domainId || request.targetId !== attempt.sessionId ||
          request.payload?.seatId !== producer.seatId ||
          String(request.payload?.generation) !== String(attempt.producerGeneration) ||
          String(attempt.generation) !== String(attempt.producerGeneration) ||
          !journal.operations.includes(originalRecord) ||
          originalRecord.rawFrame !== attempt.originalCreateRawFrame ||
          digest(originalRecord.rawFrame) !== attempt.originalCreateRawSha256 ||
          !isDeepStrictEqual(originalRecord.receipt, attempt.receipt) ||
          !matchesRequestReply(originalRecord, 'K-SESSION', 'admission-reserve', ['DENIED']) ||
          attempt.closeDispatchState !== 'NOT_DISPATCHED' || attempt.resubmissions?.length) {
        throw Error('Denied admission resubmission is not the exact first owned attempt');
      }
      attempt.originalReceipt = structuredClone(attempt.receipt);
      attempt.resubmissions = [{ requestId: request.requestId,
        rawFrame: record.rawFrame, rawSha256: record.originalRawSha256,
        dispatchState: 'PERSISTED_BEFORE_DISPATCH', continuation }];
      record.temporaryHomeResubmissionRequestId = request.requestId;
      save();
    },
    afterAdmissionResubmission(record) {
      const attempt = attemptFor(record.resubmissionOf);
      if (!attempt?.resubmissions || attempt.resubmissions.length !== 1) {
        throw Error('Denied admission resubmission tracking is missing');
      }
      if (!exactAdmissionResubmissionReply(record) ||
          record.rawFrame !== attempt.originalCreateRawFrame ||
          record.request.requestId !== attempt.originalCreateRequestId) {
        throw Error('Denied admission resubmission receipt does not match the original request');
      }
      const observation = attempt.resubmissions[0];
      observation.receipt = record.receipt;
      observation.dispatchState = 'RECEIPT_RECORDED';
      observation.status = record.receipt?.status ?? 'UNKNOWN';
      if (record.receipt?.status === 'APPLIED') {
        attempt.resubmissionReceipt = record.receipt;
        attempt.resubmissionState = 'APPLIED';
        attempt.receipt = record.receipt;
        attempt.dispatchState = 'RESUBMITTED_APPLIED';
      } else {
        attempt.resubmissionState = `REJECTED_${record.receipt?.status ?? 'UNKNOWN'}`;
      }
      save();
    },
    afterAdmissionResubmissionError(record, error) {
      const attempt = attemptFor(record.resubmissionOf);
      if (!attempt?.resubmissions || attempt.resubmissions.length !== 1) return;
      attempt.resubmissions[0].dispatchState = 'DISPATCH_RESULT_UNKNOWN';
      attempt.resubmissions[0].rawReceipt = record.rawReceipt ?? null;
      attempt.resubmissions[0].unqualifiedReceipt = record.receipt ?? null;
      attempt.resubmissions[0].error = String(error?.stack ?? error);
      attempt.resubmissionState = 'UNKNOWN';
      save();
    },
    registerProducer(session, instanceId, caseId = journal.caseId) {
      if (restored) throw Error('Restored lifecycle cannot register another producer');
      if (!session || !nonempty(session.id) || !nonempty(instanceId) ||
          !nonempty(session.seatId) || !nonempty(caseId) || caseId !== activeCaseId ||
          !Array.isArray(journal.sessions) ||
          !journal.sessions.includes(session)) {
        throw Error('Temporary-HOME producer requires the exact journaled session and configured instance');
      }
      const ownerDomain = session.domainId ?? domainId;
      if (!ownedDomains.includes(ownerDomain) ||
          (declaredDomains !== undefined && session.domainId !== ownerDomain)) {
        throw Error('Temporary-HOME producer domain is not an exact owned journal session');
      }
      if (manifest.producers.some(row => row.sessionId === session.id && row.domainId === ownerDomain)) {
        throw Error('Temporary-HOME producer anchor already exists');
      }
      const producer = { caseId, domainId: ownerDomain, sessionId: session.id, instanceId, seatId: session.seatId,
        source: 'CASE_CREATED_SESSION_BEFORE_NATIVE_CREATE_DISPATCH' };
      if (!manifest.cases.some(row => row.caseId === caseId)) {
        manifest.cases.push({ caseId, state: 'RUNNING', attempts: [] });
      }
      manifest.producers.push(producer);
      save();
      return producer;
    },
    beforeDispatch(record) {
      const request = record.request;
      assertSupportedTemporaryHomeOperation(request);
      if (createHomeFor(request)) {
        if (restored) throw Error('Restored lifecycle requires explicit original admission resubmission; no new create');
        const producer = producerFor(request);
        if (!producer || producer.caseId !== activeCaseId || request.domainId !== producer.domainId ||
            request.targetId !== producer.sessionId ||
            (request.operation === 'admission-reserve' && request.payload?.seatId !== producer.seatId) ||
            !nonempty(request.requestId) || typeof record.rawFrame !== 'string' ||
            record.rawFrame !== JSON.stringify(request) ||
            !nonempty(request.payload?.generation)) {
          throw Error('Unanchored temporary-HOME producer; native creation blocked before dispatch');
        }
        if (attemptFor(request.requestId)) throw Error('Duplicate original temporary-HOME create requestId');
        const attempt = {
          caseId: producer.caseId, domainId: producer.domainId,
          sessionId: producer.sessionId, instanceId: producer.instanceId,
          operation: request.operation,
          producerGeneration: String(request.payload.generation),
          generation: request.operation === 'resume' ? null : String(request.payload.generation),
          originalCreateRequestId: request.requestId,
          originalCreateRawFrame: record.rawFrame,
          originalCreateRawSha256: digest(record.rawFrame),
          dispatchState: 'PERSISTED_BEFORE_DISPATCH',
          receipt: null,
          closeDispatchState: 'NOT_DISPATCHED',
          closeRequestId: null,
          closeRawFrame: null,
          closeReceipt: null,
          releaseDispatchState: 'NOT_DISPATCHED',
          releaseRequestId: null,
          releaseRawFrame: null,
          releaseRawSha256: null,
          releaseReceipt: null,
          releaseError: null,
          candidateStopDispatchState: 'NOT_DISPATCHED',
          candidateStopRequestId: null,
          candidateStopRawFrame: null,
          candidateStopRawSha256: null,
          candidateStopReceipt: null,
          candidateStoppedGeneration: null,
          teardownState: 'PENDING',
        };
        manifest.attempts.push(attempt);
        manifest.cases.find(row => row.caseId === producer.caseId).attempts.push(request.requestId);
        record.temporaryHomeAttemptRequestId = request.requestId;
        save();
      } else if (request?.family === 'K-SESSION' && request.operation === 'stop' &&
          Object.hasOwn(request.payload ?? {}, 'resumeRequestId')) {
        const attempt = candidateAttemptFor(request);
        const producer = producerFor(request);
        const originals = journal.operations.filter(row =>
          row.request?.requestId === request.payload.resumeRequestId);
        const original = originals[0];
        if (!attempt || restored || attempt.caseId !== activeCaseId || producer?.caseId !== activeCaseId ||
            producer.instanceId !== attempt.instanceId ||
            request.domainId !== attempt.domainId || request.targetId !== attempt.sessionId ||
            !nonempty(request.requestId) || !nonempty(attempt.producerGeneration) ||
            nonempty(attempt.generation) || attempt.candidateStopDispatchState !== 'NOT_DISPATCHED' ||
            originals.length !== 1 || !matchesRequestReply(original, 'K-SESSION', 'resume', ['UNKNOWN']) ||
            original.rawFrame !== attempt.originalCreateRawFrame ||
            digest(original.rawFrame) !== attempt.originalCreateRawSha256 ||
            !isDeepStrictEqual(original.receipt, attempt.receipt) ||
            original.request.domainId !== attempt.domainId ||
            original.request.targetId !== attempt.sessionId ||
            Object.keys(original.request.payload ?? {}).join('|') !== 'generation' ||
            original.request.payload?.generation !== attempt.producerGeneration ||
            original.receipt.previousRevision !== original.request.expectedRevision ||
            !/^(0|[1-9][0-9]*)$/.test(original.receipt.revision ?? '') ||
            request.expectedRevision !== original.receipt.revision ||
            request.payload?.seatId !== producer.seatId ||
            request.payload?.generation !== attempt.producerGeneration ||
            request.payload?.resumeRequestId !== attempt.originalCreateRequestId ||
            Object.keys(request.payload ?? {}).sort().join('|') !== 'generation|resumeRequestId|seatId' ||
            record.rawFrame !== JSON.stringify(request) ||
            journal.operations.some(row => row.request?.family === 'K-SESSION' &&
              row.request.operation === 'stop' && row.request.domainId === attempt.domainId &&
              row.request.targetId === attempt.sessionId &&
              row.request.payload?.resumeRequestId === attempt.originalCreateRequestId)) {
          throw Error('Candidate stop requires the active case exact original UNKNOWN resume; no resend');
        }
        attempt.candidateStopDispatchState = 'PERSISTED_BEFORE_DISPATCH';
        attempt.candidateStopRequestId = request.requestId;
        attempt.candidateStopRawFrame = record.rawFrame;
        attempt.candidateStopRawSha256 = digest(record.rawFrame);
        record.temporaryHomeCandidateStopRequestId = request.requestId;
        save();
      } else if (request?.family === 'K-INSTANCE' && request.operation === 'temporary-home-close') {
        if (startup.beforeDispatch(record)) return;
        const sessionId = request.payload?.sessionId;
        const originalCreateRequestId = request.payload?.originalCreateRequestId;
        const attempt = attemptFor(originalCreateRequestId);
        if (!attempt || attempt.caseId !== activeCaseId || attempt.sessionId !== sessionId ||
            attempt.instanceId !== request.targetId ||
            attempt.domainId !== request.domainId || request.expectedRevision !== '1' ||
            Object.keys(request.payload ?? {}).sort().join('|') !== 'originalCreateRequestId|sessionId') {
          throw Error('Temporary-HOME close must select this case original create request and exact F1 revision');
        }
        if (attempt.closeDispatchState !== 'NOT_DISPATCHED') {
          throw Error('Original temporary-HOME close was already dispatched; no new requestId is allowed');
        }
        attempt.closeDispatchState = 'PERSISTED_BEFORE_DISPATCH';
        attempt.closeRequestId = request.requestId;
        attempt.closeRawFrame = record.rawFrame;
        attempt.closeRawSha256 = digest(record.rawFrame);
        record.temporaryHomeCloseRequestId = request.requestId;
        save();
      }
    },
    afterReceipt(record) {
      if (startup.afterReceipt(record)) return;
      const request = record.request;
      const attempt = request?.family === 'K-INSTANCE' && request.operation === 'temporary-home-close'
        ? attemptFor(request.payload?.originalCreateRequestId) :
        candidateAttemptFor(request) ?? attemptFor(request?.requestId);
      if (!attempt) return;
      if (candidateAttemptFor(request)) {
        attempt.candidateStopReceipt = record.receipt;
        attempt.candidateStopDispatchState = 'RECEIPT_RECORDED';
      } else if (request.operation === 'temporary-home-close') {
        attempt.closeReceipt = record.receipt;
        attempt.closeDispatchState = 'RECEIPT_RECORDED';
        attempt.teardownState = closeReceipt(record, request.requestId, request.targetId)
          ? 'CLOSE_RECEIPT_CLOSED_SHARED_READBACK_REQUIRED' : 'CLOSE_REFUSED_OR_UNQUALIFIED';
      } else {
        attempt.receipt = record.receipt;
        attempt.dispatchState = 'RECEIPT_RECORDED';
        if (request.operation === 'resume' && nonempty(record.receipt?.result?.newGeneration)) {
          attempt.generation = String(record.receipt.result.newGeneration);
        }
      }
      save();
    },
    afterRejectedReceipt(record) {
      if (startup.afterReceipt(record)) return;
      const request = record.request;
      const attempt = request?.family === 'K-INSTANCE' && request.operation === 'temporary-home-close'
        ? attemptFor(request.payload?.originalCreateRequestId) :
        candidateAttemptFor(request) ?? attemptFor(request?.requestId);
      if (!attempt) return;
      if (candidateAttemptFor(request)) {
        attempt.candidateStopReceipt = record.receipt;
        attempt.candidateStopDispatchState = 'RECEIPT_RECORDED';
      } else if (request.operation === 'temporary-home-close') {
        attempt.closeReceipt = record.receipt;
        attempt.closeDispatchState = 'RECEIPT_RECORDED';
        attempt.teardownState = `CASE_TEARDOWN_INCOMPLETE_CLOSE_${record.receipt?.status ?? 'REJECTED'}`;
      } else {
        attempt.receipt = record.receipt;
        attempt.dispatchState = 'RECEIPT_RECORDED';
      }
      save();
    },
    afterDispatchError(record, error) {
      if (startup.afterError(record, error)) return;
      if (record.request?.family === 'K-INSTANCE' && record.request.operation === 'temporary-home-close') {
        const closeAttempt = attemptFor(record.request.payload?.originalCreateRequestId);
        if (closeAttempt) {
          closeAttempt.closeDispatchState = 'DISPATCH_RESULT_UNKNOWN';
          closeAttempt.closeDispatchError = String(error?.stack ?? error);
          closeAttempt.teardownState = 'CASE_TEARDOWN_INCOMPLETE_CLOSE_FAILED_OR_UNKNOWN';
          save();
        }
        return;
      }
      const candidateAttempt = candidateAttemptFor(record.request);
      if (candidateAttempt) {
        candidateAttempt.candidateStopDispatchState = 'DISPATCH_RESULT_UNKNOWN';
        candidateAttempt.candidateStopError = String(error?.stack ?? error);
        save();
        return;
      }
      const attempt = attemptFor(record.request?.requestId);
      if (!attempt) return;
      attempt.dispatchState = 'DISPATCH_RESULT_UNKNOWN';
      attempt.dispatchError = String(error?.stack ?? error);
      save();
    },
    beforeReconciliation(originalRecord) {
      if (restored) throw Error('Restored lifecycle permits only the explicit original DENIED admission resubmission');
      const request = originalRecord?.request;
      assertSupportedTemporaryHomeOperation(request);
      const attempt = attemptFor(request?.requestId);
      const producer = request && producerFor(request);
      if (!attempt || !nonempty(activeCaseId) || request.operation !== 'resume' ||
          attempt.caseId !== activeCaseId || producer?.caseId !== activeCaseId ||
          producer.instanceId !== attempt.instanceId || request.domainId !== attempt.domainId ||
          request.targetId !== attempt.sessionId ||
          !journal.operations.includes(originalRecord) ||
          originalRecord.rawFrame !== JSON.stringify(request) ||
          originalRecord.rawFrame !== attempt.originalCreateRawFrame ||
          digest(originalRecord.rawFrame) !== attempt.originalCreateRawSha256 ||
          !matchesRequestReply(originalRecord, 'K-SESSION', 'resume', ['UNKNOWN']) ||
          attempt.receipt?.status !== 'UNKNOWN') {
        throw Error('Temporary-HOME reconciliation requires this active case exact original UNKNOWN resume attempt');
      }
    },
    afterReconciliation(originalRecord, observation) {
      const attempt = attemptFor(originalRecord.request?.requestId);
      if (!attempt) throw Error('Temporary-HOME reconciliation lost its original creation attempt');
      attempt.receipt = observation.receipt;
      attempt.dispatchState = 'ORIGINAL_REQUEST_RECONCILED';
      if (originalRecord.request.operation === 'resume' && nonempty(observation.receipt?.result?.newGeneration)) {
        attempt.generation = String(observation.receipt.result.newGeneration);
      }
      save();
    },
    async settleAttempts(attempts) {
      const errors = [];
      const previousCaseId = activeCaseId;
      // Descendants settle before the HOME from which they resumed. UNKNOWN
      // candidates also require the old current claim to remain STOPPED.
      for (const attempt of [...attempts.filter(row => row.operation === 'resume').reverse(),
        ...attempts.filter(row => row.operation !== 'resume')]) {
        activeCaseId = attempt.caseId;
        if (originalClosed(attempt)) continue;
        if (attempt.closeDispatchState !== 'NOT_DISPATCHED') {
          attempt.teardownState = 'CASE_TEARDOWN_INCOMPLETE_CLOSE_ALREADY_DISPATCHED';
          errors.push(`${attempt.originalCreateRequestId}: original close unresolved or mismatched; no replay`);
          save();
          continue;
        }
        if (attempt.operation === 'resume' && !nonempty(attempt.generation) &&
            attempt.receipt?.status === 'UNKNOWN') {
          const session = sessionFor(attempt);
          const producer = manifest.producers.find(row => row.sessionId === attempt.sessionId &&
            row.caseId === attempt.caseId && row.domainId === attempt.domainId &&
            row.instanceId === attempt.instanceId);
          const originals = journal.operations.filter(row =>
            row.request?.requestId === attempt.originalCreateRequestId);
          const original = originals[0];
          if (!session || !producer || originals.length !== 1 ||
              !matchesRequestReply(original, 'K-SESSION', 'resume', ['UNKNOWN']) ||
              original.rawFrame !== attempt.originalCreateRawFrame ||
              digest(original.rawFrame) !== attempt.originalCreateRawSha256 ||
              !isDeepStrictEqual(original.receipt, attempt.receipt) ||
              original.request.domainId !== attempt.domainId ||
              original.request.targetId !== attempt.sessionId ||
              Object.keys(original.request.payload ?? {}).join('|') !== 'generation' ||
              original.request.payload?.generation !== attempt.producerGeneration ||
              original.receipt.previousRevision !== original.request.expectedRevision ||
              !/^(0|[1-9][0-9]*)$/.test(original.receipt.revision ?? '')) {
            attempt.teardownState = 'CASE_TEARDOWN_INCOMPLETE_ORIGINAL_RESUME_SOURCE_CHANGED';
            errors.push(`${attempt.originalCreateRequestId}: original UNKNOWN resume source changed`);
            save();
            continue;
          }
          let candidates = journal.operations.filter(row => row.request?.family === 'K-SESSION' &&
            row.request.operation === 'stop' && row.request.domainId === attempt.domainId &&
            row.request.targetId === attempt.sessionId &&
            row.request.payload?.resumeRequestId === attempt.originalCreateRequestId);
          if (candidates.length === 0) {
            if (attempt.candidateStopDispatchState !== 'NOT_DISPATCHED' ||
                product.socket?.readyState !== WebSocket.OPEN) {
              attempt.teardownState = 'CASE_TEARDOWN_INCOMPLETE_CANDIDATE_STOP_UNRESOLVED';
              errors.push(`${attempt.originalCreateRequestId}: candidate stop intent or host unresolved; no replay`);
              save();
              continue;
            }
            try {
              await product.operation('K-SESSION', 'stop', attempt.sessionId,
                { seatId: producer.seatId, generation: attempt.producerGeneration,
                  resumeRequestId: attempt.originalCreateRequestId },
                original.receipt.revision, ['APPLIED', 'REPLAYED'], attempt.domainId);
            } catch (error) {
              attempt.teardownState = 'CASE_TEARDOWN_INCOMPLETE_CANDIDATE_STOP_FAILED_OR_UNKNOWN';
              attempt.candidateStopError = String(error?.stack ?? error);
              errors.push(`${attempt.originalCreateRequestId}: candidate stop failed or unknown; no replay`);
              save();
              continue;
            }
            candidates = journal.operations.filter(row => row.request?.family === 'K-SESSION' &&
              row.request.operation === 'stop' && row.request.domainId === attempt.domainId &&
              row.request.targetId === attempt.sessionId &&
              row.request.payload?.resumeRequestId === attempt.originalCreateRequestId);
          }
          const candidate = candidates[0];
          const source = candidates.length === 1 && candidateStopProof(candidate, attempt, producer, original);
          if (!source || attempt.candidateStopDispatchState !== 'RECEIPT_RECORDED' ||
              attempt.candidateStopRequestId !== source.requestId ||
              attempt.candidateStopRawFrame !== source.rawFrame ||
              attempt.candidateStopRawSha256 !== source.rawSha256 ||
              !isDeepStrictEqual(attempt.candidateStopReceipt, source.receipt)) {
            attempt.teardownState = 'CASE_TEARDOWN_INCOMPLETE_CANDIDATE_STOP_NOT_PROVEN';
            errors.push(`${attempt.originalCreateRequestId}: independent candidate stop is not proven; no replay`);
            save();
            continue;
          }
          attempt.candidateStopSource = source;
          attempt.candidateStoppedGeneration = source.stoppedGeneration;
          save();
          try {
            await product.operation('K-INSTANCE', 'temporary-home-close', attempt.instanceId,
              { sessionId: attempt.sessionId, originalCreateRequestId: attempt.originalCreateRequestId },
              '1', ['APPLIED', 'REPLAYED'], attempt.domainId);
            if (!originalClosed(attempt)) {
              attempt.teardownState = 'CASE_TEARDOWN_INCOMPLETE_CLOSE_RECEIPT_INVALID';
              errors.push(`${attempt.originalCreateRequestId}: candidate close receipt did not prove CLOSED(2)`);
            }
          } catch (error) {
            attempt.teardownState = attempt.closeReceipt?.status
              ? `CASE_TEARDOWN_INCOMPLETE_CLOSE_${attempt.closeReceipt.status}`
              : 'CASE_TEARDOWN_INCOMPLETE_CLOSE_FAILED_OR_UNKNOWN';
            attempt.teardownError = String(error?.stack ?? error);
            errors.push(`${attempt.originalCreateRequestId}: candidate close failed or unknown`);
          }
          save();
          continue;
        }
        if (!nonempty(attempt.generation)) {
          attempt.teardownState = 'CASE_TEARDOWN_INCOMPLETE_ORIGINAL_RESUME_GENERATION_UNKNOWN';
          errors.push(`${attempt.originalCreateRequestId}: no original native new-generation receipt`);
          save();
          continue;
        }
        const session = sessionFor(attempt);
        const producer = ownedProducerFor(attempt);
        if (!session || !producer || product.socket?.readyState !== WebSocket.OPEN) {
          attempt.teardownState = 'CASE_TEARDOWN_INCOMPLETE_HOST_OR_PRODUCER_UNAVAILABLE';
          errors.push(`${attempt.originalCreateRequestId}: host/producer unavailable`);
          save();
          continue;
        }

        let stops = ordinaryStops(attempt.domainId, attempt.sessionId, attempt.generation);
        if (stops.length === 0) {
          const opened = hEntries(attempt.domainId, attempt.sessionId, 'open', attempt.generation);
          const resumed = journal.operations.filter(row => row.request?.family === 'K-SESSION' &&
            row.request.domainId === attempt.domainId && row.request.targetId === attempt.sessionId &&
            row.request.operation === 'resume' &&
            String(row.receipt?.result?.newGeneration ?? '') === String(attempt.generation));
          const currentGeneration = String(session.generation ?? '');
          const startProven = (opened.length === 1 &&
            matchesRequestReply(opened[0], 'K-SESSION', 'open', ['APPLIED', 'REPLAYED'])) ||
            (resumed.length === 1 &&
              matchesRequestReply(resumed[0], 'K-SESSION', 'resume', ['APPLIED', 'REPLAYED']));
          if (!startProven ||
              currentGeneration !== String(attempt.generation) || !nonempty(session.revision)) {
            attempt.teardownState = 'CASE_TEARDOWN_INCOMPLETE_NO_PROVEN_ORIGINAL_H_STOP';
            errors.push(`${attempt.originalCreateRequestId}: no safe original H stop basis`);
            save();
            continue;
          }
          try {
            const stopped = await product.operation('K-SESSION', 'stop', attempt.sessionId,
              { generation: String(attempt.generation), seatId: producer.seatId },
              session.revision, ['APPLIED', 'REPLAYED'], attempt.domainId);
            session.revision = stopped.revision;
            product.save();
            stops = ordinaryStops(attempt.domainId, attempt.sessionId, attempt.generation);
          } catch (error) {
            attempt.teardownState = 'CASE_TEARDOWN_INCOMPLETE_H_STOP_FAILED_OR_UNKNOWN';
            attempt.teardownError = String(error?.stack ?? error);
            errors.push(`${attempt.originalCreateRequestId}: H stop failed or unknown`);
            save();
            continue;
          }
        }
        if (stops.length !== 1 || !stopProof(stops[0])) {
          attempt.teardownState = 'CASE_TEARDOWN_INCOMPLETE_H_STOP_NOT_PROVEN';
          errors.push(`${attempt.originalCreateRequestId}: existing H stop is not a proven receipt`);
          save();
          continue;
        }
        attempt.hStopSource = {
          requestId: stops[0].request.requestId,
          rawFrame: stops[0].rawFrame,
          rawSha256: digest(stops[0].rawFrame),
          stopFact: stopProof(stops[0]),
        };
        const releaseSource = releaseSourceFor(attempt, producer, stops[0]);
        if (releaseSource === null) {
          attempt.teardownState = 'CASE_TEARDOWN_INCOMPLETE_CURRENT_RELEASE_SOURCE_UNPROVEN';
          errors.push(`${attempt.originalCreateRequestId}: current release revision lacks its own settled resume source`);
          save();
          continue;
        }
        if (!releaseSource.promoted) {
          const releaseRevision = releaseSource.revision;
          attempt.releaseDispatchState ??= 'NOT_DISPATCHED';
          const releases = journal.operations.filter(row =>
            row.request?.family === 'K-SESSION' && row.request.operation === 'admission-release' &&
            row.request.domainId === attempt.domainId && row.request.targetId === attempt.sessionId &&
            String(row.request.payload?.generation ?? '') === String(attempt.generation) &&
            row.request.payload?.seatId === producer.seatId);
          if (attempt.releaseDispatchState !== 'NOT_DISPATCHED' && releases.length === 0) {
            attempt.releaseDispatchState = attempt.releaseDispatchState === 'RECEIPT_RECORDED'
              ? 'RECEIPT_MISSING_OR_AMBIGUOUS' : attempt.releaseDispatchState;
            attempt.teardownState = 'CASE_TEARDOWN_INCOMPLETE_RELEASE_PROVENANCE_MISSING';
            errors.push(`${attempt.originalCreateRequestId}: release history is incomplete; no replay`);
            save();
            continue;
          }
          if (releases.length > 1) {
            attempt.releaseDispatchState = 'DUPLICATE_OR_AMBIGUOUS';
            attempt.teardownState = 'CASE_TEARDOWN_INCOMPLETE_RELEASE_DUPLICATE';
            errors.push(`${attempt.originalCreateRequestId}: more than one matching H release; no replay`);
            save();
            continue;
          }
          const existingRelease = releases[0];
          if (existingRelease) {
            attempt.releaseRequestId = existingRelease.request?.requestId ?? null;
            attempt.releaseRawFrame = existingRelease.rawFrame ?? null;
            attempt.releaseRawSha256 = typeof existingRelease.rawFrame === 'string'
              ? digest(existingRelease.rawFrame) : null;
            attempt.releaseReceipt = existingRelease.receipt ?? null;
            if (!releaseReceipt(existingRelease, attempt, producer, releaseRevision)) {
              attempt.releaseDispatchState = existingRelease.receipt ? 'RECEIPT_RECORDED' : 'DISPATCH_RESULT_UNKNOWN';
              attempt.releaseError = existingRelease.originalError ??
                `Existing H release status=${existingRelease.receipt?.status ?? 'UNKNOWN'}`;
              attempt.teardownState = `CASE_TEARDOWN_INCOMPLETE_RELEASE_${existingRelease.receipt?.status ?? 'UNKNOWN'}`;
              errors.push(`${attempt.originalCreateRequestId}: existing H release is not an applied exact receipt; no replay`);
              save();
              continue;
            }
            attempt.releaseDispatchState = 'RECEIPT_RECORDED';
            attempt.hReleaseSource = {
              requestId: existingRelease.request.requestId,
              rawFrame: existingRelease.rawFrame,
              rawSha256: digest(existingRelease.rawFrame),
              receipt: existingRelease.receipt,
            };
            save();
          } else {
            try {
              const released = await product.operation('K-SESSION', 'admission-release', attempt.sessionId,
                { generation: String(attempt.generation), seatId: producer.seatId },
                releaseRevision, ['APPLIED', 'REPLAYED'], attempt.domainId);
              const release = journal.operations.at(-1);
              if (!releaseReceipt(release, attempt, producer, releaseRevision) || release.receipt !== released) {
                throw Error('New H release receipt did not prove the exact stopped claim was released');
              }
              attempt.releaseDispatchState = 'RECEIPT_RECORDED';
              attempt.releaseRequestId = release.request.requestId;
              attempt.releaseRawFrame = release.rawFrame;
              attempt.releaseRawSha256 = digest(release.rawFrame);
              attempt.releaseReceipt = release.receipt;
              attempt.hReleaseSource = {
                requestId: release.request.requestId,
                rawFrame: release.rawFrame,
                rawSha256: digest(release.rawFrame),
                receipt: release.receipt,
              };
              save();
            } catch (error) {
              const release = journal.operations.at(-1);
              const sameRelease = release?.request?.family === 'K-SESSION' &&
                release.request.operation === 'admission-release' &&
                release.request.domainId === attempt.domainId &&
                release.request.targetId === attempt.sessionId &&
                String(release.request.payload?.generation ?? '') === String(attempt.generation) &&
                release.request.payload?.seatId === producer.seatId;
              if (sameRelease) {
                attempt.releaseDispatchState = release.receipt ? 'RECEIPT_RECORDED' : 'DISPATCH_RESULT_UNKNOWN';
                attempt.releaseRequestId = release.request.requestId;
                attempt.releaseRawFrame = release.rawFrame;
                attempt.releaseRawSha256 = digest(release.rawFrame);
                attempt.releaseReceipt = release.receipt ?? null;
                attempt.releaseError = String(error?.stack ?? error);
              } else {
                attempt.releaseDispatchState = 'DISPATCH_RESULT_UNKNOWN';
                attempt.releaseError = String(error?.stack ?? error);
              }
              attempt.teardownState = `CASE_TEARDOWN_INCOMPLETE_RELEASE_${release?.receipt?.status ?? 'UNKNOWN'}`;
              errors.push(`${attempt.originalCreateRequestId}: H release failed or unknown; no F close`);
              save();
              continue;
            }
          }
        }
        save();
        try {
          await product.operation('K-INSTANCE', 'temporary-home-close', attempt.instanceId,
            { sessionId: attempt.sessionId, originalCreateRequestId: attempt.originalCreateRequestId },
            '1', ['APPLIED', 'REPLAYED'], attempt.domainId);
          const close = journal.operations.at(-1);
          if (!originalClosed(attempt)) {
            attempt.teardownState = 'CASE_TEARDOWN_INCOMPLETE_CLOSE_RECEIPT_INVALID';
            errors.push(`${attempt.originalCreateRequestId}: close receipt did not prove CLOSED(2)`);
          }
        } catch (error) {
          attempt.teardownState = attempt.closeReceipt?.status
            ? `CASE_TEARDOWN_INCOMPLETE_CLOSE_${attempt.closeReceipt.status}`
            : 'CASE_TEARDOWN_INCOMPLETE_CLOSE_FAILED_OR_UNKNOWN';
          attempt.teardownError = String(error?.stack ?? error);
          errors.push(`${attempt.originalCreateRequestId}: close failed or unknown`);
        }
        save();
      }
      manifest.state = errors.length ? 'CASE_TEARDOWN_INCOMPLETE' :
        'CLOSE_RECEIPTS_COMPLETE_SHARED_CLOSED_READBACK_REQUIRED';
      manifest.errors = errors;
      save();
      activeCaseId = previousCaseId;
      journal.temporaryHomeLifecycle = { manifest: path.basename(manifestPath),
        state: manifest.state, attempts: attempts.length,
        closedReceipts: attempts.filter(originalClosed).length,
        independentSharedReadback: 'NOT_VERIFIED' };
      product.save();
      return { complete: errors.length === 0, errors };
    },
    async finishCase(caseId) {
      const caseRow = manifest.cases.find(row => row.caseId === caseId);
      if (!caseRow) throw Error('Temporary-HOME case was not begun or anchored');
      const attempts = manifest.attempts.filter(row => row.caseId === caseId);
      if (attempts.length === 0) {
        caseRow.state = 'NO_HOME_CREATE_DISPATCHED';
        if (activeCaseId === caseId) activeCaseId = null;
        save();
        return { complete: true, state: caseRow.state, errors: [] };
      }
      const settled = await lifecycle.settleAttempts(attempts);
      const complete = settled.complete && attempts.every(originalClosed);
      caseRow.state = complete ? 'CASE_HOME_CLOSE_RECEIPTS_COMPLETE' : 'CASE_TEARDOWN_INCOMPLETE';
      caseRow.errors = settled.errors;
      if (activeCaseId === caseId) activeCaseId = null;
      save();
      return { complete, state: caseRow.state, errors: settled.errors };
    },
    beforeCheckpointClose(caseId) {
      const attempts = manifest.attempts.filter(row => row.caseId === caseId);
      if (activeCaseId !== caseId || startup.hasCreatedHomes() || attempts.length === 0 ||
          attempts.length !== manifest.attempts.length) {
        throw Error('Checkpoint requires the active exclusive case and its original HOME attempts');
      }
      const stops = attempts.map(attempt => {
        const producer = ownedProducerFor(attempt);
        const session = sessionFor(attempt);
        const records = ordinaryStops(attempt.domainId, attempt.sessionId, attempt.generation);
        const stopped = records[0];
        const releases = hEntries(attempt.domainId, attempt.sessionId, 'admission-release', attempt.generation);
        if (!producer || !session || String(session.generation) !== String(attempt.generation) ||
            records.length !== 1 || !stopProof(stopped) || releases.length !== 0 ||
            attempt.closeDispatchState !== 'NOT_DISPATCHED' ||
            stopped.request.payload?.seatId !== producer.seatId ||
            !/^(0|[1-9][0-9]*)$/.test(stopped.receipt.revision) ||
            !/^(0|[1-9][0-9]*)$/.test(stopped.request.expectedRevision) ||
            stopped.receipt.revision !== session.revision ||
            stopped.receipt.previousRevision !== stopped.request.expectedRevision ||
            BigInt(stopped.receipt.revision) !== BigInt(stopped.request.expectedRevision) + 1n) {
          throw Error('Checkpoint requires one original stop-only receipt per current owned generation');
        }
        return { sessionId: attempt.sessionId, generation: String(attempt.generation),
          requestId: stopped.request.requestId, stopFact: stopProof(stopped) };
      });
      // A checkpoint keeps these same stopped H/HOME owners for later resume.
      // Its caller still observes the real caption exit and immutable DB facts.
      return { complete: true, state: 'STOP_ONLY_CHECKPOINT_READBACK_REQUIRED', caseId, stops };
    },
    async beforeCaptionClose() {
      const own = await lifecycle.settleAttempts(manifest.attempts);
      const boot = await startup.settle();
      if (!own.complete || !boot.complete) {
        manifest.state = 'CASE_TEARDOWN_INCOMPLETE'; save();
      }
      return { complete: own.complete && boot.complete, errors: [...own.errors, ...boot.errors] };
    },
    applySharedClosedReadback(facts) {
      if (manifest.attempts.length === 0 && !startup.hasTrackedHomes()) {
        const unchanged = Array.isArray(facts?.temporaryHomes) && facts.temporaryHomes.length === 0 &&
          facts.homeInventoryUnchangedExceptOwnClosed === true;
        manifest.sharedReadback = unchanged ? 'NO_CREATE_HOME_INVENTORY_UNCHANGED' : 'NOT_QUALIFIED';
        manifest.state = unchanged ? 'NO_HOME_CREATE_DISPATCHED' : 'CASE_TEARDOWN_INCOMPLETE';
        save();
        journal.temporaryHomeLifecycle = { ...journal.temporaryHomeLifecycle,
          manifest: path.basename(manifestPath), state: manifest.state,
          independentSharedReadback: manifest.sharedReadback };
        product.save();
        return unchanged;
      }
      const homes = facts?.temporaryHomes;
      const valid = Array.isArray(homes) && manifest.attempts.every(attempt => {
        const receipt = attempt.closeReceipt?.result;
        const rows = homes.filter(row => row.originalCreateRequestId === attempt.originalCreateRequestId &&
          row.caseId === attempt.caseId && row.domainId === attempt.domainId && row.sessionId === attempt.sessionId &&
          row.instanceId === attempt.instanceId && row.state === 'CLOSED' && String(row.revision) === '2' &&
          row.closeRequestId === attempt.closeRequestId && row.nativeReceiptId === receipt?.nativeReceiptId &&
          row.homeId === receipt?.homeId && row.proofKind === receipt?.proofKind &&
          row.sourceFingerprint === receipt?.sourceFingerprint);
        return (attempt.hStopSource || attempt.candidateStopSource) && rows.length === 1 &&
          closeReceipt({ receipt: attempt.closeReceipt },
          attempt.closeRequestId, attempt.instanceId);
      }) && homes.length === manifest.attempts.length &&
        (facts.homeInventoryMatchesDeclaredScope === true ||
          (manifest.authorizedRepairs.length === 0 && facts.homeInventoryUnchangedExceptOwnClosed === true)) &&
        startup.applyFacts(facts);
      manifest.sharedReadback = valid ? (manifest.authorizedRepairs.length
        ? 'EXACT_DECLARED_HOME_OUTCOMES' : manifest.attempts.length || startup.hasCreatedHomes()
          ? 'EXACT_OWN_HOMES_CLOSED' : 'NO_CREATE_HOME_INVENTORY_UNCHANGED') : 'NOT_QUALIFIED';
      manifest.state = valid ? 'CASE_TEARDOWN_COMPLETE_REVIEW_REQUIRED' : 'CASE_TEARDOWN_INCOMPLETE';
      for (const caseRow of manifest.cases) {
        caseRow.state = caseRow.state === 'NO_HOME_CREATE_DISPATCHED'
          ? (valid ? caseRow.state : 'CASE_TEARDOWN_INCOMPLETE')
          : caseRow.state === 'CASE_HOME_CLOSE_REFUSED'
          ? (valid ? 'CASE_HOME_REFUSAL_READBACK_CONFIRMED' : 'CASE_TEARDOWN_INCOMPLETE')
          : (valid ? 'CASE_TEARDOWN_COMPLETE_REVIEW_REQUIRED' : 'CASE_TEARDOWN_INCOMPLETE');
      }
      save();
      journal.temporaryHomeLifecycle = { ...journal.temporaryHomeLifecycle,
        state: manifest.state, independentSharedReadback: manifest.sharedReadback };
      product.save();
      return valid;
    },
    async verifyClosedReadback() {
      let actualClose = null;
      let normalCloseError = null;
      if (restored) {
        try { actualClose = requireRecordedNormalClose(journal, product.endpoint, undefined, config); }
        catch (error) { normalCloseError = String(error?.stack ?? error); actualClose = null; }
      } else {
        // Preserve the existing fresh-case producer contract. Its normal close
        // records a real NodeChild and nativeCloseObservations, not a causal attempt.
        const latestClose = journal.closes?.at(-1);
        actualClose = latestClose && product.endpoint && product.child &&
          latestClose.pid === product.endpoint.pid && latestClose.forceKill === false &&
          latestClose.exitCode === product.child.exitCode && product.child.exitCode !== null;
      }
      if (!baselineReference || !actualClose || manifest.cases.some(row => row.state === 'RUNNING')) {
        manifest.state = 'CASE_TEARDOWN_INCOMPLETE';
        manifest.sharedReadback = 'NOT_RUN_BASELINE_OR_NORMAL_CAPTION_CLOSE_MISSING';
        manifest.sharedReadbackError = normalCloseError;
        save();
        journal.temporaryHomeLifecycle = { manifest: path.basename(manifestPath),
          state: manifest.state, independentSharedReadback: manifest.sharedReadback,
          error: normalCloseError };
        product.save();
        return false;
      }
      try {
        const { report, reference } = await runReadback('verify', baselineReference);
        const valid = lifecycle.applySharedClosedReadback(report.facts);
        journal.temporaryHomeReadback.verify = reference;
        journal.temporaryHomeLifecycle = { ...journal.temporaryHomeLifecycle,
          state: valid ? manifest.state : 'CASE_TEARDOWN_INCOMPLETE',
          independentSharedReadback: valid ? manifest.sharedReadback : 'NOT_QUALIFIED' };
        if (!valid) manifest.state = 'CASE_TEARDOWN_INCOMPLETE';
        save(); product.save();
        return valid;
      } catch (error) {
        manifest.state = 'CASE_TEARDOWN_INCOMPLETE';
        manifest.sharedReadbackError = String(error?.stack ?? error);
        save();
        journal.temporaryHomeLifecycle = { manifest: path.basename(manifestPath),
          state: manifest.state, independentSharedReadback: 'NOT_QUALIFIED',
          error: manifest.sharedReadbackError };
        product.save();
        throw error;
      }
    },
    assertComplete() {
      if (manifest.state === 'NO_HOME_CREATE_DISPATCHED') return;
      if (manifest.state !== 'CASE_TEARDOWN_COMPLETE_REVIEW_REQUIRED') {
        throw Error('CASE_TEARDOWN_INCOMPLETE: own temporary HOME receipts or shared closed readback are missing');
      }
    },
  };
  product.temporaryHomeLifecycle = lifecycle;
  return lifecycle;
}

export const isTemporaryHomeCreateRequest = createHomeFor;
