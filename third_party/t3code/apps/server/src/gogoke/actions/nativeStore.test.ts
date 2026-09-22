import { describe, expect, it } from "vite-plus/test";

import {
  decodeActionReservationReply,
  decodeActionBeginReply,
  encodeActionBeginFrame,
  encodeActionOutcomeFrame,
  encodeActionReservationFrame,
  NativeHostClientError,
} from "../persistence/base/nativeHostClient.ts";
import type { DurableActionReservation } from "./typedAction.ts";

const reservation: DurableActionReservation = {
  schema: "gogoke.typed-action.v1",
  operationId: `opr_${"1".repeat(32)}`,
  semanticDigest: `sha256:${"a".repeat(64)}`,
  binding: {
    bindingId: "binding-main",
    sessionId: "session-main",
    executionId: "execution-main",
    runtimeInstanceId: "runtime-main",
    profileId: "profile-main",
    authRevision: "7",
    generation: "11",
  },
  commitment: {
    packageDigest:`sha256:${"b".repeat(64)}`,parentGrantRef:"grant-main",parentGrantRevision:"3",
    parentCeilingDigest:`sha256:${"c".repeat(64)}`,childCeilingDigest:`sha256:${"d".repeat(64)}`,
    sourcePrincipalId:"principal-source",sourceProjectId:"project-main",sourceDomainId:"domain-source",sourceRole:"controller",
    targetPrincipalId:"principal-target",targetProjectId:"project-main",targetDomainId:"domain-target",targetRole:"worker",
    sourceSessionId:"session-source",sourceExecutionId:"execution-source",sourceGeneration:"3",
    childSessionId:"session-main",childExecutionId:"execution-main",childGeneration:"11",
    route:"controller-worker",policyAction:"delegate",sink:"task-package",
    materialSetDigest:`sha256:${"e".repeat(64)}`,instructionDigest:`sha256:${"f".repeat(64)}`,
  },
  lane: "work",
  action: { kind: "queue", text: "hello" },
};

describe("native durable action store frames", () => {
  it("encodes one closed reservation without SQL or database path", () => {
    const frame = encodeActionReservationFrame(reservation);
    const decoded = JSON.parse(frame) as Record<string, unknown>;
    expect(decoded.operation).toBe("ReserveAction");
    expect(decoded.actionKind).toBe("queue");
    expect(decoded.payloadHex).toBe(Buffer.from('{"kind":"queue","text":"hello"}').toString("hex"));
    expect(frame.toLowerCase()).not.toContain("sql");
    expect(frame).not.toContain("databasePath");
  });

  it("decodes exact reserved, replay and conflict replies", () => {
    expect(
      decodeActionReservationReply(
        `{"kind":"reserved","operationId":"${reservation.operationId}","semanticDigest":"${reservation.semanticDigest}","reservationId":"reservation-1"}`,
      ),
    ).toMatchObject({ kind: "reserved", reservationId: "reservation-1" });
    expect(
      decodeActionReservationReply(
        `{"kind":"replay","operationId":"${reservation.operationId}","reservationId":"reservation-1","semanticDigest":"${reservation.semanticDigest}","state":"outcome-unknown"}`,
      ),
    ).toMatchObject({ kind: "replay", state: "outcome-unknown" });
    expect(
      decodeActionReservationReply(
        `{"kind":"conflict","operationId":"${reservation.operationId}","existingSemanticDigest":"sha256:${"b".repeat(64)}"}`,
      ),
    ).toMatchObject({ kind: "conflict" });
    expect(() => decodeActionReservationReply('{"kind":"reserved"}')).toThrow(
      NativeHostClientError,
    );
  });

  it("encodes and decodes the closed begin commitment without minting send authority on replay", () => {
    const frame=JSON.parse(encodeActionBeginFrame(reservation)) as Record<string,unknown>;
    expect(frame.operation).toBe("BeginActionCommitment");
    expect(frame.packageDigest).toBe(reservation.commitment.packageDigest);
    expect(decodeActionBeginReply(`{"kind":"granted","operationId":"${reservation.operationId}","reservationId":"reservation-${reservation.operationId}","sendAuthority":"send-one"}`)).toMatchObject({kind:"granted"});
    expect(decodeActionBeginReply(`{"kind":"replay","operationId":"${reservation.operationId}","reservationId":"reservation-${reservation.operationId}","state":"dispatching"}`)).toEqual({kind:"replay",operationId:reservation.operationId,reservationId:`reservation-${reservation.operationId}`,state:"dispatching"});
  });

  it("encodes durable unknown without turning it into retryable acceptance", () => {
    const frame = encodeActionOutcomeFrame(
      "reservation-1",
      reservation.operationId,
      reservation.semanticDigest,
      { kind: "outcome-unknown", reason: "EOF", detail: "lost reply" },
    );
    expect(JSON.parse(frame)).toEqual({
      detail: "EOF:lost reply",
      operation: "RecordActionOutcome",
      operationId: reservation.operationId,
      receiptRef: "",
      reservationId: "reservation-1",
      semanticDigest: reservation.semanticDigest,
      state: "outcome-unknown",
    });
  });
});
