import { expect, it } from "@effect/vitest";
import { decodeProductGoalRequest, parseProductProcessArgs } from "./productEntry.ts";

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
