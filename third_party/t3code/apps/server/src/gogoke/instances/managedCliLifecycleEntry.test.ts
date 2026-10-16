// @effect-diagnostics nodeBuiltinImport:off - platform path shape only.
import * as NodePath from "node:path";
import { expect, it } from "@effect/vitest";
import { handleManagedCliOperation } from "../bootstrap/productEntry.ts";

it("preserves the two-key managed CLI stage request and uses its default catalog pin", async () => {
  const calls: unknown[][] = [];
  const response = await handleManagedCliOperation({
    operation: "managed-cli-stage", driverId: "opencode",
  }, "fixture-root", {
    stageDefault: async (root, driver) => {
      calls.push(["default", root, driver]);
      return { driver, version: "1.18.32", stagePath: "fixture-root/stage-32",
        imagePath: "fixture-root/stage-32/opencode.exe", archiveSha256: "a", imageSha256: "b" };
    },
    stagePinned: async () => { throw new Error("unexpected pinned stage"); },
  });
  expect(calls).toEqual([["default", NodePath.join("fixture-root", "v37-managed-cli", "staging"), "opencode"]]);
  expect(response).toEqual({ schema: "gogoke.37.managed-cli-stage.v1", driverId: "opencode",
    version: "1.18.32", stageName: "stage-32" });
});

it("stages only the selected trusted 1.18.31 or 1.18.32 catalog pin", async () => {
  for (const version of ["1.18.31", "1.18.32"]) {
    let selected: unknown[] | undefined;
    const response = await handleManagedCliOperation({
      operation: "managed-cli-stage", driverId: "opencode", targetVersion: version,
    }, "fixture-root", {
      stageDefault: async () => { throw new Error("unexpected default stage"); },
      stagePinned: async (root, driver, pin) => {
        selected = [root, driver, pin];
        return { driver, version: pin, stagePath: `fixture-root/stage-${pin}`,
          imagePath: `fixture-root/stage-${pin}/opencode.exe`, archiveSha256: "a", imageSha256: "b" };
      },
    });
    expect(selected).toEqual([
      NodePath.join("fixture-root", "v37-managed-cli", "staging"), "opencode", version,
    ]);
    expect(response).toMatchObject({ driverId: "opencode", version });
  }
});

it("rejects an unknown requested version and caller source fields before staging", async () => {
  let stageCalls = 0;
  const stage = async () => {
    stageCalls += 1;
    throw new Error("stage must not be reached");
  };
  await expect(handleManagedCliOperation({
    operation: "managed-cli-stage", driverId: "opencode", targetVersion: "9.9.9",
  }, "fixture-root", { stagePinned: stage })).rejects.toThrow("MANAGED_CLI_DRIVER_OR_VERSION_UNSUPPORTED");
  for (const extra of [
    { url: "https://example.invalid/a.tgz" },
    { path: "C:\\caller.exe" },
    { sha256: "a".repeat(64) },
  ]) {
    await expect(handleManagedCliOperation({
      operation: "managed-cli-stage", driverId: "opencode", targetVersion: "1.18.31", ...extra,
    }, "fixture-root", { stagePinned: stage })).rejects.toThrow("INVALID_PRODUCT_ENTRY");
  }
  expect(stageCalls).toBe(0);
});

it("returns unverified official update metadata and time without staging or activation", async () => {
  const effects: string[] = [];
  const response = await handleManagedCliOperation({
    operation: "managed-cli-check-updates", driverId: "codex",
  }, "fixture-root", {
    check: async (driver) => { effects.push(`check:${driver}`); return "999.0.0"; },
    stageDefault: async () => { effects.push("stage-default"); throw new Error("unexpected stage"); },
    stagePinned: async () => { effects.push("stage-pinned"); throw new Error("unexpected stage"); },
    now: () => 123456789,
  });
  expect(response).toEqual({ schema: "gogoke.37.managed-cli-notice.v1", driverId: "codex",
    officialVersion: "999.0.0", checkedAt: "123456789" });
  expect(effects).toEqual(["check:codex"]);
  await expect(handleManagedCliOperation({
    operation: "managed-cli-check-updates", driverId: "antigravity",
  }, "fixture-root", { check: async () => { throw new Error("unsupported driver was queried"); } }))
    .rejects.toThrow("MANAGED_CLI_DRIVER_UNSUPPORTED");
});
