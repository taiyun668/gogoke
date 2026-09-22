import { describe, expect, it } from "vite-plus/test";

import {
  BoundedEventDeliveryBuffer,
  EVENT_STREAM_SCHEMA,
  EpochRegistry,
  EventRecoveryError,
  MAX_FRAME_BYTES,
  MAX_OUTPUT_BYTES,
  MAX_OUTPUT_EVENTS,
  NativeFrameAccumulator,
  OrderedEventRecovery,
  type EventBinding,
  type EventBindingAuthority,
  type EventCursor,
  type EventPortWrite,
  type EventProjectionPort,
  type EventSnapshot,
  type EventSnapshotSource,
  type OutputDeliveryEvent,
  type SequencedEvent,
  type TerminalEvidence,
} from "./index.ts";

type FixtureState = { readonly value: string };
type FixturePayload = { readonly text: string };

const decoder = new TextDecoder();

const binding: EventBinding = {
  domainId: "domain-a",
  sourceEpoch: "7",
  generation: "11",
};

const event = (
  sequence: string,
  eventId = `event-${sequence}`,
  target = binding,
  eventType = "output.delta",
  payload: FixturePayload = { text: `payload-${sequence}` },
): SequencedEvent<FixturePayload> => ({
  schema: EVENT_STREAM_SCHEMA,
  eventId,
  eventType,
  binding: target,
  sequence,
  payload,
});

const cursorKey = (cursor: EventCursor): string =>
  JSON.stringify([cursor.domainId, cursor.sourceEpoch, cursor.generation, cursor.sequence]);

class FakeAuthority implements EventBindingAuthority {
  current: EventBinding | null = binding;
  readonly currentReads: Array<EventBinding | null> = [];
  readonly knownTypes = new Set(["output.delta", "turn.terminal"]);

  async currentBinding(domainId: string): Promise<EventBinding | null> {
    const selected =
      this.currentReads.length > 0 ? (this.currentReads.shift() ?? null) : this.current;
    return selected?.domainId === domainId ? selected : null;
  }

  async isKnownEventType(_binding: Readonly<EventBinding>, eventType: string): Promise<boolean> {
    return this.knownTypes.has(eventType);
  }
}

class FakeProjection implements EventProjectionPort<FixtureState, FixturePayload> {
  cursor: EventCursor | null = null;
  readonly eventIds = new Map<string, string>();
  readonly calls: string[] = [];
  readonly payloads: FixturePayload[] = [];
  nextAppend: EventPortWrite = "committed";
  nextReplace: EventPortWrite = "committed";

  async readCursor(): Promise<EventCursor | null> {
    return this.cursor;
  }

  async eventIdAt(cursor: EventCursor): Promise<string | null> {
    return this.eventIds.get(cursorKey(cursor)) ?? null;
  }

  async append(
    value: Readonly<SequencedEvent<FixturePayload>>,
    expectedCursor: Readonly<EventCursor> | null,
    expectedBinding: Readonly<EventBinding>,
  ): Promise<EventPortWrite> {
    this.calls.push(
      `append:${value.sequence}:${expectedCursor?.sequence ?? "null"}:${expectedBinding.sourceEpoch}:${expectedBinding.generation}`,
    );
    const result = this.nextAppend;
    this.nextAppend = "committed";
    if (result !== "committed") return result;
    this.cursor = { ...value.binding, sequence: value.sequence };
    this.eventIds.set(cursorKey(this.cursor), value.eventId);
    this.payloads.push(value.payload);
    return result;
  }

  async replaceSnapshot(
    snapshot: Readonly<EventSnapshot<FixtureState>>,
    expectedCursor: Readonly<EventCursor> | null,
    expectedBinding: Readonly<EventBinding>,
  ): Promise<EventPortWrite> {
    this.calls.push(
      `replace:${snapshot.cursor.sequence}:${expectedCursor?.sequence ?? "null"}:${expectedBinding.sourceEpoch}:${expectedBinding.generation}`,
    );
    const result = this.nextReplace;
    this.nextReplace = "committed";
    if (result !== "committed") return result;
    this.cursor = snapshot.cursor;
    return result;
  }
}

class FakeSnapshots implements EventSnapshotSource<FixtureState> {
  calls = 0;
  snapshot: EventSnapshot<FixtureState> = {
    cursor: { ...binding, sequence: "0" },
    state: { value: "empty" },
  };

  async loadSnapshot(): Promise<EventSnapshot<FixtureState>> {
    this.calls += 1;
    return this.snapshot;
  }
}

function fixture() {
  const authority = new FakeAuthority();
  const projection = new FakeProjection();
  const snapshots = new FakeSnapshots();
  const recovery = new OrderedEventRecovery(authority, projection, snapshots);
  return { authority, projection, snapshots, recovery };
}

function output(sequence: string, bytes = new Uint8Array()): OutputDeliveryEvent {
  return { kind: "output", ...binding, sequence, bytes };
}

function terminal(sequence: string, overrides: Partial<TerminalEvidence> = {}): TerminalEvidence {
  return {
    kind: "terminal",
    ...binding,
    sequence,
    status: "completed",
    eof: true,
    exitCode: 0,
    completionObserved: true,
    cancelled: false,
    partialFrame: null,
    nativeError: null,
    toolFailure: null,
    ...overrides,
  };
}

describe("S1-04-E ordered event recovery", () => {
  it("gogoke-s1-r4/T16.L applies once, identifies duplicates, and rejects late events", async () => {
    const { recovery, projection, snapshots } = fixture();
    await expect(recovery.ingest(event("1"))).resolves.toEqual({
      status: "applied",
      cursor: { ...binding, sequence: "1" },
      resynced: false,
    });
    await expect(recovery.ingest(event("1"))).resolves.toMatchObject({ status: "duplicate" });
    await expect(recovery.ingest(event("0", "late-zero"))).resolves.toMatchObject({
      status: "late",
    });
    expect(projection.calls).toEqual(["append:1:null:7:11"]);
    expect(snapshots.calls).toBe(0);
  });

  it("rejects unknown domains, unknown event types, and stale epoch/generation without writes", async () => {
    const { authority, recovery, projection } = fixture();
    await expect(
      recovery.ingest(event("1", "unknown-domain", { ...binding, domainId: "domain-x" })),
    ).resolves.toEqual({ status: "unknown", reason: "DOMAIN" });
    await expect(
      recovery.ingest(event("1", "unknown-type", binding, "vendor.magic")),
    ).resolves.toEqual({ status: "unknown", reason: "EVENT_TYPE" });
    await expect(
      recovery.ingest(event("1", "old-epoch", { ...binding, sourceEpoch: "6" })),
    ).resolves.toEqual({ status: "stale", current: binding });
    authority.current = { ...binding, generation: "12" };
    await expect(recovery.ingest(event("1", "old-generation"))).resolves.toEqual({
      status: "stale",
      current: { ...binding, generation: "12" },
    });
    expect(projection.calls).toEqual([]);
  });

  it("recovers a gap from an exact-domain snapshot and then appends the next event", async () => {
    const { recovery, projection, snapshots } = fixture();
    projection.cursor = { ...binding, sequence: "1" };
    snapshots.snapshot = {
      cursor: { ...binding, sequence: "2" },
      state: { value: "through-two" },
    };
    await expect(recovery.ingest(event("3"))).resolves.toEqual({
      status: "applied",
      cursor: { ...binding, sequence: "3" },
      resynced: true,
    });
    expect(projection.calls).toEqual(["replace:2:1:7:11", "append:3:2:7:11"]);
    expect(snapshots.calls).toBe(1);
  });

  it("reports out-of-order when the authoritative snapshot still leaves a gap", async () => {
    const { recovery, projection, snapshots } = fixture();
    projection.cursor = { ...binding, sequence: "1" };
    snapshots.snapshot = {
      cursor: { ...binding, sequence: "2" },
      state: { value: "through-two" },
    };
    await expect(recovery.ingest(event("5"))).resolves.toEqual({
      status: "out-of-order",
      cursor: { ...binding, sequence: "2" },
      expectedSequence: "3",
      receivedSequence: "5",
    });
    expect(projection.calls).toEqual(["replace:2:1:7:11"]);
  });

  it("does not append onto a cursor from an old epoch even when its number is adjacent", async () => {
    const { recovery, projection, snapshots } = fixture();
    projection.cursor = { ...binding, sourceEpoch: "6", sequence: "1" };
    snapshots.snapshot = {
      cursor: { ...binding, sequence: "1" },
      state: { value: "new-epoch-one" },
    };
    await expect(recovery.ingest(event("2"))).resolves.toMatchObject({
      status: "applied",
      resynced: true,
    });
    expect(projection.calls).toEqual(["replace:1:1:7:11", "append:2:1:7:11"]);
  });

  it("surfaces cursor conflicts and an atomic stale-authority rejection", async () => {
    const appendConflict = fixture();
    appendConflict.projection.nextAppend = "conflict";
    await expect(appendConflict.recovery.ingest(event("1"))).resolves.toEqual({
      status: "conflict",
      operation: "append",
    });

    const resyncConflict = fixture();
    resyncConflict.projection.cursor = { ...binding, sequence: "1" };
    resyncConflict.projection.nextReplace = "conflict";
    resyncConflict.snapshots.snapshot = {
      cursor: { ...binding, sequence: "2" },
      state: { value: "two" },
    };
    await expect(resyncConflict.recovery.ingest(event("3"))).resolves.toEqual({
      status: "conflict",
      operation: "resync",
    });

    const stale = fixture();
    stale.projection.nextAppend = "stale-authority";
    stale.authority.currentReads.push(binding, { ...binding, generation: "12" });
    await expect(stale.recovery.ingest(event("1"))).resolves.toEqual({
      status: "stale",
      current: { ...binding, generation: "12" },
    });
    expect(stale.projection.calls).toEqual(["append:1:null:7:11"]);
  });

  it("fails closed when a production port returns an unknown write result", async () => {
    const invalid = fixture();
    invalid.projection.nextAppend = "future-result" as EventPortWrite;
    await expect(invalid.recovery.ingest(event("1"))).rejects.toMatchObject({
      code: "INVALID_PORT_RESULT",
    });
  });

  it("rejects cross-domain and regressing recovery snapshots", async () => {
    const wrongDomain = fixture();
    wrongDomain.projection.cursor = { ...binding, sequence: "1" };
    wrongDomain.snapshots.snapshot = {
      cursor: { ...binding, domainId: "domain-b", sequence: "2" },
      state: { value: "wrong" },
    };
    await expect(wrongDomain.recovery.ingest(event("3"))).rejects.toMatchObject({
      code: "INVALID_SNAPSHOT",
    });

    const regression = fixture();
    regression.projection.cursor = { ...binding, sequence: "3" };
    regression.snapshots.snapshot = {
      cursor: { ...binding, sequence: "2" },
      state: { value: "old" },
    };
    const result = regression.recovery.ingest(event("5"));
    await expect(result).rejects.toBeInstanceOf(EventRecoveryError);
    await expect(result).rejects.toMatchObject({ code: "SNAPSHOT_REGRESSION" });
  });

  it("snapshots JSON payload before awaiting external authority", async () => {
    const { recovery, projection } = fixture();
    const payload = { text: "before" };
    const admitted = recovery.ingest(event("1", "event-1", binding, "output.delta", payload));
    payload.text = "after";
    await expect(admitted).resolves.toMatchObject({ status: "applied" });
    expect(projection.payloads).toEqual([{ text: "before" }]);
    expect(Object.isFrozen(projection.payloads[0])).toBe(true);
  });
});

describe("S1-04-E bounded persisted output and terminal delivery", () => {
  it("gogoke-s1-r4/T53.L bounds a slow consumer at 1024 events and never drops terminal", () => {
    const buffer = new BoundedEventDeliveryBuffer(binding);
    for (let index = 1; index <= MAX_OUTPUT_EVENTS; index += 1) {
      expect(buffer.offerPersistedOutput(output(String(index)))).toBe("accepted");
    }
    expect(buffer.offerPersistedOutput(output("1025"))).toBe("backpressure");
    expect(buffer.resyncRequired).toBe(true);

    const firstDrain = buffer.drain();
    expect(firstDrain).toHaveLength(MAX_OUTPUT_EVENTS + 1);
    expect(firstDrain.at(-1)).toMatchObject({
      kind: "gap",
      revision: "1",
      semantics: "cumulative-replacement",
      fromSequence: "1025",
      toSequence: "1025",
      droppedEvents: 1,
      reason: "event-limit",
      resyncRequired: true,
    });

    // Draining capacity is not a resync. Later output remains excluded from the lane.
    expect(buffer.offerPersistedOutput(output("1026"))).toBe("backpressure");
    expect(buffer.offerPersistedTerminal(terminal("1027"))).toBe("accepted");
    const finalDrain = buffer.drain();
    expect(finalDrain).toHaveLength(2);
    expect(finalDrain[0]).toMatchObject({
      kind: "gap",
      revision: "2",
      semantics: "cumulative-replacement",
      fromSequence: "1025",
      toSequence: "1026",
      droppedEvents: 2,
    });
    expect(finalDrain[1]).toMatchObject({ kind: "terminal", status: "completed" });
    expect(buffer.offerPersistedOutput(output("1028"))).toBe("closed");
  });

  it("bounds output at 8 MiB, reports exact dropped bytes, and snapshots caller buffers", () => {
    const buffer = new BoundedEventDeliveryBuffer(binding);
    const full = new Uint8Array(MAX_OUTPUT_BYTES);
    full[0] = 7;
    expect(buffer.offerPersistedOutput(output("1", full))).toBe("accepted");
    full[0] = 99;
    expect(buffer.offerPersistedOutput(output("2", Uint8Array.of(1, 2, 3)))).toBe("backpressure");
    expect(buffer.offerPersistedTerminal(terminal("3"))).toBe("accepted");
    const drained = buffer.drain();
    expect(drained[0]).toMatchObject({ kind: "output", sequence: "1" });
    expect((drained[0] as OutputDeliveryEvent).bytes[0]).toBe(7);
    expect(drained[1]).toMatchObject({
      kind: "gap",
      fromSequence: "2",
      toSequence: "2",
      droppedEvents: 1,
      droppedBytes: 3,
      reason: "byte-limit",
    });
    expect(drained[2]).toMatchObject({ kind: "terminal", sequence: "3" });
  });

  it("resumes only through an explicit durable exact-binding resync", () => {
    const buffer = new BoundedEventDeliveryBuffer(binding);
    expect(buffer.offerPersistedOutput(output("1", new Uint8Array(MAX_OUTPUT_BYTES)))).toBe(
      "accepted",
    );
    expect(buffer.offerPersistedOutput(output("2", Uint8Array.of(1)))).toBe("backpressure");
    expect(() => buffer.resumeAfterDurableResync({ ...binding, sequence: "1" })).toThrow(
      "durable resync cursor does not cover the delivery gap",
    );
    expect(buffer.resyncRequired).toBe(true);
    buffer.resumeAfterDurableResync({ ...binding, sequence: "2" });
    expect(buffer.resyncRequired).toBe(false);
    expect(buffer.offerPersistedOutput(output("3", Uint8Array.of(3)))).toBe("accepted");
    expect(buffer.drain()).toEqual([expect.objectContaining({ kind: "output", sequence: "3" })]);
    expect(() =>
      buffer.resumeAfterDurableResync({ ...binding, generation: "12", sequence: "2" }),
    ).toThrow("different domain, epoch, or generation");
  });

  it("keeps cumulative replacement revisions monotonic across durable resync", () => {
    const buffer = new BoundedEventDeliveryBuffer(binding);
    const full = new Uint8Array(MAX_OUTPUT_BYTES);
    expect(buffer.offerPersistedOutput(output("1", full))).toBe("accepted");
    expect(buffer.offerPersistedOutput(output("2", Uint8Array.of(2)))).toBe("backpressure");
    expect(buffer.drain().at(-1)).toMatchObject({
      kind: "gap",
      revision: "1",
      semantics: "cumulative-replacement",
    });
    expect(buffer.offerPersistedOutput(output("3", Uint8Array.of(3)))).toBe("backpressure");
    expect(buffer.drain()).toEqual([
      expect.objectContaining({
        kind: "gap",
        revision: "2",
        semantics: "cumulative-replacement",
      }),
    ]);

    buffer.resumeAfterDurableResync({ ...binding, sequence: "3" });
    expect(buffer.offerPersistedOutput(output("4", full))).toBe("accepted");
    expect(buffer.offerPersistedOutput(output("5", Uint8Array.of(5)))).toBe("backpressure");
    expect(buffer.drain().at(-1)).toMatchObject({
      kind: "gap",
      revision: "3",
      semantics: "cumulative-replacement",
    });
  });

  it("rejects a terminal label that is stronger than its persisted evidence", () => {
    const buffer = new BoundedEventDeliveryBuffer(binding);
    expect(() =>
      buffer.offerPersistedTerminal(terminal("1", { completionObserved: false })),
    ).toThrow("terminal.status must be unknown");
  });
});

describe("S1-04-E native EOF evidence", () => {
  it("gogoke-s1-r4/T18.L preserves complete and partial content without inferring completion", () => {
    const accumulator = new NativeFrameAccumulator(binding, "10");
    expect(accumulator.push(new TextEncoder().encode("alpha\nbe"))).toMatchObject([
      { kind: "output", sequence: "10", ...binding },
    ]);
    const second = accumulator.push(new TextEncoder().encode("ta\r\npartial"));
    expect(second).toHaveLength(1);
    expect(second[0]?.sequence).toBe("11");
    expect(decoder.decode(second[0]?.bytes)).toBe("beta");

    const finished = accumulator.finish({ exitCode: 0, completionObserved: false });
    expect(finished.partial).toMatchObject({ kind: "partial", sequence: "12", ...binding });
    expect(decoder.decode(finished.partial?.bytes)).toBe("partial");
    expect(finished.terminal).toMatchObject({
      sequence: "13",
      status: "unknown",
      exitCode: 0,
      completionObserved: false,
    });
    expect(decoder.decode(finished.terminal.partialFrame ?? undefined)).toBe("partial");
  });

  it("retains native/tool failure detail, cancellation, and requires explicit clean completion", () => {
    const failed = new NativeFrameAccumulator(binding, "1").finish({
      exitCode: 0,
      completionObserved: true,
      nativeError: "provider disconnected after stderr content",
      toolFailure: "tool result was rejected",
    });
    expect(failed.terminal).toMatchObject({
      status: "failed",
      nativeError: "provider disconnected after stderr content",
      toolFailure: "tool result was rejected",
    });

    const whitespace = new NativeFrameAccumulator(binding, "1").finish({
      exitCode: 1,
      completionObserved: false,
      nativeError: " native failure ",
    });
    expect(whitespace.terminal).toMatchObject({
      status: "failed",
      nativeError: " native failure ",
    });

    const cancelled = new NativeFrameAccumulator(binding, "1").finish({
      exitCode: null,
      completionObserved: false,
      cancelled: true,
      nativeError: "cancel transport closed",
    });
    expect(cancelled.terminal).toMatchObject({ status: "cancelled", cancelled: true });

    const completed = new NativeFrameAccumulator(binding, "1").finish({
      exitCode: 0,
      completionObserved: true,
    });
    expect(completed.terminal.status).toBe("completed");
    const eofOnly = new NativeFrameAccumulator(binding, "1").finish({
      exitCode: null,
      completionObserved: false,
    });
    expect(eofOnly.terminal.status).toBe("unknown");
  });

  it("enforces the 4 MiB native frame ceiling before emitting the frame", () => {
    const accumulator = new NativeFrameAccumulator(binding, "1");
    accumulator.push(new TextEncoder().encode("kept"));
    // The trailing LF is framing, so MAX_FRAME_BYTES + LF remains valid.
    const oversized = new Uint8Array(MAX_FRAME_BYTES + 2);
    oversized[oversized.byteLength - 1] = 0x0a;
    expect(() => accumulator.push(oversized)).toThrow("native frame exceeds 4 MiB");
    const finished = accumulator.finish({
      exitCode: null,
      completionObserved: false,
      nativeError: "native frame exceeds 4 MiB",
    });
    expect(decoder.decode(finished.partial?.bytes)).toBe("kept");
    expect(finished.terminal.partialFrame?.byteLength).toBe(4);
    expect(finished.terminal.status).toBe("failed");
  });
});

describe("S1-04-E epoch cleanup", () => {
  it("uses identity compare-and-clear so old epoch/generation cleanup cannot clear a replacement", () => {
    const registry = new EpochRegistry<{ readonly channel: string }>();
    const oldLease = registry.replace(binding, { channel: "old" });
    const replacement = registry.replace(
      { ...binding, sourceEpoch: "8", generation: "12" },
      { channel: "new" },
    );
    expect(oldLease.clear()).toBe(false);
    expect(registry.current("domain-a")).toEqual({
      binding: { ...binding, sourceEpoch: "8", generation: "12" },
      value: { channel: "new" },
    });
    expect(replacement.clear()).toBe(true);
    expect(registry.current("domain-a")).toBeNull();
  });
});
