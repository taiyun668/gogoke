/** Test-fake request identity is the received wire bytes, not parsed JSON. */
export function rawV37RequestKey(bytes: Uint8Array): string {
  return Buffer.from(bytes).toString("base64");
}
