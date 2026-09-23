import type { NativeStoreSession } from "../../bootstrap/nativeStoreService.ts";
import type { ContextManifest } from "../../contracts/model.ts";
import type {
  NativeR2ActionDecisionBasis,
  NativeR2TestDelegationReceipt,
} from "../../persistence/base/nativeHostClient.ts";
import { ContextManifestAssembler } from "./assembler.ts";
import { createNativeContextAssemblyAuthorityPort } from "./nativeAuthorityPort.ts";

/** Public fixture Context: zero private partitions, committed through native authority. */
export async function prepareR2ControlledManifest(input: {
  readonly store: NativeStoreSession;
  readonly basis: NativeR2ActionDecisionBasis;
  readonly grant: NativeR2TestDelegationReceipt;
  readonly recordedAt: string;
}): Promise<ContextManifest> {
  const { store, basis, grant, recordedAt } = input;
  if (basis.state !== "TEST_ONLY_DECISION_BASIS_NOT_ACTION" ||
      basis.bindingId !== "binding-r2-02-worker" || basis.bindingGeneration !== "1" ||
      grant.state !== "TEST_ONLY_GRANT_PREPARED_NOT_ACTION" ||
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
    maxContentBytes: 1,
    maxCandidates: 1,
    partitionBindings: [],
  });
  const native = createNativeContextAssemblyAuthorityPort({
    store,
    plan: { snapshot, recordedAt },
    content: {
      async search() { return []; },
      async load() { return []; },
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
    maxContentBytes: 1,
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
