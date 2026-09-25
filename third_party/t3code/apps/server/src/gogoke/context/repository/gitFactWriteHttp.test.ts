import { describe, expect, it } from "vite-plus/test";

import { R2_TEST_LEDGER } from "./gitFactWrite.ts";
import { createR2TestGitHubWritePort, GitFactHttpError } from "./gitFactWriteHttp.ts";

const commit = "a".repeat(40);
const tree = "b".repeat(40);
const next = "c".repeat(40);
const requests: Array<{ method: string; url: string; body: Record<string, unknown> | null }> = [];

function fixture() {
  requests.length = 0;
  const fetcher: typeof fetch = async (url, init) => {
    const method = init?.method ?? "GET";
    const body = init?.body ? JSON.parse(String(init.body)) as Record<string, unknown> : null;
    requests.push({ method, url: String(url), body });
    expect(init?.redirect).toBe("error");
    expect((init?.headers as Record<string, string>).Authorization).toBe("Bearer test-only-secret");
    const path = new URL(String(url)).pathname;
    const value = method === "GET" && path.includes("/git/ref/") ? { object: { sha: commit } }
      : method === "GET" && path.includes("/git/commits/") ? { tree: { sha: tree } }
      : method === "GET" && path.includes("/compare/") ? {
        status: "ahead", base_commit: { sha: commit }, merge_base_commit: { sha: commit },
      }
      : method === "PATCH" ? { object: { sha: next } }
      : { sha: next };
    return new Response(JSON.stringify(value), { status: 200 });
  };
  return createR2TestGitHubWritePort({
    currentAuthority: async () => {}, credential: async () => "test-only-secret", fetcher,
  });
}

describe("R2-02 pinned GitHub write transport", () => {
  it("pins repository and branch; updates ref without force", async () => {
    const port = fixture();
    expect([port.repository, port.branch]).toEqual([R2_TEST_LEDGER.repository, R2_TEST_LEDGER.branch]);
    expect(await port.readHead()).toEqual({ commit, tree });
    await port.updateRef(next);
    const patch = requests.find((request) => request.method === "PATCH")!;
    expect(patch.url).toBe("https://api.github.com/repos/taiyun668/gogoke/git/refs/heads/s1-r4-ledger-test/r2-02");
    expect(patch.body).toEqual({ sha: next, force: false });
    expect(requests.some((request) => request.url.endsWith("/heads/main"))).toBe(false);
  });

  it("sends blob bytes in JSON body and never embeds them in URL", async () => {
    const port = fixture();
    const value = Buffer.from("public test-only fact");
    await port.createBlob(value);
    const request = requests[0]!;
    expect(request.url).toBe("https://api.github.com/repos/taiyun668/gogoke/git/blobs");
    expect(request.body).toEqual({ content: value.toString("base64"), encoding: "base64" });
  });

  it("requires a credential before any HTTP request", async () => {
    let requested = false;
    const port = createR2TestGitHubWritePort({
      currentAuthority: async () => {}, credential: async () => "",
      fetcher: async () => { requested = true; throw new Error("should not fetch"); },
    });
    await expect(port.readHead()).rejects.toBeInstanceOf(GitFactHttpError);
    expect(requested).toBe(false);
  });

  it("checks ancestry with GitHub compare using exact commit SHAs", async () => {
    const port = fixture();
    expect(await port.isAncestor(commit, next)).toBe(true);
    expect(requests).toHaveLength(1);
    expect(requests[0]).toMatchObject({ method: "GET",
      url: `https://api.github.com/repos/taiyun668/gogoke/compare/${commit}...${next}?per_page=1&page=1` });
  });

  it("distinguishes an explicit non-fast-forward rejection from other 422 responses", async () => {
    for (const message of ["Update is not a fast forward", "Validation Failed"]) {
      const port = createR2TestGitHubWritePort({
        currentAuthority: async () => {}, credential: async () => "test-only-secret",
        fetcher: async () => new Response(JSON.stringify({ message }), { status: 422 }),
      });
      await expect(port.updateRef(next)).rejects.toMatchObject({
        code: message === "Update is not a fast forward"
          ? "GIT_FACT_REF_NON_FAST_FORWARD" : "GIT_FACT_HTTP_422",
      });
    }
  });
});
