import { describe, expect, it } from "vite-plus/test";
import * as NodeFS from "node:fs/promises";
import * as NodeOS from "node:os";
import * as NodePath from "node:path";

import {
  createR2GhCredentialAccess,
  currentGhToken,
  currentGhTokenIfAvailable,
  GhCredentialError,
  resolveTrustedGhExecutable,
} from "./ghCredential.ts";

describe("R2-02 existing gh login adapter", () => {
  it("checks native admission and exact Owner login before a test ledger write", async () => {
    const calls: string[] = [];
    const access = createR2GhCredentialAccess(
      async () => {
        calls.push("native-admission");
      },
      (args) => {
        calls.push(args.join(" "));
        return args[0] === "api" ? "taiyun668" : "synthetic-token-with-enough-length";
      },
    );
    await access.assertCurrentAuthority();
    expect(await access.credential()).toBe("synthetic-token-with-enough-length");
    expect(calls).toEqual([
      "native-admission",
      "api user --hostname github.com --jq .login",
      "auth token --hostname github.com",
    ]);
  });

  it("rejects a different active account before obtaining any token", async () => {
    const calls: string[] = [];
    const access = createR2GhCredentialAccess(
      async () => {},
      (args) => {
        calls.push(args[0]!);
        return "another-account";
      },
    );
    await expect(access.assertCurrentAuthority()).rejects.toBeInstanceOf(GhCredentialError);
    expect(calls).toEqual(["api"]);
  });

  it("does not return an empty or multiline credential", async () => {
    for (const value of ["", "synthetic-token-with-enough-length\nsecond"]) {
      const access = createR2GhCredentialAccess(
        async () => {},
        () => value,
      );
      await expect(access.credential()).rejects.toMatchObject({ code: "GH_AUTH_UNAVAILABLE" });
    }
  });

  it("allows public reads without gh login while keeping strict credential access", () => {
    const unavailable = () => {
      throw new GhCredentialError("GH_AUTH_UNAVAILABLE");
    };
    expect(currentGhTokenIfAvailable(unavailable)).toBeUndefined();
    expect(() => currentGhToken(unavailable)).toThrow("GH_AUTH_UNAVAILABLE");
  });

  it("resolves gh only from a trusted absolute path outside cwd and product install", async () => {
    const root = await NodeFS.mkdtemp(NodePath.join(NodeOS.tmpdir(), "gogoke-gh-path-"));
    try {
      const programFiles = NodePath.join(root, "Program Files");
      const cwd = NodePath.join(programFiles, "product-cwd");
      const install = NodePath.join(programFiles, "product-install");
      const userBin = NodePath.join(root, "user-bin");
      const trustedBin = NodePath.join(programFiles, "GitHub CLI");
      for (const directory of [cwd, install, userBin, trustedBin]) {
        await NodeFS.mkdir(directory, { recursive: true });
        await NodeFS.writeFile(NodePath.join(directory, "gh.exe"), "fixture");
      }
      const resolved = resolveTrustedGhExecutable(
        "win32",
        {
          PATH: [cwd, install, userBin, trustedBin].join(";"),
          ProgramFiles: programFiles,
        },
        cwd,
        NodePath.join(install, "node.exe"),
      );
      expect(NodePath.win32.isAbsolute(resolved)).toBe(true);
      expect(NodePath.win32.normalize(resolved)).toBe(
        NodePath.win32.normalize(NodePath.join(trustedBin, "gh.exe")),
      );
    } finally {
      await NodeFS.rm(root, { recursive: true, force: true });
    }
  });
});
