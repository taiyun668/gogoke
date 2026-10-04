export const DESIGN37_OWNER_NOTICES_PROJECTION = "OWNER_HOST_RULE_NOTICES";
const OPERATIONS_SCHEMA = "gogoke.37.operations.v1";

/** Native mechanical facts only; this projection is not an Owner delivery receipt. */
export type Design37OwnerNotice = {
  domainId: string;
  messageId: string;
  revision: string;
  state: "PENDING";
  sourceSeatId: string;
  causeEventId: string;
  triggerId: string;
  body: string;
};

export function buildDesign37OwnerNoticesRequest(requestId: string) {
  return {
    schema: OPERATIONS_SCHEMA,
    family: "K-INBOX" as const,
    operation: "check-unknown" as const,
    requestId,
    domainId: "global",
    targetId: "OWNER",
    expectedRevision: "0",
    payload: { projection: DESIGN37_OWNER_NOTICES_PROJECTION },
  };
}

export type Design37OwnerNoticesRequest = ReturnType<typeof buildDesign37OwnerNoticesRequest>;
export type Design37OwnerNoticesSourceOperation =
  (request: Design37OwnerNoticesRequest) => Promise<unknown>;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** Accept the original K-INBOX receipt, never a K-UI wrapper or a cached page. */
export function readDesign37OwnerNoticesReceipt(value: unknown, requestId?: string): Design37OwnerNotice[] {
  if (!isRecord(value) || value.schema !== OPERATIONS_SCHEMA ||
      value.family !== "K-INBOX" || value.operation !== "check-unknown" ||
      value.targetId !== "OWNER" || typeof value.requestId !== "string" ||
      (requestId !== undefined && value.requestId !== requestId) ||
      value.previousRevision !== "0" || value.revision !== "0") {
    throw new Error("OWNER_NOTICE_INVALID_K_INBOX_RECEIPT");
  }
  if (value.status !== "APPLIED") {
    throw new Error(`OWNER_NOTICE_RECEIPT: ${JSON.stringify(value)}`);
  }
  if (!isRecord(value.result) || value.result.projection !== DESIGN37_OWNER_NOTICES_PROJECTION ||
      !Array.isArray(value.result.notices)) {
    throw new Error("OWNER_NOTICE_INVALID_PROJECTION");
  }
  return value.result.notices.map((notice: unknown): Design37OwnerNotice => {
    if (!isRecord(notice) || notice.state !== "PENDING" ||
        ["domainId", "messageId", "revision", "sourceSeatId", "causeEventId", "triggerId", "body"].some(
          field => typeof notice[field] !== "string" || (notice[field] as string).length === 0)) {
      throw new Error("OWNER_NOTICE_INVALID_RECORD");
    }
    return {
      domainId: notice.domainId as string,
      messageId: notice.messageId as string,
      revision: notice.revision as string,
      state: "PENDING",
      sourceSeatId: notice.sourceSeatId as string,
      causeEventId: notice.causeEventId as string,
      triggerId: notice.triggerId as string,
      body: notice.body as string,
    };
  });
}

/** The composition root supplies its existing User source-operation bridge. */
export async function requestDesign37OwnerNotices(
  executeUserSourceOperation: Design37OwnerNoticesSourceOperation,
  requestId: string,
): Promise<Design37OwnerNotice[]> {
  const receipt = await executeUserSourceOperation(buildDesign37OwnerNoticesRequest(requestId));
  return readDesign37OwnerNoticesReceipt(receipt, requestId);
}

export function design37OwnerNoticeCauseKey(notice: Design37OwnerNotice): string {
  return JSON.stringify([notice.domainId, notice.causeEventId]);
}
