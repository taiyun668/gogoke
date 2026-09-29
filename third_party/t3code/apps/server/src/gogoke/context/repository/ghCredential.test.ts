import { describe, expect, it } from "vite-plus/test";
import * as NodeFS from "node:fs";
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

  it("uses a fixed absolute CLI path and rejects the full packaged install root", () => {
    const gh = "C:\\Program Files\\GitHub CLI\\gh.exe";
    const fsApi = {
      realpathSync: (value: string) => value,
      statSync: (value: string) => {
        if (value !== gh) throw new Error("missing");
        return { isFile: () => true };
      },
    } as unknown as Pick<typeof NodeFS, "realpathSync" | "statSync">;
    const product = "C:\\Program Files\\Gogoke";
    const resolved = resolveTrustedGhExecutable(
      "win32",
      NodePath.win32.join(product, "gogoke-service"),
      NodePath.win32.join(product, "gogoke-service", "runtime", "node.exe"),
      fsApi,
    );
    expect(resolved).toBe(gh);
    expect(NodePath.win32.isAbsolute(resolved)).toBe(true);
    const conflictingInstall = "C:\\Program Files\\GitHub CLI";
    expect(() =>
      resolveTrustedGhExecutable(
        "win32",
        NodePath.win32.join(conflictingInstall, "gogoke-service"),
        NodePath.win32.join(conflictingInstall, "gogoke-service", "runtime", "node.exe"),
        fsApi,
      ),
    ).toThrow("GH_AUTH_UNAVAILABLE");
  });
});
