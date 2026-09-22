import { expect, it, vi } from "@effect/vitest";

import {
  createProductionHostCoordinator,
  EpochSingleflight,
  HostCoordinator,
  HostCoordinatorError,
  runtimeInstanceKey,
  snapshotPreparedLaunch,
  type DurableHostStore,
  type DurableCustodyInput,
  type HostAdapters,
  type HostBinding,
  type HostOperationContext,
  type OwnerReconciliation,
  type PreparedLaunch,
  type RuntimeChannel,
  type RuntimeExit,
} from "./coordinator.ts";

const unknownReconciliation = async (): Promise<OwnerReconciliation> => "unknown";

const binding: HostBinding = {
  rootIdentity: "volume:0123456789abcdef/file:00112233445566778899aabbccddeeff",
  profileId: "current-user",
  runtimeInstanceId: "fixture-runtime",
  authRevision: "7",
  generation: "3",
};

const launch: PreparedLaunch = {
  ticketId: "ticket-1",
  custodyRef: "custody-1",
  processIdentity: { processId: 42, creationTime: "133801632000000000" },
};

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

async function waitUntil(assertion: () => void): Promise<void> {
  await vi.waitFor(assertion, { timeout: 500 });
}

function fixture(overrides: Partial<HostAdapters> = {}) {
  const exit = deferred<RuntimeExit>();
  const channel: RuntimeChannel = {
    authenticate: vi.fn(async () => undefined),
    subscribe: vi.fn(async () => undefined),
    initialize: vi.fn(async () => undefined),
    close: vi.fn(async () => undefined),
    exited: exit.promise,
  };
  const store: DurableHostStore = {
    persistOwner: vi.fn(async () => undefined),
    reconcileOwner: vi.fn(unknownReconciliation),
    retainStartupCustody: vi.fn(async () => undefined),
  };
  const adapters: HostAdapters = {
    nativeHost: {
      prepare: vi.fn(async () => launch),
      activate: vi.fn(async () => undefined),
      abortPrepared: vi.fn(async () => undefined),
    },
    store,
    connector: { connect: vi.fn(async () => channel) },
    ...overrides,
  };
  return { adapters, channel, store: adapters.store, exit };
}

it("singleflights 32 starts through one fully owned startup sequence", async () => {
  const order: string[] = [];
  const { adapters, channel } = fixture({
    nativeHost: {
      prepare: vi.fn(async (_binding, context) => {
        expect(context.signal.aborted).toBe(false);
        order.push("prepare");
        return launch;
      }),
      activate: vi.fn(async (_launch, context) => {
        expect(context.deadlineAt).toBeGreaterThan(Date.now());
        order.push("activate");
      }),
      abortPrepared: vi.fn(async () => undefined),
    },
    store: {
      persistOwner: vi.fn(async () => {
        order.push("persist");
      }),
      reconcileOwner: vi.fn(unknownReconciliation),
      retainStartupCustody: vi.fn(async () => undefined),
    },
  });
  vi.mocked(channel.authenticate).mockImplementation(async () => {
    order.push("authenticate");
  });
  vi.mocked(channel.subscribe).mockImplementation(async () => {
    order.push("subscribe");
  });
  vi.mocked(channel.initialize).mockImplementation(async () => {
    order.push("initialize");
  });
  vi.mocked(adapters.connector.connect).mockImplementation(async () => {
    order.push("connect");
    return channel;
  });
  const coordinator = new HostCoordinator(adapters);
  const ready = await Promise.all(
    Array.from({ length: 32 }, () => coordinator.start("public", binding)),
  );

  expect(new Set(ready.map((item) => item.sourceEpoch))).toEqual(new Set(["1"]));
  expect(order).toEqual([
    "prepare",
    "persist",
    "activate",
    "connect",
    "authenticate",
    "subscribe",
    "initialize",
  ]);
  expect(adapters.nativeHost.prepare).toHaveBeenCalledOnce();
  await coordinator.start("public", binding);
  expect(adapters.nativeHost.prepare).toHaveBeenCalledOnce();
});

it("shares one admission key across legacy/public and refuses another runtime", async () => {
  const gate = deferred<PreparedLaunch>();
  const { adapters } = fixture({
    nativeHost: {
      prepare: vi.fn(() => gate.promise),
      activate: vi.fn(async () => undefined),
      abortPrepared: vi.fn(async () => undefined),
    },
  });
  const coordinator = new HostCoordinator(adapters);
  const first = coordinator.start("public", binding);
  await expect(coordinator.start("legacy", { ...binding })).rejects.toMatchObject({
    code: "ROOT_PROFILE_ALREADY_OWNED",
  });
  await expect(
    coordinator.start("public", { ...binding, runtimeInstanceId: "other-runtime" }),
  ).rejects.toMatchObject({ code: "ROOT_PROFILE_ALREADY_OWNED" });
  expect(runtimeInstanceKey(binding)).toBe(runtimeInstanceKey({ ...binding }));
  expect(adapters.nativeHost.prepare).toHaveBeenCalledOnce();
  gate.resolve(launch);
  await first;
});

it("deep-snapshots and freezes a prepared launch before persistence", async () => {
  const mutable = {
    ticketId: "ticket-original",
    custodyRef: "custody-original",
    processIdentity: { processId: 77, creationTime: "123" },
  };
  let persisted!: PreparedLaunch;
  let activated!: PreparedLaunch;
  const { adapters } = fixture({
    nativeHost: {
      prepare: vi.fn(async () => mutable),
      activate: vi.fn(async (value) => {
        activated = value;
      }),
      abortPrepared: vi.fn(async () => undefined),
    },
    store: {
      persistOwner: vi.fn(async (input) => {
        persisted = input.launch;
        mutable.ticketId = "mutated-alias";
        mutable.processIdentity.processId = 999;
        expect(() => {
          (input.launch as { ticketId: string }).ticketId = "persist-mutation";
        }).toThrow();
      }),
      reconcileOwner: vi.fn(unknownReconciliation),
      retainStartupCustody: vi.fn(async () => undefined),
    },
  });
  await new HostCoordinator(adapters).start("public", binding);
  expect(activated.ticketId).toBe("ticket-original");
  expect(activated.processIdentity?.processId).toBe(77);
  expect(activated).toBe(persisted);
  expect(Object.isFrozen(activated)).toBe(true);
  expect(Object.isFrozen(activated.processIdentity)).toBe(true);
});

it("rejects proxy, accessor, symbol, extra and nonplain launch records", () => {
  const accessor = Object.defineProperty({ ...launch }, "ticketId", {
    enumerable: true,
    get: () => "ticket",
  });
  for (const value of [
    new Proxy({ ...launch }, {}),
    accessor,
    { ...launch, extra: true },
    Object.assign(Object.create(null), launch),
    { ...launch, [Symbol("hidden")]: true },
  ]) {
    expect(() => snapshotPreparedLaunch(value)).toThrowError(HostCoordinatorError);
  }
});

it("closes a channel exactly once when connect resolves after timeout", async () => {
  const late = deferred<RuntimeChannel>();
  const close = vi.fn(async () => undefined);
  const lateChannel: RuntimeChannel = {
    authenticate: vi.fn(async () => undefined),
    subscribe: vi.fn(async () => undefined),
    initialize: vi.fn(async () => undefined),
    close,
  };
  let connectContext!: HostOperationContext;
  const { adapters } = fixture({
    connector: {
      connect: vi.fn((_binding, _launch, context) => {
        connectContext = context;
        return late.promise;
      }),
    },
  });
  const coordinator = new HostCoordinator(adapters, 10);
  await expect(coordinator.start("public", binding)).rejects.toMatchObject({
    code: "OPERATION_TIMEOUT",
  });
  expect(connectContext.signal.aborted).toBe(true);
  expect(coordinator.state(binding)).toBe("startup-custody");
  late.resolve(lateChannel);
  await waitUntil(() => expect(close).toHaveBeenCalledOnce());
  await new Promise((resolve) => setTimeout(resolve, 20));
  expect(close).toHaveBeenCalledOnce();
});

it("a rejected prepare leaves fail-closed admission and never prepares again", async () => {
  const prepare = vi.fn(async () => Promise.reject(new Error("prepare rejected")));
  const { adapters } = fixture({
    nativeHost: {
      prepare,
      activate: vi.fn(async () => undefined),
      abortPrepared: vi.fn(async () => undefined),
    },
  });
  const coordinator = new HostCoordinator(adapters, 10);
  await expect(coordinator.start("public", binding)).rejects.toMatchObject({
    code: "PREPARE_OUTCOME_UNKNOWN",
  });
  expect(coordinator.state(binding)).toBe("prepare-outcome-unknown");
  await expect(coordinator.start("public", binding)).rejects.toMatchObject({
    code: "FAILED_CUSTODY",
  });
  expect(prepare).toHaveBeenCalledOnce();
});

it("a hung prepare aborts within deadline and remains custody-blocked", async () => {
  let context!: HostOperationContext;
  const prepare = vi.fn((_binding: HostBinding, value: HostOperationContext) => {
    context = value;
    return new Promise<PreparedLaunch>(() => undefined);
  });
  const { adapters } = fixture({
    nativeHost: {
      prepare,
      activate: vi.fn(async () => undefined),
      abortPrepared: vi.fn(async () => undefined),
    },
  });
  const coordinator = new HostCoordinator(adapters, 10);
  const startedAt = Date.now();
  await expect(coordinator.start("public", binding)).rejects.toMatchObject({
    code: "PREPARE_OUTCOME_UNKNOWN",
  });
  expect(Date.now() - startedAt).toBeLessThan(250);
  expect(context.signal.aborted).toBe(true);
  expect(coordinator.state(binding)).toBe("prepare-outcome-unknown");
  await expect(coordinator.start("public", binding)).rejects.toMatchObject({
    code: "FAILED_CUSTODY",
  });
  expect(prepare).toHaveBeenCalledOnce();
});

it("aborts and retains a launch that resolves after prepare timeout", async () => {
  const late = deferred<PreparedLaunch>();
  const abortPrepared = vi.fn(async () => undefined);
  const retainStartupCustody = vi.fn(
    async (_input: DurableCustodyInput, _context: HostOperationContext): Promise<void> => undefined,
  );
  const { adapters } = fixture({
    nativeHost: {
      prepare: vi.fn(() => late.promise),
      activate: vi.fn(async () => undefined),
      abortPrepared,
    },
    store: {
      persistOwner: vi.fn(async () => undefined),
      reconcileOwner: vi.fn(unknownReconciliation),
      retainStartupCustody,
    },
  });
  const coordinator = new HostCoordinator(adapters, 10);
  await expect(coordinator.start("public", binding)).rejects.toMatchObject({
    code: "PREPARE_OUTCOME_UNKNOWN",
  });
  late.resolve(launch);
  await waitUntil(() => expect(retainStartupCustody).toHaveBeenCalledOnce());
  await waitUntil(() => expect(abortPrepared).toHaveBeenCalledOnce());
  expect(coordinator.state(binding)).toBe("prepare-outcome-unknown");
});

it("a hung persist aborts, reconciles and remains custody-blocked", async () => {
  let context!: HostOperationContext;
  const reconcileOwner = vi.fn(async () => "committed" as const);
  const retainStartupCustody = vi.fn(
    async (_input: DurableCustodyInput, _context: HostOperationContext): Promise<void> => undefined,
  );
  const { adapters } = fixture({
    store: {
      persistOwner: vi.fn((_input, value) => {
        context = value;
        return new Promise<void>(() => undefined);
      }),
      reconcileOwner,
      retainStartupCustody,
    },
  });
  const coordinator = new HostCoordinator(adapters, 10);
  const startedAt = Date.now();
  await expect(coordinator.start("public", binding)).rejects.toMatchObject({
    code: "OPERATION_TIMEOUT",
  });
  expect(Date.now() - startedAt).toBeLessThan(250);
  expect(context.signal.aborted).toBe(true);
  expect(coordinator.state(binding)).toBe("startup-custody");
  await waitUntil(() => expect(reconcileOwner).toHaveBeenCalled());
  await waitUntil(() => expect(retainStartupCustody).toHaveBeenCalled());
  const firstCustodyCall = vi.mocked(retainStartupCustody).mock.calls.at(0);
  expect(firstCustodyCall).toBeDefined();
  if (firstCustodyCall === undefined) throw new Error("expected retained startup custody call");
  expect(firstCustodyCall[0].reason).toContain("persist reconciliation=committed");
  await expect(coordinator.start("public", binding)).rejects.toMatchObject({
    code: "FAILED_CUSTODY",
  });
});

it("a hung activate aborts within deadline and remains custody-blocked", async () => {
  let context!: HostOperationContext;
  const activate = vi.fn((_launch: PreparedLaunch, value: HostOperationContext) => {
    context = value;
    return new Promise<void>(() => undefined);
  });
  const { adapters, store } = fixture({
    nativeHost: {
      prepare: vi.fn(async () => launch),
      activate,
      abortPrepared: vi.fn(async () => undefined),
    },
  });
  const coordinator = new HostCoordinator(adapters, 10);
  const startedAt = Date.now();
  await expect(coordinator.start("public", binding)).rejects.toMatchObject({
    code: "OPERATION_TIMEOUT",
  });
  expect(Date.now() - startedAt).toBeLessThan(250);
  expect(context.signal.aborted).toBe(true);
  expect(coordinator.state(binding)).toBe("startup-custody");
  await waitUntil(() => expect(store.retainStartupCustody).toHaveBeenCalled());
  await expect(coordinator.start("public", binding)).rejects.toMatchObject({
    code: "FAILED_CUSTODY",
  });
});

it("persists and retains unknown identity without activation", async () => {
  const { adapters, store } = fixture({
    nativeHost: {
      prepare: vi.fn(async () => ({ ...launch, processIdentity: null })),
      activate: vi.fn(async () => undefined),
      abortPrepared: vi.fn(async () => undefined),
    },
  });
  const coordinator = new HostCoordinator(adapters);
  await expect(coordinator.start("public", binding)).rejects.toMatchObject({
    code: "UNKNOWN_PROCESS_IDENTITY",
  });
  expect(store.persistOwner).toHaveBeenCalledOnce();
  expect(adapters.nativeHost.activate).not.toHaveBeenCalled();
  await waitUntil(() => expect(store.retainStartupCustody).toHaveBeenCalled());
  expect(coordinator.state(binding)).toBe("startup-custody");
});

it("closes a failed authenticated channel and retains custody", async () => {
  const close = vi.fn(async () => undefined);
  const { adapters, store } = fixture({
    connector: {
      connect: vi.fn(async () => ({
        authenticate: vi.fn(async () => Promise.reject(new Error("auth failed"))),
        subscribe: vi.fn(async () => undefined),
        initialize: vi.fn(async () => undefined),
        close,
      })),
    },
  });
  const coordinator = new HostCoordinator(adapters);
  await expect(coordinator.start("public", binding)).rejects.toThrow("auth failed");
  await waitUntil(() => expect(close).toHaveBeenCalledOnce());
  await waitUntil(() => expect(store.retainStartupCustody).toHaveBeenCalled());
  expect(coordinator.state(binding)).toBe("startup-custody");
});

it.each([
  ["crash", 0xc0000005],
  ["abnormal", 17],
] as const)("records %s exit as residual custody", async (kind, code) => {
  const { adapters, store, exit } = fixture();
  const coordinator = new HostCoordinator(adapters);
  await coordinator.start("public", binding);
  exit.resolve({ kind, code });
  await waitUntil(() => expect(store.retainStartupCustody).toHaveBeenCalled());
  expect(coordinator.state(binding)).toBe("startup-custody");
});

it("late failure from an old epoch cannot clear a replacement connection", async () => {
  const first = deferred<string>();
  const second = deferred<string>();
  const singleflight = new EpochSingleflight<string>();
  const key = runtimeInstanceKey(binding);
  const oldPromise = singleflight.replace(key, "8", () => first.promise);
  const newPromise = singleflight.replace(key, "9", () => second.promise);
  first.reject(new Error("old connection failed late"));
  await expect(oldPromise).rejects.toThrow("old connection failed late");
  expect(singleflight.currentEpoch(key)).toBe("9");
  second.resolve("new connection");
  await expect(newPromise).resolves.toBe("new connection");
});

it("keeps production readiness blocked without DB/VFS/native-host adapters", () => {
  expect(createProductionHostCoordinator).toThrowError(HostCoordinatorError);
});
