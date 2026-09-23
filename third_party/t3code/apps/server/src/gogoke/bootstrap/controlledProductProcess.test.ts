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
import { readGitHubFact } from "../context/repository/gitFact.ts";
import { NativeHostClient } from "../persistence/base/nativeHostClient.ts";
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
    const task = await client.prepareR2TestTask(caller);
    Assert.equal(task.disposition, "COMMITTED");
    Assert.equal((await client.prepareR2TestTask(caller)).disposition, "RECONCILED");
    Assert.deepEqual(await client.readTaskContextRequirements({
      domainId: "domain-r2-02-test", taskId: "task-r2-02-test",
    }), {
      domainId: "domain-r2-02-test", taskId: task.taskId, taskRevision: task.taskRevision,
      contentHash: task.contentHash, mandatoryRefs: [],
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
    for (let task = 0; task < 2; task += 1) {
      let session!: PiManagedSession;
      let stopProofHash = "";
      const operationId = `r2-02-${Crypto.randomUUID()}`;
      session = new PiManagedSession({
        admission: { mode: "ordinary", protocolQualified: true,
          protectedDomainQualified: false, contextExposure: "UNKNOWN" },
        sink: { async write(chunk) {
          const promptJson = Buffer.from(chunk).toString("utf8").trimEnd();
          const packageReceipt = await client!.prepareR2TestPackage(caller, promptJson);
          Assert.equal(packageReceipt.state, "TEST_ONLY_PACKAGE_PREPARED_NOT_ACTION");
          if (packageDigest === undefined) {
            Assert.equal(packageReceipt.disposition, "COMMITTED");
            packageDigest = packageReceipt.packageDigest;
          } else {
            Assert.equal(packageReceipt.disposition, "REPLAYED");
            Assert.equal(packageReceipt.packageDigest, packageDigest);
          }
          const lineage = await client!.prepareR2TestLineage(caller);
          Assert.equal(lineage.state, "TEST_ONLY_LINEAGE_PREPARED_NOT_ACTION");
          if (lineageBinding === undefined) {
            Assert.equal(lineage.disposition, "COMMITTED");
            lineageBinding = lineage.bindingId;
          } else {
            Assert.equal(lineage.disposition, "REPLAYED");
            Assert.equal(lineage.bindingId, lineageBinding);
          }
          const recipe = await client!.prepareR2TestRecipe(caller);
          Assert.equal(recipe.state, "TEST_ONLY_RECIPE_PREPARED_NOT_ACTION");
          if (recipeHash === undefined) {
            Assert.equal(recipe.disposition, "COMMITTED");
            recipeHash = recipe.contentHash;
          } else {
            Assert.equal(recipe.disposition, "RECONCILED");
            Assert.equal(recipe.contentHash, recipeHash);
          }
          const evidence = await client!.runControlledFixtureProbe({ caller, operationId, promptJson });
          stopProofHash = evidence.stopProofHash;
          for (const frame of evidence.frames) session.acceptStdout(Buffer.from(frame));
        } },
      });
      const observation = await session.promptAndObserveSettlement(message, 30_000);
      Assert.match(stopProofHash, /^sha256:[0-9a-f]{64}$/);
      proofs.add(stopProofHash);
      const result = validateControlledFixtureResult(source, observation);
      Assert.equal(result.state, "VALIDATED_TEST_RESULT_NOT_ADOPTED");
      Assert.equal(result.sourceBlob, source.gitBlob);
    }
    Assert.equal(proofs.size, 2, "separate operations retain separate native custody");
    await client.close();
    client = undefined;
    const product = await handleProductGoalRequest({
      goal: { id: "goal-r2-02", title: "Controlled public fixture task" },
      ledger: {
        repository: "taiyun668/gogoke",
        commit: "6765d4e11ace61c47b9aeb123e0ef4770ab072c0",
        path: "apps/desktop/test-fixtures/s1-r4/ledger/r2-02-source-reference.json",
        contentHash: "sha256:b57db8a5fec4d9a4a09ca1e356c865017f88473916c5debeefa0ca2d87b08d08",
      },
      runControlledTask: true,
    }, { root: productRoot, hostBinary: hosted });
    Assert.equal(product.controlledTask?.state, "VALIDATED_TEST_RESULT_NOT_ADOPTED");
    Assert.equal(product.acceptance, "TEST_FIXTURE_NOT_ADOPTED");
  } finally {
    await client?.close();
    await FS.rm(root, { recursive: true, force: true });
  }
});
