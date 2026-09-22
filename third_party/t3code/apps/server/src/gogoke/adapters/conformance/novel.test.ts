import * as NodeFS from "node:fs";
import { describe, expect, it } from "vite-plus/test";

import { parseAdapterManifest, RuntimeCatalogError } from "../../runtimeCatalog/index.ts";
import { runNovelDriverConformance } from "./novel.ts";

const digest = `sha256:${"c".repeat(64)}`;

describe("R4-O-NOVEL conformance", () => {
  it("gogoke-s1-r4/R4-02 selects a post-build random id and preserves opaque config", () => {
    let calls = 0;
    const receipt = runNovelDriverConformance(digest, (size) => {
      calls += 1;
      expect(size).toBe(8);
      return Uint8Array.from({ length: size }, (_unused, index) => index + 1);
    });
    expect(calls).toBe(1);
    expect(receipt.driverId).toBe("mock_novel_0102030405060708");
    expect(receipt.initialPhase).toBe("ready");
    expect(receipt.switchingPhase).toBe("draining");
    expect(receipt.switchedVersion).toBe("2.0.0");
    expect(receipt.unavailableReason).toBe("DRIVER_NOT_REGISTERED");
    expect(receipt.reinstalled).toBe(true);
    expect(receipt.encodedConfig).toEqual({
      schema: "gogoke.runtime-instance-config.v1",
      instanceId: "mock_novel_0102030405060708_instance",
      driverId: "mock_novel_0102030405060708",
      adapterVersion: "1.0.0",
      enabled: true,
      config: { executable: "fixture.exe", opaque: { retain: true } },
      futureConfigRevision: "retained",
    });
    expect(Object.isFrozen(receipt)).toBe(true);
    expect(Object.isFrozen(receipt.encodedConfig)).toBe(true);
  });

  it("gogoke-s1-r4/R4-01 rejects an unknown manifest major", () => {
    expect(() =>
      parseAdapterManifest({
        schema: "gogoke.adapter-manifest.v2",
        packageId: "fixture.unknown",
      }),
    ).toThrow(RuntimeCatalogError);
  });

  it("gogoke-s1-r4/R4-02 rejects invalid build identity and entropy", () => {
    expect(() => runNovelDriverConformance("sha256:bad", () => new Uint8Array(8))).toThrow(
      "INVALID_CORE_BUILD_DIGEST",
    );
    expect(() => runNovelDriverConformance(digest, () => new Uint8Array(7))).toThrow(
      "INVALID_NOVEL_DRIVER_ENTROPY",
    );
  });

  it("contains no persistence, migration, provider brand or fixed generated id branch", () => {
    const source = NodeFS.readFileSync(new URL("./novel.ts", import.meta.url), "utf8");
    for (const forbidden of [
      "migrations/",
      "persistence/",
      "codex",
      "claude",
      "grok",
      "opencode",
      "antigravity",
      "mock_novel_0102030405060708",
    ]) {
      expect(source.toLowerCase()).not.toContain(forbidden);
    }
  });
});
