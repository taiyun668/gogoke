import assert from "node:assert/strict";
import test from "node:test";
import { CustodyService, hashStopProof, stopProofErrors } from "./custody.ts";
import {
  PRODUCTION_STOP_BUDGETS,
  STOP_REFUSED_EXIT_CODE,
  STOP_TIMEOUT_EXIT_CODE,
  type CustodyRecord,
  type DurableCustodyStore,
  type NativeCustodyAdapter,
  type NativePrepareRequest,
  type NativeStopConfirmation,
  type NativeStopProof,
  type PreparedCustody,
  type ReservePreparedResult,
  type StopBudgets,
} from "./model.ts";

const DIGEST = `sha256:${"1".repeat(64)}`;
const NONCE = `pcn1_${"2".repeat(64)}`;

const launchRequest = (writerDomain = "writer-a") => ({
  application: String.raw`C:\controlled-fixture\fake-native.exe`,
  arguments: ["--controlled"],
  currentDirectory: String.raw`C:\controlled-fixture`,
  hideWindow: true,
  binding: {
    binaryDigestSha256: DIGEST,
    profileId: "profile-a",
    domainId: "domain-a",
    generation: "1",
  },
  writerDomain,
});

class MemoryNative implements NativeCustodyAdapter {
  prepareCount = 0;
  activateCount = 0;
  abortCount = 0;
  stopCount = 0;
  confirmCount = 0;
  neverResolveStop = false;
  stopPatch: Partial<NativeStopProof> = {};
  readonly prepared = new Map<string, PreparedCustody>();
  readonly order: string[] = [];

  async prepare(request: NativePrepareRequest, _signal: AbortSignal): Promise<PreparedCustody> {
    this.prepareCount += 1;
    const ticket = `pct1_${this.prepareCount.toString(16).padStart(64, "0")}`;
    const prepared = {
      ticket,
      custodianNonce: NONCE,
      binding: structuredClone(request.binding),
      identity: {
        pid: 4_000 + this.prepareCount,
        creationTime100ns: String(133_801_632_000_000_000n + BigInt(this.prepareCount)),
        imagePath: request.application,
      },
    };
    this.prepared.set(ticket, prepared);
    this.order.push(`prepare:${ticket}`);
    return prepared;
  }

  async activate(prepared: PreparedCustody, _signal: AbortSignal): Promise<void> {
    assert.deepEqual(this.prepared.get(prepared.ticket), prepared);
    this.activateCount += 1;
    this.order.push(`activate:${prepared.ticket}`);
  }

  async abortPrepared(prepared: PreparedCustody, _signal: AbortSignal): Promise<void> {
    this.abortCount += 1;
    this.prepared.delete(prepared.ticket);
    this.order.push(`abort:${prepared.ticket}`);
  }

  async stop(
    prepared: PreparedCustody,
    _budgets: StopBudgets,
    signal: AbortSignal,
  ): Promise<NativeStopProof> {
    this.stopCount += 1;
    if (this.neverResolveStop) {
      void signal;
      return await new Promise<NativeStopProof>(() => undefined);
    }
    return {
      ...structuredClone(prepared),
      parentExited: true,
      activeJobProcesses: 0,
      identityStatus: "exact",
      processHandlePresent: true,
      jobHandlePresent: true,
      killAttempted: false,
      killSucceeded: false,
      writerFenceVerified: true,
      exitCode: 0,
      deadlineExceeded: false,
      errors: [],
      ...structuredClone(this.stopPatch),
    };
  }

  async confirmStopped(confirmation: NativeStopConfirmation, _signal: AbortSignal): Promise<void> {
    const prepared = this.prepared.get(confirmation.ticket);
    assert.ok(prepared);
    assert.deepEqual(confirmation.identity, prepared.identity);
    assert.match(confirmation.proofHash, /^sha256:[0-9a-f]{64}$/);
    assert.ok(confirmation.durableRevision > 0);
    this.confirmCount += 1;
    this.prepared.delete(confirmation.ticket);
  }
}

class MemoryStore implements DurableCustodyStore {
  readonly rows = new Map<string, CustodyRecord>();
  readonly writerReservations = new Map<string, string>();
  readonly order: string[] = [];
  corruptSavedProof: "null" | "mismatch" | null = null;

  async reservePrepared(
    prepared: PreparedCustody,
    writerDomain: string,
  ): Promise<ReservePreparedResult> {
    const existing = this.writerReservations.get(writerDomain);
    if (existing !== undefined) return { status: "writer-conflict" };
    const record: CustodyRecord = {
      ...structuredClone(prepared),
      writerDomain,
      phase: "PREPARED",
      revision: 1,
      stopAttempt: 0,
      leaseState: "held",
      stopProof: null,
      errors: [],
    };
    this.writerReservations.set(writerDomain, record.ticket);
    this.rows.set(record.ticket, structuredClone(record));
    this.order.push(`reserve:${record.ticket}`);
    return { status: "reserved", record: structuredClone(record) };
  }

  async read(ticket: string): Promise<CustodyRecord | null> {
    return structuredClone(this.rows.get(ticket) ?? null);
  }

  async compareAndSet(record: CustodyRecord, expectedRevision: number): Promise<CustodyRecord> {
    const current = this.rows.get(record.ticket);
    assert.equal(current?.revision, expectedRevision, "durable revision CAS");
    const copy = structuredClone(record);
    this.rows.set(copy.ticket, copy);
    if (copy.phase === "CONFIRMED") this.writerReservations.delete(copy.writerDomain);
    this.order.push(`cas:${copy.phase}:${copy.ticket}`);
    return structuredClone(copy);
  }

  async saveStoppedProof(
    ticket: string,
    expectedRevision: number,
    proof: NativeStopProof,
  ): Promise<CustodyRecord> {
    const current = this.rows.get(ticket);
    assert.ok(current);
    assert.equal(current.revision, expectedRevision);
    let savedProof: NativeStopProof | null = structuredClone(proof);
    if (this.corruptSavedProof === "null") savedProof = null;
    if (this.corruptSavedProof === "mismatch") {
      savedProof = { ...structuredClone(proof), custodianNonce: `pcn1_${"9".repeat(64)}` };
    }
    const stopped: CustodyRecord = {
      ...structuredClone(current),
      phase: "STOPPED_PENDING_CONFIRM",
      revision: current.revision + 1,
      stopProof: savedProof,
    };
    this.rows.set(ticket, structuredClone(stopped));
    this.order.push(`save-stopped:${ticket}`);
    return structuredClone(stopped);
  }
}

test("service owns trusted adapters and reserves PREPARED before activation", async () => {
  const native = new MemoryNative();
  const store = new MemoryStore();
  const service = new CustodyService(native, store);
  const input = launchRequest();
  const launched = service.launch(input);
  input.binding.profileId = "mutated-after-call";
  input.arguments[0] = "mutated-after-call";
  const active = await launched;
  assert.equal(active.binding.profileId, "profile-a");
  assert.equal(active.phase, "ACTIVE");
  assert.equal(native.activateCount, 1);
  assert.deepEqual(store.order.slice(0, 2), [
    `reserve:${active.ticket}`,
    `cas:ACTIVE:${active.ticket}`,
  ]);
  assert.ok(
    native.order.indexOf(`activate:${active.ticket}`) >
      native.order.indexOf(`prepare:${active.ticket}`),
  );
  assert.ok(Object.isFrozen(active));
  assert.ok(Object.isFrozen(active.binding));
  assert.ok(Object.isFrozen(active.identity));
});

test("concurrent same-domain launches atomically choose one writer and abort the loser", async () => {
  const native = new MemoryNative();
  const store = new MemoryStore();
  const service = new CustodyService(native, store);
  const outcomes = await Promise.allSettled([
    service.launch(launchRequest()),
    service.launch(launchRequest()),
  ]);
  assert.equal(outcomes.filter((outcome) => outcome.status === "fulfilled").length, 1);
  assert.equal(outcomes.filter((outcome) => outcome.status === "rejected").length, 1);
  assert.equal(native.prepareCount, 2);
  assert.equal(native.activateCount, 1);
  assert.equal(native.abortCount, 1);
  assert.equal(store.writerReservations.size, 1);
});

test("caller data cannot inject callbacks, handles, adapters, or StopProof", async () => {
  const service = new CustodyService(new MemoryNative(), new MemoryStore());
  await assert.rejects(
    service.launch({ ...launchRequest(), nativeStop: () => undefined }),
    /OBJECT_FIELDS_INVALID/,
  );
  await assert.rejects(
    service.launch({ ...launchRequest(), processHandleRef: "caller-handle" }),
    /OBJECT_FIELDS_INVALID/,
  );
});

test("service binds trusted adapter methods once and ignores later method replacement", async () => {
  const native = new MemoryNative();
  const store = new MemoryStore();
  const service = new CustodyService(native, store);
  const active = await service.launch(launchRequest());
  native.stop = async () => {
    throw new Error("replacement must not become authority");
  };
  const confirmed = await service.stop(active.ticket);
  assert.equal(confirmed.phase, "CONFIRMED");
  assert.equal(native.confirmCount, 1);
});

test("passive snapshots reject Proxy, accessor, symbol, extra, and nonplain input before reads", async () => {
  const service = new CustodyService(new MemoryNative(), new MemoryStore());
  let traps = 0;
  const proxy = new Proxy(launchRequest(), {
    ownKeys(target) {
      traps += 1;
      return Reflect.ownKeys(target);
    },
    get(target, property, receiver) {
      traps += 1;
      return Reflect.get(target, property, receiver);
    },
  });
  await assert.rejects(service.launch(proxy), /PASSIVE_PLAIN_OBJECT_REQUIRED/);
  assert.equal(traps, 0);

  let getterReads = 0;
  const accessor = launchRequest() as Record<string, unknown>;
  Object.defineProperty(accessor, "writerDomain", {
    enumerable: true,
    get() {
      getterReads += 1;
      return "writer-a";
    },
  });
  await assert.rejects(service.launch(accessor), /ACCESSOR_FIELDS_FORBIDDEN/);
  assert.equal(getterReads, 0);

  const symbol = launchRequest() as Record<PropertyKey, unknown>;
  symbol[Symbol("hidden")] = true;
  await assert.rejects(service.launch(symbol), /SYMBOL_FIELDS_FORBIDDEN/);
  await assert.rejects(
    service.launch({ ...launchRequest(), extra: true }),
    /OBJECT_FIELDS_INVALID/,
  );
  await assert.rejects(service.launch(Object.create(launchRequest())), /PLAIN_OBJECT_REQUIRED/);
});

test("exact native proof is durably saved before native confirmation and writer release", async () => {
  const native = new MemoryNative();
  const store = new MemoryStore();
  const service = new CustodyService(native, store);
  const active = await service.launch(launchRequest());
  const confirmed = await service.stop(active.ticket);
  assert.equal(confirmed.phase, "CONFIRMED");
  assert.ok(confirmed.stopProof);
  assert.equal(stopProofErrors(confirmed.stopProof).length, 0);
  assert.equal(native.confirmCount, 1);
  assert.equal(store.writerReservations.size, 0);
  assert.deepEqual(store.order.slice(-3), [
    `cas:STOPPING:${active.ticket}`,
    `save-stopped:${active.ticket}`,
    `cas:CONFIRMED:${active.ticket}`,
  ]);
  assert.match(hashStopProof(confirmed.stopProof), /^sha256:[0-9a-f]{64}$/);
});

test("null or mismatched stored STOPPED proof never confirms or releases", async (t) => {
  for (const corruption of ["null", "mismatch"] as const) {
    await t.test(corruption, async () => {
      const native = new MemoryNative();
      const store = new MemoryStore();
      store.corruptSavedProof = corruption;
      const service = new CustodyService(native, store);
      const active = await service.launch(launchRequest());
      if (corruption === "null") {
        const result = await service.stop(active.ticket);
        assert.equal(result.phase, "RESIDUAL");
      } else {
        await assert.rejects(service.stop(active.ticket), /BINDING_MISMATCH|PROOF_MISMATCH/);
      }
      assert.equal(native.confirmCount, 0);
      assert.equal(store.writerReservations.size, 1);
    });
  }
});

test("never-resolving native stop returns within 75ms/1ms deadlines and retains custody", async (t) => {
  for (const hostDeadlineMs of [75, 1]) {
    await t.test(`${hostDeadlineMs}ms`, async () => {
      const native = new MemoryNative();
      const store = new MemoryStore();
      const service = new CustodyService(native, store);
      const active = await service.launch(launchRequest());
      native.neverResolveStop = true;
      const started = performance.now();
      const result = await service.stop(active.ticket, {
        graceMs: hostDeadlineMs,
        terminateMs: 0,
        observeMs: 0,
        hostDeadlineMs,
      });
      assert.ok(performance.now() - started < hostDeadlineMs + 100);
      assert.equal(result.phase, "RESIDUAL");
      assert.match(result.errors.join(" "), /HOST_STOP_DEADLINE_EXCEEDED/);
      assert.equal(native.confirmCount, 0);
      assert.equal(store.writerReservations.size, 1);
    });
  }
});

test("lease loss is not stop and blocks a new writer", async () => {
  const native = new MemoryNative();
  const store = new MemoryStore();
  const service = new CustodyService(native, store);
  const active = await service.launch(launchRequest());
  const lost = await service.markLeaseLost(active.ticket);
  assert.equal(lost.phase, "RESIDUAL");
  assert.equal(lost.leaseState, "lost");
  await assert.rejects(service.launch(launchRequest()), /WRITER_DOMAIN_ALREADY_RESERVED/);
  assert.equal(native.abortCount, 1);
});

test("124/125, unknown identity, missing handles, descendants, and kill failure retain custody", async (t) => {
  const cases: ReadonlyArray<readonly [string, Partial<NativeStopProof>]> = [
    ["exit124", { exitCode: STOP_TIMEOUT_EXIT_CODE }],
    ["exit125", { exitCode: STOP_REFUSED_EXIT_CODE }],
    ["unknown", { identityStatus: "unknown" }],
    ["process handle", { processHandlePresent: false }],
    ["job handle", { jobHandlePresent: false }],
    ["descendant", { activeJobProcesses: 1 }],
    ["kill failure", { killAttempted: true, killSucceeded: false }],
  ];
  for (const [name, patch] of cases) {
    await t.test(name, async () => {
      const native = new MemoryNative();
      native.stopPatch = patch;
      const store = new MemoryStore();
      const service = new CustodyService(native, store);
      const active = await service.launch(launchRequest());
      const result = await service.stop(active.ticket);
      assert.equal(result.phase, "RESIDUAL");
      assert.equal(native.confirmCount, 0);
      assert.equal(store.writerReservations.size, 1);
    });
  }
});

test("production budgets remain 10/5/5 inside 30 seconds", () => {
  assert.deepEqual(PRODUCTION_STOP_BUDGETS, {
    graceMs: 10_000,
    terminateMs: 5_000,
    observeMs: 5_000,
    hostDeadlineMs: 30_000,
  });
  assert.equal(
    hashStopProof({
      ticket: `pct1_${"a".repeat(64)}`,
      custodianNonce: `pcn1_${"b".repeat(64)}`,
      binding: {
        binaryDigestSha256: `sha256:${"c".repeat(64)}`,
        profileId: "profile",
        domainId: "domain",
        generation: "7",
      },
      identity: {
        pid: 42,
        creationTime100ns: "99",
        imagePath: String.raw`C:\fixture\fake.exe`,
      },
      parentExited: true,
      activeJobProcesses: 0,
      identityStatus: "exact",
      processHandlePresent: true,
      jobHandlePresent: true,
      killAttempted: false,
      killSucceeded: false,
      writerFenceVerified: true,
      exitCode: 0,
      deadlineExceeded: false,
      errors: [],
    }),
    "sha256:6de76f072fd07f424de91943871f249eb3bdc2f22b7b33dcf302cd14fdd38e7e",
  );
});
