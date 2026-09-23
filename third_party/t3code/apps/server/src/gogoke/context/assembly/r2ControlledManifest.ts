import type { NativeStoreSession } from "../../bootstrap/nativeStoreService.ts";
import type { ContextManifest, ContextObject } from "../../contracts/model.ts";
import type { GitFactReadback } from "../repository/gitFact.ts";
import type {
  NativeR2ActionDecisionBasis,
  NativeR2TestContextGrantReceipt,
  NativeR2TestDelegationReceipt,
} from "../../persistence/base/nativeHostClient.ts";
import { ContextManifestAssembler } from "./assembler.ts";
import { createNativeContextAssemblyAuthorityPort } from "./nativeAuthorityPort.ts";

const SOURCE_DOMAIN = "domain-r2-02-source";
const CONTEXT_ID = "context-r2-02-public-fixture";
const SOURCE_COMMIT = "f6a820dda05a3eac5c29be48c4149bff7e1c9598";
const SOURCE_PATH = "apps/desktop/test-fixtures/s1-r4/sealing/model-asset.json";
const SOURCE_HASH = "sha256:268f5e2c65e254e7dd55e6e8dfc8eabfa23f7a14a9cfd1be8a998f1297cefa8e";
const SOURCE_REF = `git:${SOURCE_COMMIT}:${SOURCE_PATH}`;
const AUTHORITY_REF = "github:taiyun668/gogoke";

export async function prepareR2PublicContext(input: {
  readonly store: NativeStoreSession;
  readonly source: GitFactReadback;
  readonly grant: NativeR2TestContextGrantReceipt;
  readonly policyRevision: string;
}): Promise<void> {
  const { store, source, grant, policyRevision } = input;
  if (grant.state !== "TEST_ONLY_CONTEXT_GRANT_PREPARED" ||
      source.state !== "COMMITTED_BYTES_VERIFIED_NOT_ADOPTED" ||
      source.coordinate.repository !== "taiyun668/gogoke" ||
      source.coordinate.commit !== SOURCE_COMMIT ||
      source.coordinate.path !== SOURCE_PATH ||
      source.coordinate.contentHash !== SOURCE_HASH ||
      policyRevision !== "1") {
    throw new Error("INVALID_R2_PUBLIC_CONTEXT_SOURCE");
  }
  const receipt = await store.commitContextVersion({
    operationId: "r2-02-public-context",
    object: {
      contextId: CONTEXT_ID,
      version: "1",
      scope: "PROJECT",
      domainId: SOURCE_DOMAIN,
      kind: "fact",
      contentHash: SOURCE_HASH,
      sourceRef: { ref: SOURCE_REF, hash: SOURCE_HASH },
      sourceAuthority: { kind: "repository", ref: AUTHORITY_REF },
      derivedFrom: [], validity: "ACTIVE", supersedes: [],
      accessPolicyRevision: policyRevision,
    } as ContextObject,
    access: { visibility: "DOMAIN_GRANTED", readGrantRefs: [grant.grantRef] },
  });
  if (receipt.contextId !== CONTEXT_ID || receipt.version !== "1" ||
      !["COMMITTED", "RECONCILED"].includes(receipt.disposition)) {
    throw new Error("R2_PUBLIC_CONTEXT_COMMIT_UNCONFIRMED");
  }
}

/** Public PROJECT fixture Context, loaded only after native grant authorization. */
export async function prepareR2ControlledManifest(input: {
  readonly store: NativeStoreSession;
  readonly basis: NativeR2ActionDecisionBasis;
  readonly grant: NativeR2TestDelegationReceipt;
  readonly contextGrant: NativeR2TestContextGrantReceipt;
  readonly source: GitFactReadback;
  readonly recordedAt: string;
}): Promise<ContextManifest> {
  const { store, basis, grant, contextGrant, source, recordedAt } = input;
  if (basis.state !== "TEST_ONLY_DECISION_BASIS_NOT_ACTION" ||
      basis.bindingId !== "binding-r2-02-worker" || basis.bindingGeneration !== "1" ||
      grant.state !== "TEST_ONLY_GRANT_PREPARED_NOT_ACTION" ||
      contextGrant.state !== "TEST_ONLY_CONTEXT_GRANT_PREPARED" ||
      source.coordinate.commit !== SOURCE_COMMIT || source.coordinate.path !== SOURCE_PATH ||
      source.coordinate.contentHash !== SOURCE_HASH ||
      !/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$/u.test(recordedAt)) {
    throw new Error("INVALID_R2_TEST_MANIFEST_BASIS");
  }
  const snapshot = Object.freeze({
    operationId: "r2-02-context-assembly",
    principalId: "principal-r2-02-worker",
    seatId: "seat-r2-02-worker",
    taskId: "task-r2-02-test",
    sessionId: "session-r2-02-worker",
    domainId: "domain-r2-02-test",
    bindingId: basis.bindingId,
    bindingGeneration: basis.bindingGeneration,
    sourceEpoch: "1",
    runtimeInstanceId: "runtime-r2-02-fixture",
    taskRevision: basis.taskRevision,
    policyRevision: basis.policyRevision,
    authRevision: basis.policyRevision,
    revocationHead: grant.revocationHead,
    selectionDecisionId: "decision-r2-02-test",
    manifestId: "manifest-r2-02-test",
    admissionActionOperationId: "opr_22222222222222222222222222222222",
    admissionDigest: basis.actionDigest,
    maxContentBytes: source.bytes.length,
    maxCandidates: 1,
    partitionBindings: [{
      sourceDomainId: SOURCE_DOMAIN,
      destinationScope: "PROJECT" as const,
      promotionKind: "PROJECT_ONLY",
      grant: { grantId: contextGrant.grantRef, revision: contextGrant.revision,
        revocationHead: contextGrant.revocationHead },
    }],
  });
  const native = createNativeContextAssemblyAuthorityPort({
    store,
    plan: { snapshot, recordedAt },
    content: {
      async search({ authorizedSources }) {
        return authorizedSources.filter((row) => row.sourceDomainId === SOURCE_DOMAIN &&
          row.contextId === CONTEXT_ID && row.version === "1" &&
          row.contentHash === SOURCE_HASH).map((row) => ({
            sourceDomainId: row.sourceDomainId, contextId: row.contextId, version: row.version,
          }));
      },
      async load({ references, authorizedSources, authorizedReads }) {
        if (references.length !== 1 || authorizedSources.length !== 1 ||
            authorizedReads.length !== 1) throw new Error("R2_CONTEXT_READ_NOT_AUTHORIZED");
        const row = authorizedSources[0]!;
        const read = authorizedReads[0]!;
        if (row.sourceDomainId !== SOURCE_DOMAIN || row.contextId !== CONTEXT_ID ||
            row.version !== "1" || row.contentHash !== SOURCE_HASH ||
            row.sourceRef !== SOURCE_REF || row.sourceHash !== SOURCE_HASH ||
            row.sourceAuthorityKind !== "repository" ||
            row.sourceAuthorityRef !== AUTHORITY_REF ||
            read.grantRevision !== contextGrant.revision ||
            read.revocationHead !== contextGrant.revocationHead ||
            read.state !== "ACTIVE") throw new Error("R2_CONTEXT_READ_NOT_AUTHORIZED");
        return [{
          object: {
            contextId: CONTEXT_ID, version: "1", scope: "PROJECT", domainId: SOURCE_DOMAIN,
            kind: "fact", contentHash: SOURCE_HASH,
            sourceRef: { ref: SOURCE_REF, hash: SOURCE_HASH },
            sourceAuthority: { kind: "repository", ref: AUTHORITY_REF },
            derivedFrom: [], validity: "ACTIVE", supersedes: [],
            accessPolicyRevision: row.accessPolicyRevision,
          } as ContextObject,
          content: Buffer.from(source.bytes).toString("utf8"),
          state: "ACTIVE" as const,
          stateRevision: read.stateRevision,
          accessPolicyRevision: read.accessPolicyRevision,
        }];
      },
    },
  });
  const assembler = new ContextManifestAssembler(Object.freeze({
    openAssembly: native.openAssembly.bind(native),
    searchVisibleContext: native.searchVisibleContext.bind(native),
    loadVisibleVersions: native.loadVisibleVersions.bind(native),
    commitManifest: native.commitManifest.bind(native),
    readCurrentManifest: native.readCurrentManifest.bind(native),
  }));
  return assembler.assemble({
    operationId: snapshot.operationId,
    manifestId: snapshot.manifestId,
    query: "",
    maxContentBytes: source.bytes.length,
    principalId: snapshot.principalId,
    seatId: snapshot.seatId,
    taskId: snapshot.taskId,
    sessionId: snapshot.sessionId,
    domainId: snapshot.domainId,
    bindingId: snapshot.bindingId,
    bindingGeneration: snapshot.bindingGeneration,
    sourceEpoch: snapshot.sourceEpoch,
    runtimeInstanceId: snapshot.runtimeInstanceId,
  });
}
