import { createHash } from "node:crypto";
import { spawn } from "node:child_process";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vite-plus/test";

import { PiManagedSession } from "../adapters/pi/session.ts";
import type { PiSettledObservation } from "../adapters/pi/types.ts";
import type { GitFactReadback } from "../context/repository/gitFact.ts";
import { ControlledFixtureResultError, validateControlledFixtureResult } from "./controlledFixtureResult.ts";

const digest = (bytes: Uint8Array) => createHash("sha256").update(bytes).digest("hex");
const embedded = Buffer.from("public test-only bytes\n");
const material = {
  modelId: "test-model",
  relativePath: "models/test-only.bin",
  bytesBase64: embedded.toString("base64"),
  sha256: digest(embedded),
};
const sourceBytes = Buffer.from(JSON.stringify(material));
const source: GitFactReadback = {
  state: "COMMITTED_BYTES_VERIFIED_NOT_ADOPTED",
  coordinate: {
    repository: "taiyun668/gogoke",
    commit: "a".repeat(40),
    path: "apps/desktop/test-fixtures/s1-r4/sealing/model-asset.json",
    contentHash: `sha256:${digest(sourceBytes)}`,
  },
  gitBlob: createHash("sha1").update(`blob ${sourceBytes.length}\0`).update(sourceBytes).digest("hex"),
  bytes: sourceBytes,
};
const report = {
  schema: "gogoke.s1-r4.r2-02.fixture-report.v1",
  testOnly: true,
  source: {
    repository: source.coordinate.repository,
    commit: source.coordinate.commit,
    path: source.coordinate.path,
    sha256: digest(sourceBytes),
  },
  observed: {
    modelId: material.modelId,
    relativePath: material.relativePath,
    embeddedBytesSha256: digest(embedded),
  },
};
const observation: PiSettledObservation = {
  status: "protocol-settled-not-result",
  accepted: { status: "accepted", requestId: "gogoke-pi-1", command: "prompt" },
  untrustedFinalText: JSON.stringify(report),
};

describe("R2-02 controlled fixture Result boundary", () => {
  it("promotes only exact observed bytes to a test Result with no adoption claim", () => {
    const result = validateControlledFixtureResult(source, observation);
    expect(result.state).toBe("VALIDATED_TEST_RESULT_NOT_ADOPTED");
    expect(result.sourceCommit).toBe(source.coordinate.commit);
    expect(result.embeddedBytesSha256).toBe(material.sha256);
  });

  it("rejects missing final text and invented model output after protocol settlement", () => {
    for (const text of [null, JSON.stringify({ ...report,
      observed: { ...report.observed, embeddedBytesSha256: "0".repeat(64) } })]) {
      expect(() => validateControlledFixtureResult(source,
        { ...observation, untrustedFinalText: text })).toThrow(ControlledFixtureResultError);
    }
  });

  it("rejects a source from outside the authorized public fixture path", () => {
    expect(() => validateControlledFixtureResult({ ...source, coordinate: {
      ...source.coordinate, path: "private/context.json",
    } }, observation)).toThrow(ControlledFixtureResultError);
  });

  it("diagnoses the real signed-Node protocol process without claiming native custody", async () => {
    const fixture = new URL("../../../../../../../apps/desktop/test-fixtures/s1-r4/sealing/model-asset.json", import.meta.url);
    const executable = new URL("../../../../../../../apps/desktop/test-fixtures/s1-r4/ledger/controlled-pi.mjs", import.meta.url);
    const bytes = readFileSync(fixture);
    const publicSource: GitFactReadback = {
      state: "COMMITTED_BYTES_VERIFIED_NOT_ADOPTED",
      coordinate: { repository: "taiyun668/gogoke", commit: "f6a820dda05a3eac5c29be48c4149bff7e1c9598",
        path: "apps/desktop/test-fixtures/s1-r4/sealing/model-asset.json",
        contentHash: `sha256:${digest(bytes)}` },
      gitBlob: createHash("sha1").update(`blob ${bytes.length}\0`).update(bytes).digest("hex"),
      bytes,
    };
    const child = spawn(process.execPath, [fileURLToPath(executable)], { stdio: ["pipe", "pipe", "pipe"] });
    const session = new PiManagedSession({
      admission: { mode: "ordinary", protocolQualified: true,
        protectedDomainQualified: false, contextExposure: "UNKNOWN" },
      sink: { write: (chunk) => new Promise<void>((resolve, reject) =>
        child.stdin.write(chunk, (error) => error ? reject(error) : resolve())) },
    });
    child.stdout.on("data", (chunk: Buffer) => session.acceptStdout(chunk));
    child.once("exit", () => session.close("fixture process exited"));
    try {
      const observed = await session.promptAndObserveSettlement(JSON.stringify({
        schema: "gogoke.s1-r4.r2-02.fixture-task.v1", testOnly: true,
        source: { repository: publicSource.coordinate.repository, commit: publicSource.coordinate.commit,
          path: publicSource.coordinate.path, sha256: digest(bytes), content: bytes.toString("utf8") },
      }), 5_000);
      expect(validateControlledFixtureResult(publicSource, observed).state)
        .toBe("VALIDATED_TEST_RESULT_NOT_ADOPTED");
    } finally {
      child.stdin.end();
      child.kill();
    }
  });
});
