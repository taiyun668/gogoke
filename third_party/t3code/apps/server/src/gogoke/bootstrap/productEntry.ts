// @effect-diagnostics nodeBuiltinImport:off - executable product boundary owns stdin/stdout.
import * as NodeFS from "node:fs";
import { createHash } from "node:crypto";

import { constructGogokeService } from "./index.ts";
import { parseStrictJsonBytes } from "../contracts/strictJson.ts";
import { readGitHubFact } from "../context/repository/gitFact.ts";
import { createR2GhCredentialAccess, currentGhToken } from "../context/repository/ghCredential.ts";
import { R2_TEST_LEDGER, writeR2TestFact } from "../context/repository/gitFactWrite.ts";
import type { GitFactWritePort } from "../context/repository/gitFactWrite.ts";
import { createR2TestGitHubWritePort } from "../context/repository/gitFactWriteHttp.ts";
import { runNovelDriverConformance } from "../adapters/conformance/novel.ts";
import { runR2ControlledProductTask } from "./r2ControlledProductTask.ts";

const R2_02_TEST_LEDGER_REPOSITORY = "taiyun668/gogoke";
const R2_02_SOURCE = Object.freeze({
  repository: "taiyun668/gogoke",
  commit: "f6a820dda05a3eac5c29be48c4149bff7e1c9598",
  path: "apps/desktop/test-fixtures/s1-r4/sealing/model-asset.json",
  contentHash: "sha256:268f5e2c65e254e7dd55e6e8dfc8eabfa23f7a14a9cfd1be8a998f1297cefa8e",
});

export interface ProductLedgerReference {
  readonly repository: string;
  readonly commit: string;
  readonly path: string;
  readonly contentHash: string;
}

export interface ProductGoalRequest {
  readonly goal: { readonly id: string; readonly title: string };
  readonly ledger: ProductLedgerReference;
  readonly runControlledTask?: true;
  readonly publishTestDraft?: true;
  readonly fixtureDriverId?: string;
}

export interface ProductGoalView extends ProductGoalRequest {
  readonly caller: {
    readonly policyRevision: string;
    readonly principalId: string;
    readonly profileId: string;
    readonly admitted: true;
    readonly revocationHead: string;
    readonly role: "controller";
    readonly seatId: string;
  };
  readonly nativeHost: { readonly reachable: true; readonly elapsedMicros: number };
  readonly ledgerReadback: {
    readonly state: "COMMITTED_BYTES_VERIFIED_NOT_ADOPTED";
    readonly gitBlob: string;
  };
  readonly controlledTask?: {
    readonly state: "VALIDATED_TEST_RESULT_NOT_ADOPTED";
    readonly sourceCommit: string;
    readonly sourceBlob: string;
    readonly reportSha256: string;
    readonly modelId: string;
    readonly relativePath: string;
    readonly embeddedBytesSha256: string;
    readonly actionCompletionRef: string;
    readonly manifestHash: string;
    readonly decisionReceiptId: string;
    readonly objectiveOutcomeContentHash: string;
    readonly objectiveOutcomeReceiptId: string;
    readonly evaluationContentHash: string;
    readonly evaluationReceiptId: string;
    readonly metricsHash: string;
    readonly dreamRunContentHash: string;
    readonly dreamProposalContentHash: string;
    readonly dreamProposalState: "DRAFT_TEST_ONLY_NOT_ACTIVATED";
    readonly fixtureDriverBinding?: {
      readonly driverId: string;
      readonly adapterVersion: "1.0.0";
      readonly runtimeInstanceId: string;
      readonly launchDigestSha256: string;
    };
  };
  readonly testLedgerDraft?: {
    readonly state: "DRAFT_COMMITTED_NOT_ADOPTED";
    readonly repository: "taiyun668/gogoke";
    readonly branch: "s1-r4-ledger-test/r2-02";
    readonly commit: string;
    readonly path: string;
    readonly gitBlob: string;
    readonly contentHash: string;
  };
  readonly acceptance: "TEST_FIXTURE_NOT_ADOPTED";
}

const invalid = (path: string, detail: string): never => {
  throw new Error(`INVALID_PRODUCT_ENTRY: ${path} ${detail}`);
};

const exactRecord = (
  value: unknown,
  path: string,
  keys: readonly string[],
): Record<string, unknown> => {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return invalid(path, "must be an object");
  }
  const record = value as Record<string, unknown>;
  const own = Reflect.ownKeys(record);
  if (
    own.length !== keys.length ||
    own.some((key) => typeof key !== "string" || !keys.includes(key)) ||
    keys.some((key) => !Object.hasOwn(record, key))
  ) {
    return invalid(path, "fields are invalid");
  }
  return record;
};

const text = (value: unknown, path: string): string => {
  if (
    typeof value !== "string" ||
    value.length === 0 ||
    value !== value.trim() ||
    value.length > 512
  ) {
    return invalid(path, "must be canonical text");
  }
  return value;
};

export function decodeProductGoalRequest(bytes: Uint8Array): ProductGoalRequest {
  const parsed = parseStrictJsonBytes(bytes);
  const hasTask = typeof parsed === "object" && parsed !== null && !Array.isArray(parsed) &&
    Object.hasOwn(parsed, "runControlledTask");
  const hasDraft = typeof parsed === "object" && parsed !== null && !Array.isArray(parsed) &&
    Object.hasOwn(parsed, "publishTestDraft");
  const hasDriver = typeof parsed === "object" && parsed !== null && !Array.isArray(parsed) &&
    Object.hasOwn(parsed, "fixtureDriverId");
  const root = exactRecord(parsed, "request", ["goal", "ledger",
    ...(hasTask ? ["runControlledTask"] : []), ...(hasDraft ? ["publishTestDraft"] : []),
    ...(hasDriver ? ["fixtureDriverId"] : [])]);
  if (hasTask && root.runControlledTask !== true) {
    return invalid("request.runControlledTask", "must be true when present");
  }
  if (hasDraft && (!hasTask || root.publishTestDraft !== true)) {
    return invalid("request.publishTestDraft", "requires the controlled task and true");
  }
  if (hasDriver && (!hasDraft || typeof root.fixtureDriverId !== "string" ||
      !/^mock_novel_[0-9a-f]{16}$/u.test(root.fixtureDriverId))) {
    return invalid("request.fixtureDriverId", "requires a test draft and a post-build novel ID");
  }
  const goal = exactRecord(root.goal, "request.goal", ["id", "title"]);
  const ledger = exactRecord(root.ledger, "request.ledger", [
    "repository",
    "commit",
    "path",
    "contentHash",
  ]);
  const repository = text(ledger.repository, "request.ledger.repository");
  const commit = text(ledger.commit, "request.ledger.commit");
  const path = text(ledger.path, "request.ledger.path");
  const contentHash = text(ledger.contentHash, "request.ledger.contentHash");
  if (!/^[0-9a-f]{40}(?:[0-9a-f]{24})?$/.test(commit)) {
    return invalid("request.ledger.commit", "must be an immutable Git object id");
  }
  if (
    path.startsWith("/") ||
    path.includes("\\") ||
    path.split("/").some((part) => part === "" || part === "." || part === "..")
  ) {
    return invalid("request.ledger.path", "must be a canonical repository-relative path");
  }
  if (!/^(?:sha256:)?[0-9a-f]{64}$/.test(contentHash)) {
    return invalid("request.ledger.contentHash", "must be a SHA-256 content hash");
  }
  return Object.freeze({
    goal: Object.freeze({
      id: text(goal.id, "request.goal.id"),
      title: text(goal.title, "request.goal.title"),
    }),
    ledger: Object.freeze({ repository, commit, path, contentHash }),
    ...(hasTask ? { runControlledTask: true as const } : {}),
    ...(hasDraft ? { publishTestDraft: true as const } : {}),
    ...(hasDriver ? { fixtureDriverId: root.fixtureDriverId as string } : {}),
  });
}

export function parseProductProcessArgs(
  argv: readonly string[],
): { readonly root: string; readonly hostBinary: string } {
  if (argv.length !== 4 || argv[0] !== "--root" || argv[2] !== "--native-host") {
    return invalid("argv", "expected exactly --root <path> --native-host <path>");
  }
  return Object.freeze({
    root: text(argv[1], "argv.root"),
    hostBinary: text(argv[3], "argv.nativeHost"),
  });
}

export async function handleProductGoalRequest(
  request: ProductGoalRequest,
  paths: { readonly root: string; readonly hostBinary: string;
    readonly executionEvidenceSha?: string; readonly serviceEntrySha256?: string;
    readonly testWritePort?: GitFactWritePort; readonly testFetcher?: typeof fetch },
): Promise<ProductGoalView> {
  const executionEvidenceSha = paths.executionEvidenceSha ?? process.env.GOGOKE_EXECUTION_EVIDENCE_SHA;
  const serviceEntrySha256 = paths.serviceEntrySha256 ?? process.env.GOGOKE_SERVICE_ENTRY_SHA256;
  if (request.publishTestDraft === true &&
      (!/^[0-9a-f]{40}$/u.test(executionEvidenceSha ?? "") ||
       !/^sha256:[0-9a-f]{64}$/u.test(serviceEntrySha256 ?? ""))) {
    throw new Error("R2_TEST_DRAFT_BUILD_IDENTITY_UNAVAILABLE");
  }
  if (request.publishTestDraft === true) {
    const runningEntry = process.argv[1];
    if (runningEntry === undefined ||
        `sha256:${createHash("sha256").update(NodeFS.readFileSync(runningEntry))
          .digest("hex")}` !== serviceEntrySha256) {
      throw new Error("R2_TEST_DRAFT_LOADED_SERVICE_MISMATCH");
    }
  }
  const novel = request.fixtureDriverId === undefined ? undefined : runNovelDriverConformance(
    serviceEntrySha256!, () => Buffer.from(request.fixtureDriverId!.slice("mock_novel_".length), "hex"),
  );
  if (novel !== undefined && novel.driverId !== request.fixtureDriverId) {
    throw new Error("R2_NOVEL_FIXTURE_DRIVER_IDENTITY_MISMATCH");
  }
  const service = await constructGogokeService({
    request: {
      authority: "public",
      requestedCapabilities: ["local-non-model"],
      enabledRuntimeDriverIds: [],
    },
    root: paths.root,
    hostBinary: paths.hostBinary,
  });
  try {
    if (typeof service.store.admitControllerCaller !== "function") {
      throw new Error("PRODUCT_CALLER_ADMISSION_UNAVAILABLE");
    }
    const admission = await service.store.admitControllerCaller({
      policyRevision: service.identity.policyRevision,
      principalId: service.identity.principalId,
      profileId: service.identity.profileId,
      revocationHead: service.identity.revocationHead,
      role: "controller",
      seatId: service.identity.seatId,
    });
    // This construction Goal is test-only. The repository scope comes from
    // Owner's R2-02 authorization, not from the caller's ledger field.
    const gitReadToken = currentGhToken();
    const ledgerReadback = await readGitHubFact(
      request.ledger, R2_02_TEST_LEDGER_REPOSITORY, fetch, gitReadToken);
    let controlledTask: ProductGoalView["controlledTask"];
    let testLedgerDraft: ProductGoalView["testLedgerDraft"];
    let writePort: GitFactWritePort | undefined;
    let draftToken: string | undefined;
    if (request.publishTestDraft === true) {
      const currentNativeAdmission = async () => {
        const current = await service.store.admitControllerCaller!({
          policyRevision: service.identity.policyRevision,
          principalId: service.identity.principalId,
          profileId: service.identity.profileId,
          revocationHead: service.identity.revocationHead,
          role: "controller",
          seatId: service.identity.seatId,
        });
        if (current.admitted !== true || current.principalId !== service.identity.principalId ||
            current.seatId !== service.identity.seatId ||
            current.policyRevision !== service.identity.policyRevision ||
            current.revocationHead !== service.identity.revocationHead) {
          throw new Error("R2_TEST_DRAFT_NATIVE_ADMISSION_CHANGED");
        }
      };
      if (paths.testWritePort === undefined) {
        const access = createR2GhCredentialAccess(currentNativeAdmission);
        await access.assertCurrentAuthority();
        draftToken = await access.credential();
        writePort = createR2TestGitHubWritePort({
          currentAuthority: access.assertCurrentAuthority,
          credential: access.credential,
        });
      } else {
        await currentNativeAdmission();
        writePort = paths.testWritePort;
        await writePort.assertCurrentAuthority();
      }
    }
    if (request.runControlledTask === true) {
      const source = await readGitHubFact(
        R2_02_SOURCE, R2_02_TEST_LEDGER_REPOSITORY, fetch, gitReadToken);
      const task = await runR2ControlledProductTask({
        store: service.store, identity: service.identity, source,
        ...(novel === undefined ? {} : { fixtureDriverId: novel.driverId }),
      });
      if (task.state === "ACTION_REPLAY_NO_NEW_RESULT") {
        throw new Error("R2_ACTION_REPLAY_NO_NEW_RESULT");
      }
      controlledTask = Object.freeze({
        ...task.result,
        actionCompletionRef: task.actionCompletionRef,
        manifestHash: task.manifestHash,
        decisionReceiptId: task.decisionReceiptId,
        objectiveOutcomeContentHash: task.objectiveOutcomeContentHash,
        objectiveOutcomeReceiptId: task.objectiveOutcomeReceiptId,
        evaluationContentHash: task.evaluationContentHash,
        evaluationReceiptId: task.evaluationReceiptId,
        metricsHash: task.metricsHash,
        dreamRunContentHash: task.dreamRunContentHash,
        dreamProposalContentHash: task.dreamProposalContentHash,
        dreamProposalState: task.dreamProposalState,
        ...(task.fixtureDriverBinding === undefined ? {} : {
          fixtureDriverBinding: task.fixtureDriverBinding,
        }),
      });
      if (request.publishTestDraft === true) {
        if (writePort === undefined || executionEvidenceSha === undefined ||
            serviceEntrySha256 === undefined) {
          throw new Error("R2_TEST_DRAFT_PORT_UNAVAILABLE");
        }
        const operationId = `r2-02-product-${createHash("sha256")
          .update(`${task.actionCompletionRef}\0${executionEvidenceSha}`)
          .digest("hex").slice(0, 32)}`;
        const bytes = Buffer.from(JSON.stringify({
          schema: "gogoke.s1-r4.r2-02.product-test-result.v1",
          testOnly: true,
          operationId,
          executionEvidenceSha,
          serviceEntrySha256,
          sourceCommit: task.result.sourceCommit,
          sourceBlob: task.result.sourceBlob,
          reportSha256: task.result.reportSha256,
          modelId: task.result.modelId,
          relativePath: task.result.relativePath,
          embeddedBytesSha256: task.result.embeddedBytesSha256,
          actionCompletionRef: task.actionCompletionRef,
          manifestHash: task.manifestHash,
          decisionReceiptId: task.decisionReceiptId,
          objectiveOutcomeContentHash: task.objectiveOutcomeContentHash,
          objectiveOutcomeReceiptId: task.objectiveOutcomeReceiptId,
          evaluationContentHash: task.evaluationContentHash,
          evaluationReceiptId: task.evaluationReceiptId,
          metricsHash: task.metricsHash,
          dreamRunContentHash: task.dreamRunContentHash,
          dreamProposalContentHash: task.dreamProposalContentHash,
          dreamProposalState: task.dreamProposalState,
          ...(task.fixtureDriverBinding === undefined ? {} : {
            fixtureDriverBinding: task.fixtureDriverBinding,
          }),
          acceptance: "TEST_FIXTURE_NOT_ADOPTED",
        }));
        const draft = await writeR2TestFact({ operationId, executionEvidenceSha, bytes },
          writePort, paths.testFetcher ?? fetch, draftToken);
        testLedgerDraft = Object.freeze({
          state: draft.state,
          repository: R2_TEST_LEDGER.repository,
          branch: R2_TEST_LEDGER.branch,
          commit: draft.commit,
          path: draft.path,
          gitBlob: draft.readback.gitBlob,
          contentHash: draft.readback.coordinate.contentHash,
        });
      }
    }
    return Object.freeze({
      goal: request.goal,
      ledger: request.ledger,
      ...(request.runControlledTask === true ? { runControlledTask: true as const } : {}),
      ...(request.publishTestDraft === true ? { publishTestDraft: true as const } : {}),
      ...(request.fixtureDriverId === undefined ? {} : { fixtureDriverId: request.fixtureDriverId }),
      caller: Object.freeze({
        admitted: admission.admitted,
        policyRevision: admission.policyRevision,
        principalId: admission.principalId,
        profileId: admission.profileId,
        revocationHead: admission.revocationHead,
        role: admission.role,
        seatId: admission.seatId,
      }),
      nativeHost: Object.freeze({
        reachable: true as const,
        elapsedMicros: admission.elapsedMicros,
      }),
      ledgerReadback: Object.freeze({ state: ledgerReadback.state, gitBlob: ledgerReadback.gitBlob }),
      ...(controlledTask === undefined ? {} : { controlledTask }),
      ...(testLedgerDraft === undefined ? {} : { testLedgerDraft }),
      acceptance: "TEST_FIXTURE_NOT_ADOPTED" as const,
    });
  } finally {
    await service.close();
  }
}

export async function runGogokeProductProcess(argv: readonly string[]): Promise<void> {
  const paths = parseProductProcessArgs(argv);
  const input = NodeFS.readFileSync(0);
  const request = decodeProductGoalRequest(input);
  const response = await handleProductGoalRequest(request, paths);
  // @effect-diagnostics-next-line preferSchemaOverJson:off - local process response DTO.
  NodeFS.writeFileSync(1, `${JSON.stringify(response)}\n`);
}
