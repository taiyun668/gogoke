import { describe, expect, it, vi } from "vite-plus/test";

import {
  ACTION_INTENT_SCHEMA,
  actionCommitmentForPackage,
  classifyActionObservation,
  mapTypedAction,
  semanticDigestForAction,
  TypedActionDispatcher,
  TypedActionError,
  type ActionBinding,
  type ActionBindingAuthority,
  type BeginActionResult,
  type ActionTransportResult,
  type DurableActionReservation,
  type DurableActionState,
  type DurableActionStore,
  type DurableDispatchOutcome,
  type MappedNativeAction,
  type ReserveActionResult,
  type TypedAction,
  type TypedActionIntent,
  type TypedActionTransport,
} from "./typedAction.ts";
import type { AuthorizedTaskPackage } from "../policy/types.ts";

const binding: ActionBinding = {
  bindingId: "binding-main",
  sessionId: "session-main",
  executionId: "execution-worker",
  runtimeInstanceId: "runtime-main",
  profileId: "profile-main",
  authRevision: "7",
  generation: "11",
};

const digest = `sha256:${"a".repeat(64)}`;
const taskPackage: AuthorizedTaskPackage = {
  packageDigest: digest,
  parentGrantRef: "grant-main",
  parentGrantRevision: "3",
  parentGrantRevocationHead: "4",
  parentPolicyRevision: "2",
  parentSeatId: "seat-controller",
  parentGrantDigest: digest,
  parentCeilingDigest: digest,
  childCeiling: {
    allowedActions: ["return-result"],
    allowedTargetPrincipalIds: ["principal-controller"],
    allowedTargetDomainIds: ["domain-controller"],
    allowedSinks: ["task-package"],
    allowedMaterialClasses: ["spec"],
    explicitPrivateMaterialIds: [],
    allowedContinuationResponses: [],
    maxMaterialItems: 1,
    maxMaterialBytes: 1024,
    maxResponseBytes: 64,
  },
  childCeilingDigest: digest,
  action: "delegate",
  route: "controller-worker",
  source: {
    principalId: "principal-controller",
    projectId: "project-main",
    domainId: "domain-controller",
    role: "controller",
  },
  target: {
    principalId: "principal-worker",
    projectId: "project-main",
    domainId: "domain-worker",
    role: "worker",
  },
  sourceBinding: {
    sessionId: "session-controller",
    executionId: "execution-controller",
    generation: "3",
  },
  targetBinding: {
    sessionId: binding.sessionId,
    executionId: binding.executionId,
    generation: binding.generation,
  },
  targetBindingKind: "existing",
  sink: "task-package",
  instruction: "perform bounded action",
  instructionDigest: digest,
  materialSetDigest: digest,
  materials: [],
};

const operationId = (character: string): string => `opr_${character.repeat(32)}`;

function intent(id: string, action: TypedAction, target = binding): TypedActionIntent {
  const unsigned = {
    schema: ACTION_INTENT_SCHEMA,
    operationId: id,
    binding: target,
    taskPackage: {
      ...taskPackage,
      targetBinding: {
        ...taskPackage.targetBinding,
        sessionId: target.sessionId,
        executionId: target.executionId,
        generation: target.generation,
      },
    },
    action,
  } as const;
  return { ...unsigned, semanticDigest: semanticDigestForAction(unsigned) };
}

class FakeAuthority implements ActionBindingAuthority {
  current: ActionBinding | null = binding;
  packageCurrent = true;
  revalidateCalls = 0;
  expectedSendAuthority: string | null = null;
  readonly receivedSendAuthorities: string[] = [];
  readonly transport: TypedActionTransport;

  constructor(transport: TypedActionTransport) {
    this.transport = transport;
  }

  async currentBinding(bindingId: string): Promise<ActionBinding | null> {
    return this.current?.bindingId === bindingId ? this.current : null;
  }

  async revalidateTaskPackage(value: AuthorizedTaskPackage, expected: ActionBinding) {
    this.revalidateCalls += 1;
    if (!this.packageCurrent) {
      throw new TypedActionError("STALE_BINDING", "parent grant revision or ceiling changed");
    }
    if (
      value.targetBinding.sessionId !== expected.sessionId ||
      value.targetBinding.executionId !== expected.executionId ||
      value.targetBinding.generation !== expected.generation
    ) {
      throw new TypedActionError("STALE_BINDING", "task package child binding changed");
    }
    const commitment = actionCommitmentForPackage(value);
    this.expectedSendAuthority = `send:${commitment.packageDigest}`;
    return commitment;
  }

  dispatchIfCurrent(
    expected: ActionBinding,
    action: MappedNativeAction,
    sendAuthority: string,
  ): Promise<ActionTransportResult | { readonly kind: "stale-before-send" }> {
    if (this.current === null || JSON.stringify(this.current) !== JSON.stringify(expected)) {
      return Promise.resolve({ kind: "stale-before-send" });
    }
    this.receivedSendAuthorities.push(sendAuthority);
    if (sendAuthority !== this.expectedSendAuthority) {
      throw new TypedActionError(
        "STORE_PROTOCOL_ERROR",
        "send authority did not match native begin",
      );
    }
    return this.transport.send(action, expected);
  }
}

interface FakeRow {
  readonly digest: string;
  state: DurableActionState;
  readonly reservationId: string;
  readonly reservation: DurableActionReservation;
}

class FakeDurableStore implements DurableActionStore {
  readonly events: string[] = [];
  readonly outcomes: DurableDispatchOutcome[] = [];
  readonly rows = new Map<string, FakeRow>();
  afterReserve: (() => void) | undefined;
  nextReserve: ReserveActionResult | undefined;
  nextBegin: BeginActionResult | undefined;

  async reserve(reservation: DurableActionReservation): Promise<ReserveActionResult> {
    this.events.push(`reserve:${reservation.operationId}:${reservation.action.kind}`);
    if (this.nextReserve !== undefined) {
      const result = this.nextReserve;
      this.nextReserve = undefined;
      return result;
    }
    const existing = this.rows.get(reservation.operationId);
    if (existing !== undefined) {
      if (existing.digest !== reservation.semanticDigest) {
        return {
          kind: "conflict",
          operationId: reservation.operationId,
          existingSemanticDigest: existing.digest,
        };
      }
      return {
        kind: "replay",
        reservationId: existing.reservationId,
        operationId: reservation.operationId,
        semanticDigest: existing.digest,
        state: existing.state,
      };
    }
    const row = {
      digest: reservation.semanticDigest,
      state: "reserved" as const,
      reservationId: `reservation:${reservation.operationId}`,
      reservation,
    };
    this.rows.set(reservation.operationId, row);
    this.afterReserve?.();
    return {
      kind: "reserved",
      reservationId: row.reservationId,
      operationId: reservation.operationId,
      semanticDigest: reservation.semanticDigest,
    };
  }

  async begin(
    reservationId: string,
    reservation: DurableActionReservation,
  ): Promise<BeginActionResult> {
    this.events.push(`begin:${reservation.operationId}`);
    if (this.nextBegin !== undefined) {
      const result = this.nextBegin;
      this.nextBegin = undefined;
      return result;
    }
    const row = this.rows.get(reservation.operationId);
    if (
      row === undefined ||
      row.reservationId !== reservationId ||
      JSON.stringify(row.reservation) !== JSON.stringify(reservation)
    ) {
      return { kind: "conflict", operationId: reservation.operationId, reservationId };
    }
    if (row.state !== "reserved") {
      return {
        kind: "replay",
        operationId: reservation.operationId,
        reservationId,
        state: row.state,
      };
    }
    row.state = "dispatching";
    return {
      kind: "granted",
      operationId: reservation.operationId,
      reservationId,
      sendAuthority: `send:${reservation.commitment.packageDigest}`,
    };
  }

  async recordDispatchOutcome(
    reservationId: string,
    operationIdValue: string,
    semanticDigest: string,
    outcome: DurableDispatchOutcome,
  ): Promise<void> {
    this.events.push(`record:${operationIdValue}:${outcome.kind}`);
    const row = this.rows.get(operationIdValue);
    if (row === undefined || row.reservationId !== reservationId || row.digest !== semanticDigest) {
      throw new Error("fake store identity mismatch");
    }
    row.state = outcome.kind;
    this.outcomes.push(outcome);
  }
}

class FakeTransport implements TypedActionTransport {
  readonly actions: MappedNativeAction[] = [];
  readonly events: string[];
  result: ActionTransportResult = { kind: "accepted", receiptRef: "receipt-1" };

  constructor(events: string[] = []) {
    this.events = events;
  }

  async send(action: MappedNativeAction): Promise<ActionTransportResult> {
    this.actions.push(action);
    this.events.push(`send:${action.kind}`);
    return this.result;
  }
}

describe("S1-04-D typed action seam", () => {
  it("revalidates authority before reserve and leaves reserve, begin, and send untouched when stale", async () => {
    const store = new FakeDurableStore();
    const transport = new FakeTransport();
    const authority = new FakeAuthority(transport);
    authority.packageCurrent = false;
    await expect(
      new TypedActionDispatcher(store, authority).dispatch(
        intent(operationId("4"), { kind: "cancel" }),
      ),
    ).rejects.toMatchObject({ code: "STALE_BINDING" });
    expect(authority.revalidateCalls).toBe(1);
    expect(store.events).toEqual([]);
    expect(transport.actions).toEqual([]);
  });

  it("freshly revalidates after reserve and records not-sent without begin when the parent changes", async () => {
    const store = new FakeDurableStore();
    const transport = new FakeTransport();
    const authority = new FakeAuthority(transport);
    store.afterReserve = () => {
      authority.packageCurrent = false;
    };
    const actionIntent = intent(operationId("5"), {
      kind: "prompt",
      delivery: "queue",
      text: "must revalidate",
    });
    await expect(
      new TypedActionDispatcher(store, authority).dispatch(actionIntent),
    ).rejects.toMatchObject({ code: "STALE_BINDING" });
    expect(authority.revalidateCalls).toBe(2);
    expect(store.events).toEqual([
      `reserve:${actionIntent.operationId}:queue`,
      `record:${actionIntent.operationId}:not-sent`,
    ]);
    expect(store.outcomes).toEqual([{ kind: "not-sent", reason: "STALE_BINDING" }]);
    expect(transport.actions).toEqual([]);
  });

  it("maps queue and steer explicitly and durably reserves before send", async () => {
    expect(mapTypedAction({ kind: "prompt", delivery: "queue", text: "later" })).toEqual({
      kind: "queue",
      text: "later",
    });
    expect(mapTypedAction({ kind: "prompt", delivery: "steer", text: "now" })).toEqual({
      kind: "steer",
      text: "now",
    });
    expect(mapTypedAction({ kind: "cancel" })).toEqual({ kind: "interrupt" });
    expect(mapTypedAction({ kind: "stop", reason: "owner stop" })).toEqual({
      kind: "close",
      reason: "owner stop",
    });

    const store = new FakeDurableStore();
    const transport = new FakeTransport(store.events);
    const dispatcher = new TypedActionDispatcher(store, new FakeAuthority(transport));
    await expect(
      dispatcher.dispatch(
        intent(operationId("1"), { kind: "prompt", delivery: "steer", text: "turn left" }),
      ),
    ).resolves.toEqual({ status: "dispatched", receiptRef: "receipt-1" });
    expect(store.events).toEqual([
      `reserve:${operationId("1")}:steer`,
      `begin:${operationId("1")}`,
      "send:steer",
      `record:${operationId("1")}:dispatched`,
    ]);
  });

  it("passes the exact store-issued send authority into the final host fence", async () => {
    const store = new FakeDurableStore();
    const transport = new FakeTransport();
    const authority = new FakeAuthority(transport);
    const actionIntent = intent(operationId("c"), { kind: "cancel" });
    await new TypedActionDispatcher(store, authority).dispatch(actionIntent);
    expect(authority.receivedSendAuthorities).toEqual([
      `send:${actionIntent.taskPackage.packageDigest}`,
    ]);
  });

  it("rejects same operation with a different semantic digest and never sends it", async () => {
    const store = new FakeDurableStore();
    const transport = new FakeTransport();
    const dispatcher = new TypedActionDispatcher(store, new FakeAuthority(transport));
    const id = operationId("2");
    await dispatcher.dispatch(intent(id, { kind: "prompt", delivery: "queue", text: "first" }));

    await expect(
      dispatcher.dispatch(intent(id, { kind: "prompt", delivery: "steer", text: "first" })),
    ).rejects.toMatchObject({ code: "IDEMPOTENCY_CONFLICT" });
    expect(transport.actions).toHaveLength(1);
  });

  it("returns a durable replay without blindly resending", async () => {
    const store = new FakeDurableStore();
    const transport = new FakeTransport();
    const dispatcher = new TypedActionDispatcher(store, new FakeAuthority(transport));
    const actionIntent = intent(operationId("3"), {
      kind: "prompt",
      delivery: "queue",
      text: "once",
    });
    await dispatcher.dispatch(actionIntent);
    await expect(dispatcher.dispatch(actionIntent)).resolves.toEqual({
      status: "replayed",
      durableState: "dispatched",
    });
    expect(transport.actions).toHaveLength(1);
  });

  it("lets only one of two dispatchers win begin for the same reserved operation", async () => {
    const store = new FakeDurableStore();
    const transport = new FakeTransport();
    const authority = new FakeAuthority(transport);
    const actionIntent = intent(operationId("7"), {
      kind: "prompt",
      delivery: "queue",
      text: "single send",
    });
    const results = await Promise.all([
      new TypedActionDispatcher(store, authority).dispatch(actionIntent),
      new TypedActionDispatcher(store, authority).dispatch(actionIntent),
    ]);
    expect(results.filter((value) => value.status === "dispatched")).toHaveLength(1);
    expect(results.filter((value) => value.status === "replayed")).toHaveLength(1);
    expect(transport.actions).toHaveLength(1);
  });

  it("allows a pre-existing exact reserved row to compete for begin once", async () => {
    const store = new FakeDurableStore();
    const transport = new FakeTransport();
    const authority = new FakeAuthority(transport);
    const actionIntent = intent(operationId("6"), { kind: "cancel" });
    await store.reserve({
      schema: ACTION_INTENT_SCHEMA,
      operationId: actionIntent.operationId,
      semanticDigest: actionIntent.semanticDigest,
      binding: actionIntent.binding,
      commitment: actionCommitmentForPackage(actionIntent.taskPackage),
      lane: "control",
      action: { kind: "interrupt" },
    });
    await expect(
      new TypedActionDispatcher(store, authority).dispatch(actionIntent),
    ).resolves.toMatchObject({ status: "dispatched" });
    expect(transport.actions).toHaveLength(1);
  });

  it("treats a lost begin reply as non-retryable and never sends on replay", async () => {
    const store = new FakeDurableStore();
    const originalBegin = store.begin.bind(store);
    let loseReply = true;
    store.begin = async (reservationId, reservation) => {
      const result = await originalBegin(reservationId, reservation);
      if (loseReply) {
        loseReply = false;
        throw new Error("begin reply lost");
      }
      return result;
    };
    const transport = new FakeTransport();
    const dispatcher = new TypedActionDispatcher(store, new FakeAuthority(transport));
    const actionIntent = intent(operationId("8"), { kind: "cancel" });
    await expect(dispatcher.dispatch(actionIntent)).rejects.toMatchObject({
      code: "BEGIN_OUTCOME_UNKNOWN",
    });
    await expect(dispatcher.dispatch(actionIntent)).resolves.toEqual({
      status: "replayed",
      durableState: "dispatching",
    });
    expect(transport.actions).toHaveLength(0);
  });

  it("rechecks binding authority after a replay lookup before disclosing durable state", async () => {
    const store = new FakeDurableStore();
    const transport = new FakeTransport();
    const authority = new FakeAuthority(transport);
    const actionIntent = intent(operationId("3"), {
      kind: "prompt",
      delivery: "queue",
      text: "once",
    });
    await new TypedActionDispatcher(store, authority).dispatch(actionIntent);
    const replayStore: DurableActionStore = {
      async reserve(reservation) {
        const result = await store.reserve(reservation);
        authority.current = { ...binding, generation: "12" };
        return result;
      },
      begin: store.begin.bind(store),
      recordDispatchOutcome: store.recordDispatchOutcome.bind(store),
    };

    await expect(
      new TypedActionDispatcher(replayStore, authority).dispatch(actionIntent),
    ).rejects.toMatchObject({ code: "STALE_BINDING" });
    expect(transport.actions).toHaveLength(1);
  });

  it("does not deduplicate equal prompt text across distinct operation identities", async () => {
    const store = new FakeDurableStore();
    const transport = new FakeTransport();
    const dispatcher = new TypedActionDispatcher(store, new FakeAuthority(transport));
    const action = { kind: "prompt", delivery: "queue", text: "repeat me" } as const;
    await dispatcher.dispatch(intent(operationId("d"), action));
    await dispatcher.dispatch(intent(operationId("e"), action));
    expect(transport.actions).toEqual([
      { kind: "queue", text: "repeat me" },
      { kind: "queue", text: "repeat me" },
    ]);
  });

  it("does not send when the durable reservation outcome is unknown", async () => {
    const store = new FakeDurableStore();
    const id = operationId("4");
    store.nextReserve = { kind: "unknown", operationId: id };
    const transport = new FakeTransport();
    const dispatcher = new TypedActionDispatcher(store, new FakeAuthority(transport));

    await expect(dispatcher.dispatch(intent(id, { kind: "cancel" }))).rejects.toMatchObject({
      code: "RESERVATION_OUTCOME_UNKNOWN",
    });
    expect(transport.actions).toHaveLength(0);

    const failedStore: DurableActionStore = {
      async reserve() {
        throw new Error("database unavailable");
      },
      async begin() {
        throw new Error("must not be reached");
      },
      async recordDispatchOutcome() {
        throw new Error("must not be reached");
      },
    };
    const failedTransport = new FakeTransport();
    await expect(
      new TypedActionDispatcher(failedStore, new FakeAuthority(failedTransport)).dispatch(
        intent(operationId("f"), { kind: "stop", reason: "owner stop" }),
      ),
    ).rejects.toMatchObject({ code: "RESERVATION_OUTCOME_UNKNOWN" });
    expect(failedTransport.actions).toHaveLength(0);
  });

  it("runs the control lane while an ordinary work send is still pending", async () => {
    let releaseFirst!: () => void;
    let markStarted!: () => void;
    const started = new Promise<void>((resolve) => {
      markStarted = resolve;
    });
    const blocked = new Promise<void>((resolve) => {
      releaseFirst = resolve;
    });
    const order: string[] = [];
    let calls = 0;
    const transport: TypedActionTransport = {
      async send(action) {
        calls += 1;
        order.push(action.kind);
        if (calls === 1) {
          markStarted();
          await blocked;
        }
        return { kind: "accepted" };
      },
    };
    const dispatcher = new TypedActionDispatcher(
      new FakeDurableStore(),
      new FakeAuthority(transport),
    );
    const first = dispatcher.dispatch(
      intent(operationId("5"), { kind: "prompt", delivery: "queue", text: "first" }),
    );
    await started;
    const second = dispatcher.dispatch(
      intent(operationId("6"), { kind: "prompt", delivery: "queue", text: "second" }),
    );
    const control = dispatcher.dispatch(intent(operationId("7"), { kind: "cancel" }));
    await control;
    expect(order).toEqual(["queue", "interrupt"]);
    releaseFirst();
    await Promise.all([first, second]);
    expect(order).toEqual(["queue", "interrupt", "queue"]);
  });

  it("fails closed at the fixed 128 work and 64 control queue limits", async () => {
    const workDispatcher = new TypedActionDispatcher(
      new FakeDurableStore(),
      new FakeAuthority(new FakeTransport()),
    );
    const work = Array.from({ length: 130 }, (_, index) =>
      workDispatcher.dispatch(
        intent(`opr_${index.toString(16).padStart(32, "0")}`, {
          kind: "prompt",
          delivery: "queue",
          text: `work-${index}`,
        }),
      ),
    );
    const workResults = await Promise.allSettled(work);
    expect(workResults.filter((result) => result.status === "fulfilled")).toHaveLength(128);
    expect(
      workResults.filter(
        (result) =>
          result.status === "rejected" &&
          result.reason instanceof TypedActionError &&
          result.reason.code === "QUEUE_CAPACITY_EXCEEDED",
      ),
    ).toHaveLength(2);

    const controlDispatcher = new TypedActionDispatcher(
      new FakeDurableStore(),
      new FakeAuthority(new FakeTransport()),
    );
    const control = Array.from({ length: 66 }, (_, index) =>
      controlDispatcher.dispatch(
        intent(`opr_${(index + 1_000).toString(16).padStart(32, "0")}`, { kind: "cancel" }),
      ),
    );
    const controlResults = await Promise.allSettled(control);
    expect(controlResults.filter((result) => result.status === "fulfilled")).toHaveLength(64);
    expect(
      controlResults.filter(
        (result) =>
          result.status === "rejected" &&
          result.reason instanceof TypedActionError &&
          result.reason.code === "QUEUE_CAPACITY_EXCEEDED",
      ),
    ).toHaveLength(2);
  });

  it("rejects stale generation before reserve and rechecks it after reserve", async () => {
    const store = new FakeDurableStore();
    const transport = new FakeTransport();
    const authority = new FakeAuthority(transport);
    const dispatcher = new TypedActionDispatcher(store, authority);
    const stale = { ...binding, generation: "10" };
    await expect(
      dispatcher.dispatch(intent(operationId("8"), { kind: "cancel" }, stale)),
    ).rejects.toMatchObject({ code: "STALE_BINDING" });
    expect(store.events).toEqual([]);

    const staleSession = { ...binding, sessionId: "session-previous" };
    await expect(
      dispatcher.dispatch(intent(operationId("c"), { kind: "cancel" }, staleSession)),
    ).rejects.toMatchObject({ code: "STALE_BINDING" });
    expect(store.events).toEqual([]);

    store.afterReserve = () => {
      authority.current = { ...binding, generation: "12" };
    };
    await expect(
      dispatcher.dispatch(intent(operationId("9"), { kind: "cancel" })),
    ).rejects.toMatchObject({ code: "STALE_BINDING" });
    expect(transport.actions).toHaveLength(0);
    expect(store.outcomes).toContainEqual({ kind: "not-sent", reason: "STALE_BINDING" });
  });

  it("serializes the final binding check with starting transport despite queued rotation", async () => {
    const store = new FakeDurableStore();
    const observed: string[] = [];
    let current: ActionBinding = binding;
    const authority: ActionBindingAuthority = {
      async currentBinding(bindingId) {
        return current.bindingId === bindingId ? current : null;
      },
      async revalidateTaskPackage(value) {
        return actionCommitmentForPackage(value);
      },
      dispatchIfCurrent(expected, _action) {
        if (JSON.stringify(current) !== JSON.stringify(expected)) {
          return Promise.resolve({ kind: "stale-before-send" as const });
        }
        observed.push(`send:${expected.generation}:actual:${current.generation}`);
        const result = Promise.resolve({ kind: "accepted" as const });
        queueMicrotask(() => {
          current = { ...binding, generation: "12" };
          observed.push("rotated:12");
        });
        return result;
      },
    };
    await new TypedActionDispatcher(store, authority).dispatch(
      intent(operationId("0"), { kind: "prompt", delivery: "queue", text: "race" }),
    );
    expect(observed[0]).toBe("send:11:actual:11");
    expect(observed).toContain("rotated:12");
  });

  it("treats EOF and error as unknown rather than completion", async () => {
    const eofStore = new FakeDurableStore();
    const eofTransport = new FakeTransport();
    const authority = new FakeAuthority(eofTransport);
    eofTransport.result = { kind: "eof" };
    const eofIntent = intent(operationId("a"), { kind: "cancel" });
    await expect(
      new TypedActionDispatcher(eofStore, authority).dispatch(eofIntent),
    ).resolves.toEqual({ status: "outcome-unknown", reason: "EOF" });
    expect(classifyActionObservation(eofIntent, { kind: "eof" })).toBe("outcome-unknown");
    expect(classifyActionObservation(eofIntent, { kind: "error", detail: "lost" })).toBe(
      "outcome-unknown",
    );
    expect(classifyActionObservation(eofIntent, { kind: "partial-frame" })).toBe("outcome-unknown");
    expect(
      classifyActionObservation(eofIntent, {
        kind: "completed",
        operationId: eofIntent.operationId,
        semanticDigest: eofIntent.semanticDigest,
      }),
    ).toBe("completed");
  });

  it("treats an accepted request whose response is lost as unknown and never resends", async () => {
    const store = new FakeDurableStore();
    let providerAcceptances = 0;
    const send = vi.fn(async () => {
      providerAcceptances += 1;
      throw new Error("connection vanished");
    });
    const dispatcher = new TypedActionDispatcher(store, new FakeAuthority({ send }));
    const actionIntent = intent(operationId("b"), { kind: "stop", reason: "owner stopped" });
    await expect(dispatcher.dispatch(actionIntent)).rejects.toBeInstanceOf(TypedActionError);
    await expect(dispatcher.dispatch(actionIntent)).resolves.toEqual({
      status: "replayed",
      durableState: "outcome-unknown",
    });
    expect(send).toHaveBeenCalledTimes(1);
    expect(providerAcceptances).toBe(1);
  });
});
