import { createHash } from "node:crypto";

import { parseStrictJsonBytes } from "../contracts/strictJson.ts";
import type { GitFactReadback } from "../context/repository/gitFact.ts";
import type { PiSettledObservation } from "../adapters/pi/types.ts";

const TEST_SOURCE_PATH = "apps/desktop/test-fixtures/s1-r4/sealing/model-asset.json";
const SHA = /^[0-9a-f]{64}$/u;
const sha256 = (bytes: Uint8Array): string =>
  createHash("sha256").update(bytes).digest("hex");

export class ControlledFixtureResultError extends Error {
  override readonly name = "ControlledFixtureResultError";
  readonly code: string;
  constructor(code: string) { super(code); this.code = code; }
}

function invalid(): never { throw new ControlledFixtureResultError("INVALID_CONTROLLED_FIXTURE_RESULT"); }

function exact(value: unknown, keys: readonly string[]): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return invalid();
  const record = value as Record<string, unknown>;
  if (Object.keys(record).length !== keys.length || keys.some((key) => !Object.hasOwn(record, key))) {
    return invalid();
  }
  return record;
}

export interface ValidatedControlledFixtureResult {
  readonly state: "VALIDATED_TEST_RESULT_NOT_ADOPTED";
  readonly sourceCommit: string;
  readonly sourceBlob: string;
  readonly reportSha256: string;
  readonly modelId: string;
  readonly relativePath: string;
  readonly embeddedBytesSha256: string;
}

/** Model text is untrusted until compared with the exact Git source bytes. */
export function validateControlledFixtureResult(
  source: GitFactReadback,
  observation: PiSettledObservation,
): ValidatedControlledFixtureResult {
  if (source.state !== "COMMITTED_BYTES_VERIFIED_NOT_ADOPTED" ||
      source.coordinate.repository !== "taiyun668/gogoke" ||
      source.coordinate.path !== TEST_SOURCE_PATH ||
      source.coordinate.contentHash.replace(/^sha256:/u, "") !== sha256(source.bytes) ||
      source.gitBlob !== createHash("sha1").update(`blob ${source.bytes.length}\0`).update(source.bytes).digest("hex") ||
      observation.status !== "protocol-settled-not-result" ||
      observation.accepted.status !== "accepted" ||
      observation.untrustedFinalText === null ||
      Buffer.byteLength(observation.untrustedFinalText, "utf8") > 32 * 1024) return invalid();

  let material: Record<string, unknown>;
  let report: Record<string, unknown>;
  try {
    material = exact(parseStrictJsonBytes(source.bytes),
      ["modelId", "relativePath", "bytesBase64", "sha256"]);
    report = exact(parseStrictJsonBytes(Buffer.from(observation.untrustedFinalText, "utf8")),
      ["schema", "testOnly", "source", "observed"]);
  } catch { return invalid(); }
  const reference = exact(report.source, ["repository", "commit", "path", "sha256"]);
  const observed = exact(report.observed, ["modelId", "relativePath", "embeddedBytesSha256"]);
  if (report.schema !== "gogoke.s1-r4.r2-02.fixture-report.v1" || report.testOnly !== true ||
      reference.repository !== source.coordinate.repository ||
      reference.commit !== source.coordinate.commit || reference.path !== source.coordinate.path ||
      reference.sha256 !== source.coordinate.contentHash.replace(/^sha256:/u, "") ||
      typeof material.modelId !== "string" || typeof material.relativePath !== "string" ||
      typeof material.bytesBase64 !== "string" || typeof material.sha256 !== "string" ||
      typeof observed.modelId !== "string" || typeof observed.relativePath !== "string" ||
      typeof observed.embeddedBytesSha256 !== "string" || !SHA.test(observed.embeddedBytesSha256)) {
    return invalid();
  }
  const embedded = Buffer.from(material.bytesBase64, "base64");
  const embeddedHash = sha256(embedded);
  if (embeddedHash !== material.sha256 || observed.modelId !== material.modelId ||
      observed.relativePath !== material.relativePath ||
      observed.embeddedBytesSha256 !== embeddedHash) return invalid();
  return Object.freeze({
    state: "VALIDATED_TEST_RESULT_NOT_ADOPTED" as const,
    sourceCommit: source.coordinate.commit,
    sourceBlob: source.gitBlob,
    reportSha256: sha256(Buffer.from(observation.untrustedFinalText, "utf8")),
    modelId: observed.modelId,
    relativePath: observed.relativePath,
    embeddedBytesSha256: embeddedHash,
  });
}
