// @effect-diagnostics nodeBuiltinImport:off - cloud integration fixture uses runner temp paths.
import * as NodeFS from "node:fs/promises";
import * as NodeOS from "node:os";
import * as NodePath from "node:path";

import { expect, it } from "@effect/vitest";
import { NativeHostClient } from "../persistence/base/nativeHostClient.ts";
import {
  decodeProductGoalRequest,
  handleProductGoalRequest,
  parseProductProcessArgs,
} from "./productEntry.ts";

const valid = JSON.stringify({
  goal: {
    id: "goal-r2-01",
    title: "Reach the native Product Authority from the Gogoke product entry",
  },
  ledger: {
    repository: "taiyun668/gogoke",
    commit: "6765d4e11ace61c47b9aeb123e0ef4770ab072c0",
    path: "apps/desktop/test-fixtures/s1-r4/ledger/r2-02-source-reference.json",
    contentHash: "sha256:b57db8a5fec4d9a4a09ca1e356c865017f88473916c5debeefa0ca2d87b08d08",
  },
});

it("admits a strict immutable test Goal ledger reference", () => {
  expect(decodeProductGoalRequest(Buffer.from(valid))).toEqual({
    goal: {
      id: "goal-r2-01",
      title: "Reach the native Product Authority from the Gogoke product entry",
    },
    ledger: {
      repository: "taiyun668/gogoke",
      commit: "6765d4e11ace61c47b9aeb123e0ef4770ab072c0",
      path: "apps/desktop/test-fixtures/s1-r4/ledger/r2-02-source-reference.json",
      contentHash: "sha256:b57db8a5fec4d9a4a09ca1e356c865017f88473916c5debeefa0ca2d87b08d08",
    },
  });
});

it("admits only an explicit true controlled test-task request", () => {
  expect(decodeProductGoalRequest(Buffer.from(valid.slice(0, -1) + ',"runControlledTask":true}'))
    .runControlledTask).toBe(true);
  expect(() => decodeProductGoalRequest(Buffer.from(
    valid.slice(0, -1) + ',"runControlledTask":false}'))).toThrow("INVALID_PRODUCT_ENTRY");
});

it("requires the controlled task for a test ledger draft", () => {
  expect(decodeProductGoalRequest(Buffer.from(
    valid.slice(0, -1) + ',"runControlledTask":true,"publishTestDraft":true}',
  ))).toMatchObject({ runControlledTask: true, publishTestDraft: true });
  expect(() => decodeProductGoalRequest(Buffer.from(
    valid.slice(0, -1) + ',"publishTestDraft":true}',
  ))).toThrow("INVALID_PRODUCT_ENTRY");
  expect(() => decodeProductGoalRequest(Buffer.from(
    valid.slice(0, -1) + ',"runControlledTask":true,"publishTestDraft":false}',
  ))).toThrow("INVALID_PRODUCT_ENTRY");
});

it("admits one post-build open fixture ID only on the test-draft path", () => {
  const suffix = ',"runControlledTask":true,"publishTestDraft":true,"fixtureDriverId":"mock_novel_0123456789abcdef"}';
  expect(decodeProductGoalRequest(Buffer.from(valid.slice(0, -1) + suffix)).fixtureDriverId)
    .toBe("mock_novel_0123456789abcdef");
  expect(() => decodeProductGoalRequest(Buffer.from(
    valid.slice(0, -1) + ',"runControlledTask":true,"fixtureDriverId":"mock_novel_0123456789abcdef"}',
  ))).toThrow("INVALID_PRODUCT_ENTRY");
});

it("rejects a test draft without a running build identity before native construction", async () => {
  const request = decodeProductGoalRequest(Buffer.from(
    valid.slice(0, -1) + ',"runControlledTask":true,"publishTestDraft":true}',
  ));
  await expect(handleProductGoalRequest(request, {
    root: "unused", hostBinary: "unused", executionEvidenceSha: "invalid",
    serviceEntrySha256: "invalid",
  })).rejects.toThrow("R2_TEST_DRAFT_BUILD_IDENTITY_UNAVAILABLE");
});

it("rejects mutable or path-escaping ledger coordinates before construction", () => {
  for (const value of [
    valid.replace("6765d4e11ace61c47b9aeb123e0ef4770ab072c0", "main"),
    valid.replace("apps/desktop/test-fixtures/s1-r4/ledger/r2-02-source-reference.json", "../r2-01.json"),
    valid.replace("sha256:b57db8a5fec4d9a4a09ca1e356c865017f88473916c5debeefa0ca2d87b08d08", "unknown"),
  ]) {
    expect(() => decodeProductGoalRequest(Buffer.from(value))).toThrow("INVALID_PRODUCT_ENTRY");
  }
});

it("seals donor server commands at the executable argument boundary", () => {
  expect(() => parseProductProcessArgs(["start"])).toThrow("INVALID_PRODUCT_ENTRY");
  expect(() => parseProductProcessArgs(["serve", "--port", "7740"])).toThrow(
    "INVALID_PRODUCT_ENTRY",
  );
  expect(
    parseProductProcessArgs([
      "--root",
      "C:\\Gogoke",
      "--native-host",
      "C:\\Gogoke\\gogoke-native-host.exe",
    ]),
  ).toEqual({
    root: "C:\\Gogoke",
    hostBinary: "C:\\Gogoke\\gogoke-native-host.exe",
  });
});


const nativeIntegrationTest =
  process.platform === "win32" && Boolean(process.env.GOGOKE_NATIVE_HOST) ? it : it.skip;

nativeIntegrationTest(
  "reaches the cloud-built native Product Authority through admitted Controller/Seat context",
  async () => {
    const hostBinary = process.env.GOGOKE_NATIVE_HOST;
    if (hostBinary === undefined) throw new Error("GOGOKE_NATIVE_HOST missing");
    const root = await NodeFS.mkdtemp(NodePath.join(NodeOS.tmpdir(), "gogoke-r2-entry-"));
    try {
      const response = await handleProductGoalRequest(decodeProductGoalRequest(Buffer.from(valid)), {
        root,
        hostBinary,
      });
      expect(response.goal.id).toBe("goal-r2-01");
      expect(response.ledger.repository).toBe("taiyun668/gogoke");
      expect(response.caller.admitted).toBe(true);
      expect(response.caller.role).toBe("controller");
      expect(response.nativeHost.reachable).toBe(true);
      expect(response.ledgerReadback.state).toBe("COMMITTED_BYTES_VERIFIED_NOT_ADOPTED");
      expect(response.ledgerReadback.gitBlob).toBe("a20115fdd5acf9e7e5025c3b3ca50696001badac");
      expect(response.acceptance).toBe("TEST_FIXTURE_NOT_ADOPTED");

      const inspector = await NativeHostClient.attach({ root, hostBinary });
      try {
        const snapshot = await inspector.readSnapshot(1);
        expect(snapshot.body).toBe('{"count":0}');
      } finally {
        await inspector.close();
      }
    } finally {
      await NodeFS.rm(root, { force: true, recursive: true });
    }
  },
);
