import type { JsonValue } from "../contracts/model.ts";
import {
  EventRecoveryError,
  type EventBinding,
  type EventBindingAuthority,
  type EventCursor,
  type EventIngestResult,
  type EventProjectionPort,
  type EventSnapshotSource,
  type SequencedEvent,
} from "./types.ts";
import {
  compareU64,
  cursorFor,
  incrementU64,
  sameBinding,
  snapshotBinding,
  snapshotCursor,
  snapshotEvent,
  snapshotRecoveryState,
} from "./validation.ts";

/**
 * Admits only events for the host's current binding. Gaps are recovered from a
 * binding-exact durable snapshot; received events never become authority.
 */
export class OrderedEventRecovery<
  State extends JsonValue = JsonValue,
  Payload extends JsonValue = JsonValue,
> {
  readonly #authority: EventBindingAuthority;
  readonly #projection: EventProjectionPort<State, Payload>;
  readonly #snapshots: EventSnapshotSource<State>;

  constructor(
    authority: EventBindingAuthority,
    projection: EventProjectionPort<State, Payload>,
    snapshots: EventSnapshotSource<State>,
  ) {
    this.#authority = authority;
    this.#projection = projection;
    this.#snapshots = snapshots;
  }

  async ingest(value: SequencedEvent<Payload>): Promise<EventIngestResult> {
    const event = snapshotEvent<Payload>(value);
    const current = await this.#currentBinding(event.binding.domainId);
    if (current === null) return Object.freeze({ status: "unknown", reason: "DOMAIN" });
    if (!sameBinding(event.binding, current)) {
      return Object.freeze({ status: "stale", current });
    }
    if ((await this.#authority.isKnownEventType(event.binding, event.eventType)) !== true) {
      return Object.freeze({ status: "unknown", reason: "EVENT_TYPE" });
    }

    const cursor = await this.#readCursor(event.binding.domainId);
    const initial = await this.#classifyAtCursor(event, cursor);
    if (initial !== null) return initial;

    const next = cursor === null ? "1" : incrementU64(cursor.sequence);
    if (next === event.sequence && (cursor === null || sameBinding(cursor, event.binding))) {
      return this.#append(event, cursor, false);
    }

    const rawSnapshot = await this.#snapshots.loadSnapshot(event.binding);
    const snapshot = snapshotRecoveryState<State>(rawSnapshot);
    if (!sameBinding(snapshot.cursor, event.binding)) {
      throw new EventRecoveryError(
        "INVALID_SNAPSHOT",
        "recovery snapshot binding does not match the current event binding",
      );
    }
    if (
      cursor !== null &&
      sameBinding(cursor, snapshot.cursor) &&
      compareU64(snapshot.cursor.sequence, cursor.sequence) < 0
    ) {
      throw new EventRecoveryError(
        "SNAPSHOT_REGRESSION",
        `snapshot ${snapshot.cursor.sequence} is behind cursor ${cursor.sequence}`,
      );
    }

    const replaced = await this.#projection.replaceSnapshot(snapshot, cursor, event.binding);
    if (replaced === "conflict") {
      return Object.freeze({ status: "conflict", operation: "resync" });
    }
    if (replaced === "stale-authority") {
      return this.#authorityChanged(event.binding.domainId);
    }
    if (replaced !== "committed") {
      throw new EventRecoveryError(
        "INVALID_PORT_RESULT",
        "replaceSnapshot returned an unknown result",
      );
    }

    const recoveredCursor = snapshot.cursor;
    const afterRecovery = await this.#classifyAtCursor(event, recoveredCursor);
    if (afterRecovery !== null) return afterRecovery;
    const recoveredNext = incrementU64(recoveredCursor.sequence);
    if (recoveredNext === event.sequence) return this.#append(event, recoveredCursor, true);
    return Object.freeze({
      status: "out-of-order",
      cursor: recoveredCursor,
      expectedSequence: recoveredNext ?? recoveredCursor.sequence,
      receivedSequence: event.sequence,
    });
  }

  async #currentBinding(domainId: string): Promise<EventBinding | null> {
    const raw = await this.#authority.currentBinding(domainId);
    return raw === null
      ? null
      : snapshotBinding(raw, "authority.binding", "INVALID_AUTHORITY_BINDING");
  }

  async #readCursor(domainId: string): Promise<EventCursor | null> {
    const raw = await this.#projection.readCursor(domainId);
    return raw === null ? null : snapshotCursor(raw);
  }

  async #classifyAtCursor(
    event: Readonly<SequencedEvent<Payload>>,
    cursor: Readonly<EventCursor> | null,
  ): Promise<EventIngestResult | null> {
    if (cursor === null || !sameBinding(cursor, event.binding)) return null;
    if (compareU64(event.sequence, cursor.sequence) > 0) return null;
    const existingEventId = await this.#projection.eventIdAt(cursorFor(event));
    if (existingEventId === event.eventId) {
      return Object.freeze({ status: "duplicate", cursor });
    }
    return Object.freeze({ status: "late", cursor });
  }

  async #append(
    event: Readonly<SequencedEvent<Payload>>,
    expectedCursor: Readonly<EventCursor> | null,
    resynced: boolean,
  ): Promise<EventIngestResult> {
    const written = await this.#projection.append(event, expectedCursor, event.binding);
    if (written === "conflict") {
      return Object.freeze({ status: "conflict", operation: "append" });
    }
    if (written === "stale-authority") {
      return this.#authorityChanged(event.binding.domainId);
    }
    if (written !== "committed") {
      throw new EventRecoveryError("INVALID_PORT_RESULT", "append returned an unknown result");
    }
    return Object.freeze({ status: "applied", cursor: cursorFor(event), resynced });
  }

  async #authorityChanged(domainId: string): Promise<EventIngestResult> {
    const current = await this.#currentBinding(domainId);
    return current === null
      ? Object.freeze({ status: "unknown" as const, reason: "DOMAIN" as const })
      : Object.freeze({ status: "stale" as const, current });
  }
}
