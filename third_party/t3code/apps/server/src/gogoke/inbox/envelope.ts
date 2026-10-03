/**
 * Adapted from the delimiter-fencing idea in claw-orchestrator's MIT inbox manager:
 * https://github.com/Enderfga/claw-orchestrator/blob/main/src/inbox-manager.ts
 * The source uses an in-memory queue; C keeps the queue in the native store.
 */
const invisible = "[\\p{Cc}\\p{Cf}\\p{Mn}\\p{Me}\\p{Default_Ignorable_Code_Point}]*";
const opening = new RegExp(`<(?=${invisible}/?${invisible}gogoke-inbox-message)`, "giu");

function attribute(value: string): string {
  return value.replaceAll("&", "&amp;").replaceAll('"', "&quot;")
    .replaceAll("<", "&lt;").replaceAll(">", "&gt;");
}

/** Sender identity must be supplied by C's native issuer, never by message text. */
export function wrapInboxMessage(senderSeatId: string, messageId: string, body: string): string {
  const fenced = body.replace(opening, "&lt;");
  return `<gogoke-inbox-message from-seat="${attribute(senderSeatId)}" message-id="${attribute(messageId)}">\n${fenced}\n</gogoke-inbox-message>`;
}
