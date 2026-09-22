import * as NodeAssert from "node:assert/strict";
import { describe, it } from "vite-plus/test";

import {
  LineageError,
  SessionLineageLedger,
  assessExposure,
  inheritExposure,
  snapshotExposureReceipt,
  type ExposureReceipt,
  type NativeSourceCoverage,
  type NativeSessionIdentity,
  type PendingActionRef,
} from "./lineage.ts";
import type { U64String } from "../../contracts/model.ts";

const assertLineageError = (operation: () => unknown, code: LineageError["code"]): void => {
  NodeAssert.throws(
    operation,
    (error: unknown) => error instanceof LineageError && error.code === code,
  );
};

type TestNativeOverrides = Omit<Partial<NativeSessionIdentity>, "generation" | "sourceEpoch"> & {
  readonly generation?: string;
  readonly sourceEpoch?: string;
};

const native = (overrides: TestNativeOverrides = {}): NativeSessionIdentity => ({
  nativeSessionId: overrides.nativeSessionId ?? "native-one",
  bindingId: overrides.bindingId ?? "binding-one",
  generation: (overrides.generation ?? "1") as U64String,
  sourceEpoch: (overrides.sourceEpoch ?? "1") as U64String,
  domainId: overrides.domainId ?? "domain-one",
});

const coverage = (overrides: Partial<ExposureReceipt["nativeSourceCoverage"]> = {}) => ({
  complete: true,
  observations: [{ sourceRef: "manifest://source", status: "COMPLETE" as const }],
  unknownSources: [],
  ...overrides,
});

type TestReceiptOverrides = Omit<Partial<ExposureReceipt>, "generation"> & {
  readonly generation?: string;
};

const receipt = (overrides: TestReceiptOverrides = {}): ExposureReceipt => ({
  receiptId: overrides.receiptId ?? "receipt-one",
  manifestId: overrides.manifestId ?? "manifest-one",
  bindingId: overrides.bindingId ?? "binding-one",
  generation: (overrides.generation ?? "1") as U64String,
  evidenceLevel: overrides.evidenceLevel ?? "NATIVE_ACKED",
  nativeSourceCoverage: overrides.nativeSourceCoverage ?? coverage(),
  taintLabels: overrides.taintLabels ?? [],
  evidenceRefs: overrides.evidenceRefs ?? ["native://ack"],
});

const pendingAction = (overrides: Partial<PendingActionRef> = {}): PendingActionRef => ({
  actionId: "action-one",
  operationId: "operation-one",
  bindingId: "binding-one",
  generation: "1" as U64String,
  state: "PENDING",
  ...overrides,
});

describe("R4-C-LINEAGE exposure evidence", () => {
  it("does not turn HOST_DELIVERED into proof that the model saw the full manifest", () => {
    const delivered = snapshotExposureReceipt(receipt({ evidenceLevel: "HOST_DELIVERED" }));
    const assessment = assessExposure(delivered);
    NodeAssert.equal(assessment.classification, "UNKNOWN");
    NodeAssert.equal(assessment.reason, "HOST_DELIVERY_NOT_OBSERVATION");
    NodeAssert.equal(assessment.evidenceLevel, "HOST_DELIVERED");
  });

  it("requires complete native coverage and keeps unknown sources visible", () => {
    const clean = assessExposure(receipt());
    NodeAssert.equal(clean.classification, "CLEAN");

    const partial = assessExposure(
      receipt({
        nativeSourceCoverage: coverage({
          complete: false,
          observations: [{ sourceRef: "manifest://source", status: "PARTIAL" }],
        }),
      }),
    );
    NodeAssert.equal(partial.classification, "UNKNOWN");
    NodeAssert.deepEqual(partial.unknownSources, []);

    const unknown = assessExposure(
      receipt({
        nativeSourceCoverage: coverage({
          complete: false,
          observations: [{ sourceRef: "native://file-read", status: "UNKNOWN" }],
          unknownSources: ["native://plugin-output"],
        }),
      }),
    );
    NodeAssert.equal(unknown.classification, "UNKNOWN");
    NodeAssert.deepEqual(unknown.unknownSources, ["native://plugin-output", "native://file-read"]);
  });

  it("propagates taint and inherited provenance through a native fork", () => {
    const inherited = inheritExposure(receipt({ taintLabels: ["native-extra-source"] }), {
      receiptId: "receipt-child",
      manifestId: "manifest-child",
      bindingId: "binding-child",
      generation: "2" as U64String,
    });
    NodeAssert.equal(inherited.evidenceLevel, "INHERITED");
    NodeAssert.deepEqual(inherited.taintLabels, ["native-extra-source"]);
    NodeAssert.equal(inherited.nativeSourceCoverage.inheritedFromReceiptId, "receipt-one");
    NodeAssert.equal(assessExposure(inherited).classification, "TAINTED");
  });
});

describe("R4-C-LINEAGE session lifecycle", () => {
  it("isolates NEW_CLEAN from parent native history and keeps parent actions on the parent", () => {
    const ledger = new SessionLineageLedger();
    const initialParent = ledger.createNewClean({ sessionId: "session-parent", ...native() });
    ledger.recordExposure("session-parent", receipt());
    ledger.retainPendingAction("session-parent", pendingAction());

    const child = ledger.createNewClean({
      sessionId: "session-clean",
      ...native({
        nativeSessionId: "native-clean",
        bindingId: "binding-clean",
        generation: "2",
        sourceEpoch: "2",
      }),
    });
    NodeAssert.deepEqual(child.lineage.parentRefs, []);
    NodeAssert.equal(child.exposure, null);
    NodeAssert.deepEqual(child.pendingActions, []);
    NodeAssert.equal(child.materialHandoff, null);
    NodeAssert.equal(initialParent.pendingActions.length, 0);
    NodeAssert.equal(ledger.get("session-parent").pendingActions.length, 1);
  });

  it("requires exact native identity for RESUME and retains pending actions without replay", () => {
    const ledger = new SessionLineageLedger();
    ledger.createNewClean({ sessionId: "session-one", ...native() });
    ledger.retainPendingAction("session-one", pendingAction());

    const resumed = ledger.resume({ sessionId: "session-one", native: native() });
    NodeAssert.equal(resumed.lineage.operationKind, "RESUME");
    NodeAssert.equal(resumed.pendingActionDisposition, "RETAINED_NOT_REPLAYED");
    NodeAssert.equal(resumed.pendingActions[0]?.actionId, "action-one");
    NodeAssert.throws(
      () =>
        ledger.resume({
          sessionId: "session-one",
          native: native({ nativeSessionId: "native-other" }),
        }),
      (error: unknown) =>
        error instanceof LineageError && error.code === "NATIVE_IDENTITY_MISMATCH",
    );
  });

  it("forks with a new native identity while inheriting exposure labels and taint", () => {
    const ledger = new SessionLineageLedger();
    ledger.createNewClean({ sessionId: "session-parent", ...native() });
    ledger.recordExposure(
      "session-parent",
      receipt({
        taintLabels: ["native-extra-source"],
      }),
    );
    ledger.retainPendingAction("session-parent", pendingAction());

    const child = ledger.fork(
      "session-parent",
      {
        sessionId: "session-fork",
        ...native({
          nativeSessionId: "native-fork",
          bindingId: "binding-fork",
          generation: "2",
          sourceEpoch: "2",
        }),
        operation: "NATIVE_FORK",
      },
      { receiptId: "receipt-fork", manifestId: "manifest-fork" },
    );
    NodeAssert.deepEqual(child.lineage.parentRefs, ["session-parent"]);
    NodeAssert.equal(child.lineage.operationKind, "NATIVE_FORK");
    NodeAssert.equal(child.exposure?.evidenceLevel, "INHERITED");
    NodeAssert.deepEqual(child.exposure?.taintLabels, ["native-extra-source"]);
    NodeAssert.equal(child.exposureAssessment.classification, "TAINTED");
    NodeAssert.equal(child.exposure?.nativeSourceCoverage.inheritedFromReceiptId, "receipt-one");
    NodeAssert.equal(child.pendingActionDisposition, "RETAINED_ON_PARENT");
    NodeAssert.deepEqual(child.pendingActions, []);
    NodeAssert.equal(ledger.get("session-parent").pendingActions[0]?.actionId, "action-one");
  });

  it("keeps REBUILD distinct from RESUME and preserves inherited risk", () => {
    const ledger = new SessionLineageLedger();
    ledger.createNewClean({ sessionId: "session-parent", ...native() });
    ledger.recordExposure("session-parent", receipt());
    ledger.retainPendingAction("session-parent", pendingAction());
    const rebuilt = ledger.rebuild(
      "session-parent",
      {
        sessionId: "session-rebuilt",
        ...native({
          nativeSessionId: "native-rebuilt",
          bindingId: "binding-rebuilt",
          generation: "3",
          sourceEpoch: "3",
        }),
        operation: "REBUILD",
      },
      { receiptId: "receipt-rebuilt", manifestId: "manifest-rebuilt" },
    );
    NodeAssert.equal(rebuilt.lineage.operationKind, "REBUILD");
    NodeAssert.notEqual(rebuilt.native.nativeSessionId, "native-one");
    NodeAssert.equal(rebuilt.exposure?.evidenceLevel, "INHERITED");
    NodeAssert.equal(rebuilt.exposureAssessment.classification, "UNKNOWN");
    NodeAssert.deepEqual(rebuilt.pendingActions, []);
    NodeAssert.equal(rebuilt.pendingActionDisposition, "RETAINED_ON_PARENT");
    NodeAssert.equal(ledger.get("session-parent").pendingActions[0]?.actionId, "action-one");
  });

  it("uses explicit material handoff instead of native resume and archive does not stop the process", () => {
    const ledger = new SessionLineageLedger();
    ledger.createNewClean({ sessionId: "session-parent", ...native() });
    ledger.retainPendingAction("session-parent", pendingAction());
    const handoff = ledger.handoff("session-parent", {
      sessionId: "session-handoff",
      ...native({
        nativeSessionId: "native-handoff",
        bindingId: "binding-handoff",
        generation: "2",
        sourceEpoch: "2",
      }),
      materialIds: ["material-one"],
    });
    NodeAssert.equal(handoff.lineage.operationKind, "HANDOFF");
    NodeAssert.equal(handoff.materialHandoff?.nativeResumeUsed, false);
    NodeAssert.deepEqual(handoff.materialHandoff?.materialIds, ["material-one"]);
    NodeAssert.equal(handoff.exposure, null);
    NodeAssert.equal(handoff.exposureAssessment.classification, "UNKNOWN");
    NodeAssert.equal(
      handoff.lineage.inheritedExposure.evidenceLevels.includes("NATIVE_ACKED"),
      false,
    );
    NodeAssert.equal(handoff.pendingActionDisposition, "RETAINED_ON_PARENT");
    NodeAssert.deepEqual(handoff.pendingActions, []);

    const archived = ledger.archive("session-parent");
    NodeAssert.equal(archived.lifecycle, "ARCHIVED");
    NodeAssert.equal(archived.processState, "RUNNING");
    NodeAssert.equal(archived.pendingActions[0]?.state, "PENDING");
  });

  it("returns immutable snapshots and rejects mismatched receipt generations", () => {
    const ledger = new SessionLineageLedger();
    const created = ledger.createNewClean({ sessionId: "session-one", ...native() });
    NodeAssert.equal(Object.isFrozen(created), true);
    NodeAssert.equal(Object.isFrozen(created.lineage), true);
    NodeAssert.throws(() => {
      (created.lineage.parentRefs as string[]).push("forged");
    }, TypeError);
    NodeAssert.throws(
      () => ledger.recordExposure("session-one", receipt({ generation: "2" })),
      (error: unknown) => error instanceof LineageError && error.code === "INVALID_EXPOSURE",
    );
  });
});

describe("R4-C-LINEAGE fresh-audit adversarial axes", () => {
  it("rejects active, inherited, extra, and symbol-bearing top-level records passively", () => {
    let getterReads = 0;
    const getterReceipt = { ...receipt() };
    Object.defineProperty(getterReceipt, "receiptId", {
      configurable: true,
      enumerable: true,
      get: () => {
        getterReads += 1;
        return "receipt-one";
      },
    });
    assertLineageError(() => snapshotExposureReceipt(getterReceipt), "INVALID_INPUT");
    NodeAssert.equal(getterReads, 0);

    let proxyTrapCalls = 0;
    const proxiedReceipt = new Proxy(receipt(), {
      ownKeys: () => {
        proxyTrapCalls += 1;
        throw new Error("proxy trap must not run");
      },
    });
    assertLineageError(() => snapshotExposureReceipt(proxiedReceipt), "INVALID_INPUT");
    NodeAssert.equal(proxyTrapCalls, 0);

    const inheritedReceipt = Object.assign(Object.create({ inherited: true }), receipt());
    assertLineageError(() => snapshotExposureReceipt(inheritedReceipt), "INVALID_INPUT");
    assertLineageError(
      () => snapshotExposureReceipt({ ...receipt(), extra: true } as ExposureReceipt),
      "INVALID_INPUT",
    );
    const symbolReceipt = { ...receipt() } as ExposureReceipt & { [key: symbol]: unknown };
    symbolReceipt[Symbol("extra")] = "rejected";
    assertLineageError(() => snapshotExposureReceipt(symbolReceipt), "INVALID_INPUT");

    let inputGetterReads = 0;
    const input = { sessionId: "session-one", ...native() };
    Object.defineProperty(input, "sessionId", {
      configurable: true,
      enumerable: true,
      get: () => {
        inputGetterReads += 1;
        return "session-one";
      },
    });
    assertLineageError(() => new SessionLineageLedger().createNewClean(input), "INVALID_INPUT");
    NodeAssert.equal(inputGetterReads, 0);
  });

  it("rejects hostile nested records and arrays before reading their values", () => {
    let nestedGetterReads = 0;
    const getterCoverage = coverage();
    Object.defineProperty(getterCoverage, "complete", {
      configurable: true,
      enumerable: true,
      get: () => {
        nestedGetterReads += 1;
        return true;
      },
    });
    assertLineageError(
      () => snapshotExposureReceipt(receipt({ nativeSourceCoverage: getterCoverage })),
      "INVALID_INPUT",
    );
    NodeAssert.equal(nestedGetterReads, 0);

    const extraObservation = {
      sourceRef: "manifest://source",
      status: "COMPLETE" as const,
      extra: true,
    };
    const invalidObservationCoverage = coverage({
      observations: [extraObservation as unknown as NativeSourceCoverage["observations"][number]],
    });
    assertLineageError(
      () => snapshotExposureReceipt(receipt({ nativeSourceCoverage: invalidObservationCoverage })),
      "INVALID_INPUT",
    );

    const extraArrayProperty = ["native://ack"];
    Object.defineProperty(extraArrayProperty, "extra", { value: "rejected" });
    assertLineageError(
      () => snapshotExposureReceipt(receipt({ evidenceRefs: extraArrayProperty })),
      "INVALID_INPUT",
    );

    let arrayProxyTrapCalls = 0;
    const proxiedObservations = new Proxy(coverage().observations, {
      ownKeys: () => {
        arrayProxyTrapCalls += 1;
        throw new Error("array proxy trap must not run");
      },
    });
    assertLineageError(
      () =>
        snapshotExposureReceipt(
          receipt({ nativeSourceCoverage: coverage({ observations: proxiedObservations }) }),
        ),
      "INVALID_INPUT",
    );
    NodeAssert.equal(arrayProxyTrapCalls, 0);
  });

  it("keeps NEW_CLEAN isolated and records missing fork exposure as UNKNOWN", () => {
    const ledger = new SessionLineageLedger();
    const input = { sessionId: "session-clean", ...native() };
    const created = ledger.createNewClean(input);
    NodeAssert.deepEqual(created.lineage.inheritedExposure, {
      evidenceLevels: [],
      taintLabels: [],
      unknownSources: [],
      sourceReceiptRefs: [],
    });
    NodeAssert.strictEqual(ledger.createNewClean(input), created);

    const forkInput = {
      sessionId: "session-clean-fork",
      ...native({ nativeSessionId: "native-clean-fork", bindingId: "binding-clean-fork" }),
      operation: "NATIVE_FORK" as const,
    };
    const receiptIds = { receiptId: "receipt-clean-fork", manifestId: "manifest-clean-fork" };
    const fork = ledger.fork("session-clean", forkInput, receiptIds);
    NodeAssert.deepEqual(fork.lineage.inheritedExposure.evidenceLevels, ["UNKNOWN"]);
    NodeAssert.equal(fork.exposureAssessment.classification, "UNKNOWN");
    NodeAssert.strictEqual(ledger.fork("session-clean", forkInput, receiptIds), fork);
  });

  it("reserves full native tuples across every creation path and permits only exact replay", () => {
    const ledger = new SessionLineageLedger();
    const firstInput = { sessionId: "session-owner", ...native() };
    const owner = ledger.createNewClean(firstInput);
    NodeAssert.strictEqual(ledger.createNewClean(firstInput), owner);
    assertLineageError(
      () => ledger.createNewClean({ sessionId: "session-reuse", ...native() }),
      "NATIVE_IDENTITY_MISMATCH",
    );

    ledger.createNewClean({
      sessionId: "parent-fork",
      ...native({ nativeSessionId: "native-parent-fork", bindingId: "binding-parent-fork" }),
    });
    ledger.createNewClean({
      sessionId: "parent-rebuild",
      ...native({ nativeSessionId: "native-parent-rebuild", bindingId: "binding-parent-rebuild" }),
    });
    ledger.createNewClean({
      sessionId: "parent-handoff",
      ...native({ nativeSessionId: "native-parent-handoff", bindingId: "binding-parent-handoff" }),
    });

    assertLineageError(
      () =>
        ledger.fork(
          "parent-fork",
          { sessionId: "fork-reuses-tuple", ...native(), operation: "NATIVE_FORK" },
          { receiptId: "receipt-fork-reuse", manifestId: "manifest-fork-reuse" },
        ),
      "NATIVE_IDENTITY_MISMATCH",
    );
    assertLineageError(
      () =>
        ledger.rebuild(
          "parent-rebuild",
          { sessionId: "rebuild-reuses-tuple", ...native(), operation: "REBUILD" },
          { receiptId: "receipt-rebuild-reuse", manifestId: "manifest-rebuild-reuse" },
        ),
      "NATIVE_IDENTITY_MISMATCH",
    );
    assertLineageError(
      () =>
        ledger.handoff("parent-handoff", {
          sessionId: "handoff-reuses-tuple",
          ...native(),
          materialIds: ["material-reuse"],
        }),
      "NATIVE_IDENTITY_MISMATCH",
    );

    const forkInput = {
      sessionId: "session-fork",
      ...native({ nativeSessionId: "native-fork", bindingId: "binding-fork" }),
      operation: "NATIVE_FORK" as const,
    };
    const forkReceipts = { receiptId: "receipt-fork", manifestId: "manifest-fork" };
    const fork = ledger.fork("parent-fork", forkInput, forkReceipts);
    NodeAssert.strictEqual(ledger.fork("parent-fork", forkInput, forkReceipts), fork);
    assertLineageError(
      () =>
        ledger.fork("parent-fork", forkInput, {
          receiptId: "receipt-fork-conflict",
          manifestId: "manifest-fork-conflict",
        }),
      "DUPLICATE_SESSION",
    );

    const rebuildInput = {
      sessionId: "session-rebuild",
      ...native({ nativeSessionId: "native-rebuild", bindingId: "binding-rebuild" }),
      operation: "REBUILD" as const,
    };
    const rebuildReceipts = { receiptId: "receipt-rebuild", manifestId: "manifest-rebuild" };
    const rebuilt = ledger.rebuild("parent-rebuild", rebuildInput, rebuildReceipts);
    NodeAssert.strictEqual(
      ledger.rebuild("parent-rebuild", rebuildInput, rebuildReceipts),
      rebuilt,
    );

    const handoffInput = {
      sessionId: "session-handoff",
      ...native({ nativeSessionId: "native-handoff", bindingId: "binding-handoff" }),
      materialIds: ["material-one"],
    };
    const handoff = ledger.handoff("parent-handoff", handoffInput);
    NodeAssert.strictEqual(ledger.handoff("parent-handoff", handoffInput), handoff);
  });

  it("retains every parent pending action when creating children and archiving", () => {
    const ledger = new SessionLineageLedger();
    ledger.createNewClean({ sessionId: "session-parent", ...native() });
    ledger.retainPendingAction("session-parent", pendingAction());

    const fork = ledger.fork(
      "session-parent",
      {
        sessionId: "session-fork",
        ...native({ nativeSessionId: "native-fork", bindingId: "binding-fork" }),
        operation: "NATIVE_FORK",
      },
      { receiptId: "receipt-fork", manifestId: "manifest-fork" },
    );
    const rebuilt = ledger.rebuild(
      "session-parent",
      {
        sessionId: "session-rebuilt",
        ...native({ nativeSessionId: "native-rebuilt", bindingId: "binding-rebuilt" }),
        operation: "REBUILD",
      },
      { receiptId: "receipt-rebuilt", manifestId: "manifest-rebuilt" },
    );
    const handoff = ledger.handoff("session-parent", {
      sessionId: "session-handoff",
      ...native({ nativeSessionId: "native-handoff", bindingId: "binding-handoff" }),
      materialIds: ["material-one"],
    });

    for (const child of [fork, rebuilt, handoff]) {
      NodeAssert.deepEqual(child.pendingActions, []);
      NodeAssert.equal(child.pendingActionDisposition, "RETAINED_ON_PARENT");
    }
    NodeAssert.deepEqual(ledger.get("session-parent").pendingActions, [pendingAction()]);
    NodeAssert.deepEqual(ledger.archive("session-parent").pendingActions, [pendingAction()]);
  });

  it("carries handoff provenance and taint without claiming resume or child exposure", () => {
    const ledger = new SessionLineageLedger();
    ledger.createNewClean({ sessionId: "session-parent", ...native() });
    const parentReceipt = receipt({
      taintLabels: ["native-extra-source"],
      nativeSourceCoverage: coverage({
        complete: false,
        observations: [
          { sourceRef: "native://unobserved", status: "UNKNOWN" },
          { sourceRef: "native://partial", status: "PARTIAL" },
        ],
        unknownSources: ["native://plugin-output"],
      }),
      evidenceRefs: ["native://ack", "evidence://parent"],
    });
    ledger.recordExposure("session-parent", parentReceipt);

    const handoff = ledger.handoff("session-parent", {
      sessionId: "session-handoff",
      ...native({ nativeSessionId: "native-handoff", bindingId: "binding-handoff" }),
      materialIds: ["material-one"],
    });
    NodeAssert.equal(handoff.materialHandoff?.nativeResumeUsed, false);
    NodeAssert.equal(handoff.exposure, null);
    NodeAssert.equal(handoff.exposureAssessment.classification, "TAINTED");
    NodeAssert.deepEqual(handoff.lineage.inheritedExposure.evidenceLevels, ["NATIVE_ACKED"]);
    NodeAssert.deepEqual(handoff.lineage.inheritedExposure.taintLabels, ["native-extra-source"]);
    NodeAssert.deepEqual(handoff.lineage.inheritedExposure.unknownSources, [
      "native://plugin-output",
      "native://unobserved",
      "native://partial",
    ]);
    NodeAssert.ok(handoff.lineage.inheritedExposure.sourceReceiptRefs.includes("receipt-one"));
    NodeAssert.ok(
      handoff.lineage.inheritedExposure.sourceReceiptRefs.includes("evidence://parent"),
    );
  });

  it("carries inherited exposure through handoff, fork, and rebuild chains", () => {
    const ledger = new SessionLineageLedger();
    ledger.createNewClean({ sessionId: "session-root", ...native() });
    ledger.recordExposure(
      "session-root",
      receipt({
        taintLabels: ["inherited-taint"],
        nativeSourceCoverage: coverage({
          complete: false,
          observations: [{ sourceRef: "native://parent-partial", status: "PARTIAL" }],
        }),
      }),
    );
    const firstHandoff = ledger.handoff("session-root", {
      sessionId: "session-handoff-one",
      ...native({ nativeSessionId: "native-handoff-one", bindingId: "binding-handoff-one" }),
      materialIds: ["material-one"],
    });
    const children = [
      ledger.handoff("session-handoff-one", {
        sessionId: "session-handoff-two",
        ...native({ nativeSessionId: "native-handoff-two", bindingId: "binding-handoff-two" }),
        materialIds: ["material-two"],
      }),
      ledger.fork(
        "session-handoff-one",
        {
          sessionId: "session-fork-after-handoff",
          ...native({ nativeSessionId: "native-fork", bindingId: "binding-fork" }),
          operation: "NATIVE_FORK",
        },
        { receiptId: "receipt-fork-after-handoff", manifestId: "manifest-fork-after-handoff" },
      ),
      ledger.rebuild(
        "session-handoff-one",
        {
          sessionId: "session-rebuild-after-handoff",
          ...native({ nativeSessionId: "native-rebuild", bindingId: "binding-rebuild" }),
          operation: "REBUILD",
        },
        {
          receiptId: "receipt-rebuild-after-handoff",
          manifestId: "manifest-rebuild-after-handoff",
        },
      ),
    ];

    NodeAssert.equal(firstHandoff.exposureAssessment.classification, "TAINTED");
    for (const child of children) {
      NodeAssert.equal(child.exposure, null);
      NodeAssert.equal(child.exposureAssessment.classification, "TAINTED");
      NodeAssert.deepEqual(child.lineage.inheritedExposure.taintLabels, ["inherited-taint"]);
      NodeAssert.ok(
        child.lineage.inheritedExposure.unknownSources.includes("native://parent-partial"),
      );
      NodeAssert.ok(child.lineage.inheritedExposure.sourceReceiptRefs.includes("receipt-one"));
    }
  });

  it("does not let a clean current receipt wash inherited taint or unknown evidence", () => {
    const scenarios = [
      {
        suffix: "tainted",
        initial: receipt({ taintLabels: ["inherited-taint"] }),
        classification: "TAINTED" as const,
      },
      {
        suffix: "unknown",
        initial: receipt({
          nativeSourceCoverage: coverage({
            complete: false,
            observations: [{ sourceRef: "native://incomplete-parent", status: "PARTIAL" }],
          }),
        }),
        classification: "UNKNOWN" as const,
      },
    ];

    for (const scenario of scenarios) {
      const ledger = new SessionLineageLedger();
      const parentId = `session-parent-${scenario.suffix}`;
      const childId = `session-handoff-${scenario.suffix}`;
      const bindingId = `binding-handoff-${scenario.suffix}`;
      ledger.createNewClean({ sessionId: parentId, ...native() });
      ledger.recordExposure(parentId, scenario.initial);
      ledger.handoff(parentId, {
        sessionId: childId,
        ...native({
          nativeSessionId: `native-handoff-${scenario.suffix}`,
          bindingId,
          generation: "2",
          sourceEpoch: "2",
        }),
        materialIds: ["material-one"],
      });

      const updated = ledger.recordExposure(
        childId,
        receipt({
          receiptId: `receipt-clean-${scenario.suffix}`,
          manifestId: `manifest-clean-${scenario.suffix}`,
          bindingId,
          generation: "2",
        }),
      );
      NodeAssert.equal(updated.exposureAssessment.classification, scenario.classification);
      if (scenario.suffix === "tainted") {
        NodeAssert.ok(updated.exposureAssessment.taintLabels.includes("inherited-taint"));
      } else {
        NodeAssert.ok(
          updated.exposureAssessment.unknownSources.includes("native://incomplete-parent"),
        );
      }
    }
  });

  it("makes receipt ids immutable and keeps UNKNOWN or TAINTED evidence from being laundered", () => {
    const scenarios = [
      {
        initial: receipt({
          receiptId: "receipt-unknown",
          evidenceLevel: "UNKNOWN",
          nativeSourceCoverage: coverage({
            complete: false,
            observations: [{ sourceRef: "native://unknown", status: "UNKNOWN" }],
          }),
        }),
        classification: "UNKNOWN" as const,
      },
      {
        initial: receipt({ receiptId: "receipt-tainted", taintLabels: ["native-extra-source"] }),
        classification: "TAINTED" as const,
      },
    ];

    for (const { initial, classification } of scenarios) {
      const ledger = new SessionLineageLedger();
      ledger.createNewClean({ sessionId: "session-one", ...native() });
      const first = ledger.recordExposure("session-one", initial);
      NodeAssert.equal(first.exposureAssessment.classification, classification);
      NodeAssert.strictEqual(ledger.recordExposure("session-one", initial), first);

      const cleanConflict = receipt({
        ...initial,
        evidenceLevel: "NATIVE_ACKED",
        nativeSourceCoverage: coverage(),
        taintLabels: [],
      });
      assertLineageError(
        () => ledger.recordExposure("session-one", cleanConflict),
        "INVALID_EXPOSURE",
      );
      NodeAssert.strictEqual(ledger.get("session-one"), first);
      NodeAssert.equal(ledger.get("session-one").exposureAssessment.classification, classification);

      ledger.createNewClean({
        sessionId: "session-two",
        ...native({ nativeSessionId: "native-two", sourceEpoch: "2" }),
      });
      assertLineageError(() => ledger.recordExposure("session-two", initial), "INVALID_EXPOSURE");
    }
  });
});
