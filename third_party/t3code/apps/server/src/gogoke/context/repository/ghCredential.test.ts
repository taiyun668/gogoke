import { describe, expect, it } from "vite-plus/test";

import { createR2GhCredentialAccess, GhCredentialError } from "./ghCredential.ts";

describe("R2-02 existing gh login adapter", () => {
  it("checks native admission and exact Owner login before a test ledger write", async () => {
    const calls: string[] = [];
    const access = createR2GhCredentialAccess(
      async () => { calls.push("native-admission"); },
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
    const access = createR2GhCredentialAccess(async () => {}, (args) => {
      calls.push(args[0]!);
      return "another-account";
    });
    await expect(access.assertCurrentAuthority()).rejects.toBeInstanceOf(GhCredentialError);
    expect(calls).toEqual(["api"]);
  });

  it("does not return an empty or multiline credential", async () => {
    for (const value of ["", "synthetic-token-with-enough-length\nsecond"]) {
      const access = createR2GhCredentialAccess(async () => {}, () => value);
      await expect(access.credential()).rejects.toMatchObject({ code: "GH_AUTH_UNAVAILABLE" });
    }
  });
});
