// @effect-diagnostics nodeBuiltinImport:off - cloud integration fixture uses runner temp paths.
import * as NodeFS from "node:fs/promises";
import * as NodeOS from "node:os";
import * as NodePath from "node:path";

import { expect, it } from "@effect/vitest";
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
    repository: "fixture/authorized-project",
    commit: "a".repeat(40),
    path: "goals/r2-01.json",
    contentHash: "sha256:" + "b".repeat(64),
  },
});

it("admits a strict immutable test Goal ledger reference", () => {
  expect(decodeProductGoalRequest(Buffer.from(valid))).toEqual({
    goal: {
      id: "goal-r2-01",
      title: "Reach the native Product Authority from the Gogoke product entry",
    },
    ledger: {
      repository: "fixture/authorized-project",
      commit: "a".repeat(40),
      path: "goals/r2-01.json",
      contentHash: "sha256:" + "b".repeat(64),
    },
  });
});

it("rejects mutable or path-escaping ledger coordinates before construction", () => {
  for (const value of [
    valid.replace("a".repeat(40), "main"),
    valid.replace("goals/r2-01.json", "../r2-01.json"),
    valid.replace("sha256:" + "b".repeat(64), "unknown"),
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


it.runIf(process.platform === "win32" && Boolean(process.env.GOGOKE_NATIVE_HOST))(
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
      expect(response.ledger.repository).toBe("fixture/authorized-project");
      expect(response.caller.admitted).toBe(true);
      expect(response.caller.role).toBe("controller");
      expect(response.nativeHost.reachable).toBe(true);
      expect(response.acceptance).toBe("TEST_FIXTURE_NOT_ADOPTED");
    } finally {
      await NodeFS.rm(root, { force: true, recursive: true });
    }
  },
);
