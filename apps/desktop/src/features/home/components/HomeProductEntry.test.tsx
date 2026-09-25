// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { HomeProductEntry } from "./HomeProductEntry";

const probe = vi.hoisted(() => vi.fn());
vi.mock("../../../services/tauri", () => ({ runGogokeR2GoalProbe: probe }));

afterEach(() => {
  cleanup();
  probe.mockReset();
});

it("reads the merged test result through the product entry without another draft write", async () => {
  const draft = {
    state: "DRAFT_COMMITTED_NOT_ADOPTED",
    repository: "taiyun668/gogoke",
    branch: "s1-r4-ledger-test/r2-02",
    commit: "a".repeat(40),
    path: "apps/desktop/test-fixtures/s1-r4/ledger/r2-02-results/result.json",
    gitBlob: "b".repeat(40),
    contentHash: `sha256:${"c".repeat(64)}`,
  };
  const mergeCommit = "d".repeat(40);
  probe.mockResolvedValueOnce({
    testLedgerDraft: draft,
    ledgerReadback: { gitBlob: draft.gitBlob },
    nativeHost: { elapsedMicros: 1 },
  });
  probe.mockResolvedValueOnce({
    ledgerMerge: {
      state: "PR_MERGE_ACCEPTED_FACT_VERIFIED",
      pullNumber: 42,
      mergeCommit,
      mergedBy: "taiyun668",
    },
  });
  render(<HomeProductEntry />);
  fireEvent.click(screen.getByRole("button", { name: "Run test and save draft" }));
  await screen.findByText(/Test ledger draft a{12} verified/u);
  fireEvent.change(screen.getByRole("textbox", { name: "Test result PR number" }), {
    target: { value: "42" },
  });
  fireEvent.change(screen.getByRole("textbox", { name: "Test result merge commit" }), {
    target: { value: mergeCommit },
  });
  fireEvent.click(screen.getByRole("button", { name: "Read accepted test fact" }));
  await waitFor(() => expect(probe).toHaveBeenCalledTimes(2));
  expect(probe.mock.calls[1]?.[0]).toEqual({
    goal: { id: "goal-r2-01", title: "Verify the Gogoke product entry reaches native Product Authority" },
    ledger: {
      repository: draft.repository,
      commit: mergeCommit,
      path: draft.path,
      contentHash: draft.contentHash,
    },
    ledgerMergePullNumber: 42,
  });
  expect(await screen.findByText(/Test result accepted Git fact verified · PR #42/u)).toBeTruthy();
});
