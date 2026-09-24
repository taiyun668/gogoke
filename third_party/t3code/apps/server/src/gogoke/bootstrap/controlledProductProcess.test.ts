// @effect-diagnostics nodeBuiltinImport:off - Windows cloud process integration.
import * as Assert from "node:assert/strict";
import * as Crypto from "node:crypto";
import * as FS from "node:fs/promises";
import * as OS from "node:os";
import * as Path from "node:path";
import { it } from "node:test";
import { fileURLToPath } from "node:url";

import { PiManagedSession } from "../adapters/pi/session.ts";
import { validateControlledFixtureResult } from "../actions/controlledFixtureResult.ts";
import { prepareR2ControlledManifest, prepareR2PublicContext } from "../context/assembly/r2ControlledManifest.ts";
import { commitR2ControlledDecision } from "../decision/r2ControlledDecision.ts";
import { readGitHubFact } from "../context/repository/gitFact.ts";
import { R2_TEST_LEDGER, type GitFactWritePort } from "../context/repository/gitFactWrite.ts";
import { NativeHostClient, type NativeContextManifestReceipt, type NativeR2ActionDecisionBasis, type NativeR2TestActionPreparation, type NativeR2TestLineageReceipt, type NativeR2TestPackageReceipt, type NativeR2TestRecipeReceipt } from "../persistence/base/nativeHostClient.ts";
import { handleProductGoalRequest } from "./productEntry.ts";

const sourceCoordinate = Object.freeze({
  repository: "taiyun668/gogoke",
  commit: "f6a820dda05a3eac5c29be48c4149bff7e1c9598",
  path: "apps/desktop/test-fixtures/s1-r4/sealing/model-asset.json",
  contentHash: "sha256:268f5e2c65e254e7dd55e6e8dfc8eabfa23f7a14a9cfd1be8a998f1297cefa8e",
});
const sha256 = (bytes: Uint8Array) => Crypto.createHash("sha256").update(bytes).digest("hex");
const fixturePath = fileURLToPath(new URL("../../../../../../../apps/desktop/test-fixtures/s1-r4/ledger/controlled-pi.mjs", import.meta.url));

const cloudOnly = process.platform === "win32" && Boolean(process.env.GOGOKE_NATIVE_HOST) ? it : it.skip;

cloudOnly("runs the fixed public fixture through native custody and Pi protocol without adoption", { timeout: 40_000 }, async () => {
  const builtHost = process.env.GOGOKE_NATIVE_HOST;
  if (builtHost === undefined) throw new Error("GOGOKE_NATIVE_HOST missing");
  const source = await readGitHubFact(
    sourceCoordinate, sourceCoordinate.repository, fetch, process.env.GH_TOKEN);
  const root = await FS.mkdtemp(Path.join(OS.tmpdir(), "gogoke-r2-controlled-"));
  const resourceDir = Path.join(root, "resources");
  const productRoot = Path.join(root, "product-root");
  const hosted = Path.join(resourceDir, "gogoke-native-host.exe");
  const runtime = Path.join(resourceDir, "gogoke-service", "runtime", "node.exe");
  const script = Path.join(resourceDir, "gogoke-service", "fixtures", "controlled-pi.mjs");
  let client: NativeHostClient | undefined;
  try {
    await FS.mkdir(Path.dirname(runtime), { recursive: true });
    await FS.mkdir(Path.dirname(script), { recursive: true });
    await FS.mkdir(productRoot, { recursive: true });
    await FS.copyFile(builtHost, hosted);
    await FS.copyFile(process.execPath, runtime);
    await FS.copyFile(fixturePath, script);
    const previousNodeOptions = process.env.NODE_OPTIONS;
    process.env.NODE_OPTIONS = "--import=data:text/javascript,process.exit(42)";
    try {
      client = await NativeHostClient.attach({ root: productRoot, hostBinary: hosted });
    } finally {
      if (previousNodeOptions === undefined) delete process.env.NODE_OPTIONS;
      else process.env.NODE_OPTIONS = previousNodeOptions;
    }
    const identity = await client.readProductIdentity();
    const caller = {
      policyRevision: identity.policyRevision,
      principalId: identity.principalId,
      profileId: identity.profileId,
      revocationHead: identity.revocationHead,
      role: "controller" as const,
      seatId: identity.seatId,
    };
    const preparedGrant = await client.prepareR2TestDelegation(caller);
    Assert.equal(preparedGrant.state, "TEST_ONLY_GRANT_PREPARED_NOT_ACTION");
    Assert.deepEqual(await client.prepareR2TestDelegation(caller), preparedGrant,
      "a repeated fixed test operation must return the same grant");
    const grant = await client.readCurrentDelegationGrant(preparedGrant.grantRef);
    Assert.deepEqual(grant.ceiling.allowedActions, ["delegate"]);
    Assert.deepEqual(grant.ceiling.allowedTargetPrincipalIds, ["principal-r2-02-worker"]);
    Assert.deepEqual(grant.ceiling.explicitPrivateMaterialIds, []);
    const contextGrant = await client.prepareR2TestContextGrant(caller);
    Assert.equal(contextGrant.state, "TEST_ONLY_CONTEXT_GRANT_PREPARED");
    Assert.deepEqual(await client.prepareR2TestContextGrant(caller), contextGrant);
    await prepareR2PublicContext({ store: client, source, grant: contextGrant,
      policyRevision: identity.policyRevision });
    const task = await client.prepareR2TestTask(caller);
    Assert.equal(task.disposition, "COMMITTED");
    Assert.equal((await client.prepareR2TestTask(caller)).disposition, "RECONCILED");
    Assert.deepEqual(await client.readTaskContextRequirements({
      domainId: "domain-r2-02-test", taskId: "task-r2-02-test",
    }), {
      domainId: "domain-r2-02-test", taskId: task.taskId, taskRevision: task.taskRevision,
      contentHash: task.contentHash, mandatoryRefs: [{
        sourceDomainId: "domain-r2-02-source", contextId: "context-r2-02-public-fixture",
        version: "1",
      }],
    });
    const message = JSON.stringify({
      schema: "gogoke.s1-r4.r2-02.fixture-task.v1", testOnly: true,
      source: { repository: sourceCoordinate.repository, commit: sourceCoordinate.commit,
        path: sourceCoordinate.path, sha256: sha256(source.bytes),
        content: Buffer.from(source.bytes).toString("utf8") },
    });
    const proofs = new Set<string>();
    let packageDigest: string | undefined;
    let lineageBinding: string | undefined;
    let recipeHash: string | undefined;
    let actionDigest: string | undefined;
    let decisionReceiptId: string | undefined;
    let manifestHash: string | undefined;
    let nativeActionCompletionRef: string | undefined;
    const decisionRecordedAt = new Date().toISOString();
    const promptJson = JSON.stringify({ type: "prompt", message, id: "gogoke-pi-1" });
    for (let task = 0; task < 2; task += 1) {
      let session!: PiManagedSession;
      let stopProofHash = "";
      const operationId = `r2-02-${Crypto.randomUUID()}`;
      const packageReceipt: NativeR2TestPackageReceipt = await client.prepareR2TestPackage(caller, promptJson);
      Assert.equal(packageReceipt.state, "TEST_ONLY_PACKAGE_PREPARED_NOT_ACTION");
      if (packageDigest === undefined) {
        Assert.equal(packageReceipt.disposition, "COMMITTED");
        packageDigest = packageReceipt.packageDigest;
      } else {
        Assert.equal(packageReceipt.disposition, "REPLAYED");
        Assert.equal(packageReceipt.packageDigest, packageDigest);
      }
      const lineage: NativeR2TestLineageReceipt = await client.prepareR2TestLineage(caller);
      Assert.equal(lineage.state, "TEST_ONLY_LINEAGE_PREPARED_NOT_ACTION");
      if (lineageBinding === undefined) {
        Assert.equal(lineage.disposition, "COMMITTED");
        lineageBinding = lineage.bindingId;
      } else {
        Assert.equal(lineage.disposition, "REPLAYED");
        Assert.equal(lineage.bindingId, lineageBinding);
      }
      const recipe: NativeR2TestRecipeReceipt = await client.prepareR2TestRecipe(caller);
      Assert.equal(recipe.state, "TEST_ONLY_RECIPE_PREPARED_NOT_ACTION");
      if (recipeHash === undefined) {
        Assert.equal(recipe.disposition, "COMMITTED");
        recipeHash = recipe.contentHash;
      } else {
        Assert.equal(recipe.disposition, "RECONCILED");
        Assert.equal(recipe.contentHash, recipeHash);
      }
      const basis: NativeR2ActionDecisionBasis = await client.readR2TestActionDecisionBasis(caller, promptJson);
      Assert.equal(basis.state, "TEST_ONLY_DECISION_BASIS_NOT_ACTION");
      Assert.match(basis.actionDigest, /^sha256:[0-9a-f]{64}$/);
      if (actionDigest === undefined) actionDigest = basis.actionDigest;
      else Assert.equal(basis.actionDigest, actionDigest);
      const decision = await commitR2ControlledDecision({
        store: client, basis, grant: preparedGrant, recordedAt: decisionRecordedAt,
      });
      if (decisionReceiptId === undefined) {
        Assert.equal(decision.kind, "committed");
        if (decision.kind !== "committed") throw new Error("Decision did not commit");
        decisionReceiptId = decision.decisionReceiptId;
      } else {
        Assert.equal(decision.kind, "replayed");
        if (decision.kind !== "replayed") throw new Error("Decision did not replay");
        Assert.equal(decision.decisionReceiptId, decisionReceiptId);
      }
      const preparedAction: NativeR2TestActionPreparation = await client.prepareR2TestAction({
        grantRef: preparedGrant.grantRef,
        promptJson,
        expectedActionDigest: basis.actionDigest,
        expectedPackageDigest: packageReceipt.packageDigest,
      });
      Assert.equal(preparedAction.kind, task === 0 ? "reserved" : "replay");
      Assert.equal(preparedAction.reservationState, task === 0 ? "reserved" : "completed");
      if (task === 0) {
        const manifest = await prepareR2ControlledManifest({
          store: client, basis, grant: preparedGrant, contextGrant, source,
          recordedAt: decisionRecordedAt,
        });
        Assert.equal(manifest.manifestId, "manifest-r2-02-test");
        Assert.equal(manifest.includedVersions.length, 1);
        manifestHash = manifest.manifestHash;
      } else {
        const replay: NativeContextManifestReceipt = await client.readContextManifest({
          operationId: "r2-02-context-assembly", principalId: "principal-r2-02-worker",
          seatId: "seat-r2-02-worker", taskId: "task-r2-02-test",
          sessionId: "session-r2-02-worker", domainId: "domain-r2-02-test",
          bindingId: basis.bindingId, bindingGeneration: basis.bindingGeneration,
          sourceEpoch: "1", runtimeInstanceId: "runtime-r2-02-fixture",
        });
        Assert.equal(replay.disposition, "REPLAYED");
        Assert.equal(replay.manifestHash, manifestHash);
      }
      session = new PiManagedSession({
        admission: { mode: "ordinary", protocolQualified: true,
          protectedDomainQualified: false, contextExposure: "UNKNOWN" },
        sink: { async write(chunk) {
          Assert.equal(Buffer.from(chunk).toString("utf8"), `${promptJson}\n`);
          if (task === 0) {
            const evidence = await client!.runControlledFixtureAction({
              caller, domainId: "domain-r2-02-test",
              operationId: "opr_22222222222222222222222222222222",
              reservationId: "reservation-r2-02-controlled", promptJson,
            });
            Assert.equal(evidence.state, "ACTION_TRANSPORT_COMPLETED_NOT_RESULT");
            if (evidence.state !== "ACTION_TRANSPORT_COMPLETED_NOT_RESULT") {
              throw new Error("Action did not produce fresh transport evidence");
            }
            nativeActionCompletionRef = evidence.actionCompletionRef;
            stopProofHash = evidence.stopProofHash;
            for (const frame of evidence.frames) session.acceptStdout(Buffer.from(frame));
          } else {
            const evidence = await client!.runControlledFixtureProbe({ caller, operationId, promptJson });
            stopProofHash = evidence.stopProofHash;
            for (const frame of evidence.frames) session.acceptStdout(Buffer.from(frame));
          }
        } },
      });
      const observation = await session.promptAndObserveSettlement(message, 30_000);
      Assert.match(stopProofHash, /^sha256:[0-9a-f]{64}$/);
      proofs.add(stopProofHash);
      const result = validateControlledFixtureResult(source, observation);
      Assert.equal(result.state, "VALIDATED_TEST_RESULT_NOT_ADOPTED");
      Assert.equal(result.sourceBlob, source.gitBlob);
    }
    Assert.match(nativeActionCompletionRef ?? "", /^[A-Za-z0-9][A-Za-z0-9._:/-]+$/);
    const objectiveRefs = await client.readR2ObjectiveFactRefs(
      caller, nativeActionCompletionRef!,
    );
    Assert.equal(objectiveRefs.state, "TEST_ONLY_NATIVE_OBJECTIVE_REFS");
    Assert.equal(objectiveRefs.manifestHash, manifestHash);
    Assert.match(objectiveRefs.manifestContentHash, /^sha256:[0-9a-f]{64}$/);
    Assert.match(objectiveRefs.decisionContentHash, /^sha256:[0-9a-f]{64}$/);
    Assert.match(objectiveRefs.actionCompletionHash, /^sha256:[0-9a-f]{64}$/);
    const observationEnd = new Date(Math.max(
      Date.now(), Date.parse(objectiveRefs.actionCompletedAt) + 1,
    )).toISOString();
    const outcome = await client.appendObjectiveOutcome({
      domainId: "domain-r2-02-test",
      outcomeId: "outcome-r2-02-test",
      revision: "1",
      expectedPreviousRevision: null,
      expectedPreviousContentHash: null,
      operationId: "outcome-op-r2-02-test",
      eventId: "outcome-event-r2-02-test",
      receiptId: "outcome-receipt-r2-02-test",
      recordedAt: observationEnd,
      manifestId: "manifest-r2-02-test",
      manifestVersion: "1",
      manifestHash: objectiveRefs.manifestHash,
      decisionId: "decision-r2-02-test",
      decisionVersion: "1",
      decisionHash: objectiveRefs.decisionContentHash,
      actionOperationId: "opr_22222222222222222222222222222222",
      actionCompletionRef: nativeActionCompletionRef!,
      resultRefs: [{ objectType: "ActionCompletion", objectId: "opr_22222222222222222222222222222222", revision: "1", contentHash: objectiveRefs.actionCompletionHash }],
      evidenceRefs: [{ objectType: "ContextManifest", objectId: "manifest-r2-02-test", revision: "1", contentHash: objectiveRefs.manifestContentHash }],
      observationStartsAt: objectiveRefs.actionCompletedAt,
      observationEndsAt: observationEnd,
      observationStatus: "OBSERVED",
    });
    Assert.equal(outcome.disposition, "COMMITTED");
    const outcomeReadback = await client.readObjectiveOutcome(
      "domain-r2-02-test", "outcome-r2-02-test", "1",
    );
    Assert.equal(outcomeReadback.contentHash, outcome.contentHash);
    Assert.equal(proofs.size, 2, "separate operations retain separate native custody");
    await client.close();
    client = undefined;
    const entryRoot = Path.join(root, "product-entry-root");
    await FS.mkdir(entryRoot);
    const productRequest = {
      goal: { id: "goal-r2-02", title: "Controlled public fixture task" },
      ledger: {
        repository: "taiyun668/gogoke",
        commit: "6765d4e11ace61c47b9aeb123e0ef4770ab072c0",
        path: "apps/desktop/test-fixtures/s1-r4/ledger/r2-02-source-reference.json",
        contentHash: "sha256:b57db8a5fec4d9a4a09ca1e356c865017f88473916c5debeefa0ca2d87b08d08",
      },
      runControlledTask: true as const,
    };
    const product = await handleProductGoalRequest(productRequest, { root: entryRoot, hostBinary: hosted });
    Assert.equal(product.controlledTask?.state, "VALIDATED_TEST_RESULT_NOT_ADOPTED");
    Assert.match(product.controlledTask?.actionCompletionRef ?? "", /^[A-Za-z0-9][A-Za-z0-9._:/-]+$/);
    Assert.match(product.controlledTask?.manifestHash ?? "", /^sha256:[0-9a-f]{64}$/);
    Assert.match(product.controlledTask?.objectiveOutcomeContentHash ?? "", /^sha256:[0-9a-f]{64}$/);
    Assert.equal(product.controlledTask?.objectiveOutcomeReceiptId, "outcome-receipt-r2-02-test");
    Assert.match(product.controlledTask?.evaluationContentHash ?? "", /^sha256:[0-9a-f]{64}$/);
    Assert.equal(product.controlledTask?.evaluationReceiptId, "evaluation-receipt-r2-02-test");
    Assert.match(product.controlledTask?.metricsHash ?? "", /^sha256:[0-9a-f]{64}$/);
    Assert.match(product.controlledTask?.dreamRunContentHash ?? "", /^sha256:[0-9a-f]{64}$/);
    Assert.match(product.controlledTask?.dreamProposalContentHash ?? "", /^sha256:[0-9a-f]{64}$/);
    Assert.equal(product.controlledTask?.dreamProposalState, "DRAFT_TEST_ONLY_NOT_ACTIVATED");
    Assert.equal(product.acceptance, "TEST_FIXTURE_NOT_ADOPTED");
    await Assert.rejects(
      handleProductGoalRequest(productRequest, { root: entryRoot, hostBinary: hosted }),
      /R2_ACTION_REPLAY_NO_NEW_RESULT/,
      "a repeated product request may read completion but cannot resend or invent a new Result",
    );
    const writeRoot = Path.join(root, "product-entry-write-root");
    await FS.mkdir(writeRoot);
    let writtenBytes: Buffer | undefined;
    let writtenPath: string | undefined;
    let testHead = "a".repeat(40);
    const testCommit = "d".repeat(40);
    const testTree = "b".repeat(40);
    const writtenBlob = () => {
      if (writtenBytes === undefined) throw new Error("test draft bytes missing");
      return Crypto.createHash("sha1").update(`blob ${writtenBytes.length}\0`)
        .update(writtenBytes).digest("hex");
    };
    const testWritePort: GitFactWritePort = {
      repository: R2_TEST_LEDGER.repository,
      branch: R2_TEST_LEDGER.branch,
      async assertCurrentAuthority() { Assert.equal(testHead.length, 40); },
      async readHead() { return { commit: testHead, tree: testTree }; },
      async readPath() { return null; },
      async createBlob(bytes) { writtenBytes = Buffer.from(bytes); return writtenBlob(); },
      async createTree(_base, path, blob) {
        Assert.equal(blob, writtenBlob());
        writtenPath = path;
        return "c".repeat(40);
      },
      async createCommit(parent) { Assert.equal(parent, testHead); return testCommit; },
      async updateRef(commit) { Assert.equal(commit, testCommit); testHead = commit; },
    };
    const testFetcher: typeof fetch = async () => {
      if (writtenBytes === undefined || writtenPath === undefined) throw new Error("test draft not written");
      return new Response(JSON.stringify({
        type: "file", path: writtenPath, sha: writtenBlob(), size: writtenBytes.length,
        encoding: "base64", content: writtenBytes.toString("base64"),
      }), { status: 200 });
    };
    const runningEntry = process.argv[1];
    if (runningEntry === undefined) throw new Error("node test entry missing");
    const entryHash = `sha256:${sha256(await FS.readFile(runningEntry))}`;
    const writtenProduct = await handleProductGoalRequest(
      { ...productRequest, publishTestDraft: true },
      { root: writeRoot, hostBinary: hosted,
        executionEvidenceSha: process.env.GITHUB_SHA ?? "1".repeat(40),
        serviceEntrySha256: entryHash, testWritePort, testFetcher },
    );
    Assert.equal(writtenProduct.testLedgerDraft?.state, "DRAFT_COMMITTED_NOT_ADOPTED");
    Assert.equal(writtenProduct.testLedgerDraft?.commit, testCommit);
    Assert.equal(writtenProduct.testLedgerDraft?.gitBlob, writtenBlob());
    Assert.equal(writtenProduct.acceptance, "TEST_FIXTURE_NOT_ADOPTED");
    const writtenFact = JSON.parse(writtenBytes?.toString("utf8") ?? "null") as Record<string, unknown>;
    Assert.equal(writtenFact.testOnly, true);
    Assert.equal(writtenFact.serviceEntrySha256, entryHash);
    Assert.equal(writtenFact.dreamProposalState, "DRAFT_TEST_ONLY_NOT_ACTIVATED");
    const evidenceRoot = process.env.GOGOKE_SERVER_EVIDENCE_ROOT;
    if (evidenceRoot !== undefined) {
      const executionEvidenceSha = process.env.GITHUB_SHA;
      const runId = process.env.GITHUB_RUN_ID;
      const runAttempt = process.env.GITHUB_RUN_ATTEMPT;
      if (!/^[0-9a-f]{40}$/u.test(executionEvidenceSha ?? "") ||
          !/^[1-9][0-9]*$/u.test(runId ?? "") ||
          !/^[1-9][0-9]*$/u.test(runAttempt ?? "") ||
          product.controlledTask === undefined) {
        throw new Error("R2_PRODUCT_MACHINE_IDENTITY_MISSING");
      }
      const fact = {
        schema: "gogoke.s1-r4.r2-02.product-test-result.v1",
        testOnly: true,
        operationId: `r2-02-result-${runId}-${runAttempt}`,
        executionEvidenceSha,
        cloudRunId: runId,
        runAttempt,
        sourceCommit: product.controlledTask.sourceCommit,
        sourceBlob: product.controlledTask.sourceBlob,
        reportSha256: product.controlledTask.reportSha256,
        modelId: product.controlledTask.modelId,
        relativePath: product.controlledTask.relativePath,
        embeddedBytesSha256: product.controlledTask.embeddedBytesSha256,
        actionCompletionRef: product.controlledTask.actionCompletionRef,
        manifestHash: product.controlledTask.manifestHash,
        decisionReceiptId: product.controlledTask.decisionReceiptId,
        objectiveOutcomeContentHash: product.controlledTask.objectiveOutcomeContentHash,
        objectiveOutcomeReceiptId: product.controlledTask.objectiveOutcomeReceiptId,
        evaluationContentHash: product.controlledTask.evaluationContentHash,
        evaluationReceiptId: product.controlledTask.evaluationReceiptId,
        metricsHash: product.controlledTask.metricsHash,
        dreamRunContentHash: product.controlledTask.dreamRunContentHash,
        dreamProposalContentHash: product.controlledTask.dreamProposalContentHash,
        dreamProposalState: product.controlledTask.dreamProposalState,
        acceptance: product.acceptance,
      };
      await FS.writeFile(Path.join(evidenceRoot, "product-r2-02-test-result.json"),
        `${JSON.stringify(fact)}\n`, "utf8");
    }
  } finally {
    await client?.close();
    await FS.rm(root, { recursive: true, force: true });
  }
});
