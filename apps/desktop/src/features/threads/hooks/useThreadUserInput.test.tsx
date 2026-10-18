// @vitest-environment jsdom
import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { respondToUserInputRequest } from "@services/tauri";
import { useThreadUserInput } from "./useThreadUserInput";

vi.mock("@services/tauri", () => ({
  respondToUserInputRequest: vi.fn().mockResolvedValue(undefined),
}));

describe("useThreadUserInput", () => {
  beforeEach(() => {
    vi.mocked(respondToUserInputRequest).mockReset().mockResolvedValue(undefined);
  });
  it("submits request-user-input answers and appends an answered item", async () => {
    const dispatch = vi.fn();
    const { result } = renderHook(() => useThreadUserInput({ dispatch }));
    const request = {
      workspace_id: "ws-1",
      request_id: "req-7",
      params: {
        thread_id: "thread-1",
        turn_id: "turn-1",
        item_id: "item-1",
        questions: [
          {
            id: "q-choice",
            header: "Pick",
            question: "Which option?",
            options: [
              { label: "A", description: "Option A" },
              { label: "B", description: "Option B" },
            ],
          },
        ],
      },
    };
    const response = {
      answers: {
        "q-choice": { answers: ["A", "user_note: with details"] },
      },
    };

    await act(async () => {
      await result.current.handleUserInputSubmit(request, response);
    });

    expect(respondToUserInputRequest).toHaveBeenCalledWith(
      "ws-1",
      "req-7",
      response.answers,
    );
    expect(dispatch).toHaveBeenNthCalledWith(
      1,
      expect.objectContaining({
        type: "upsertItem",
        workspaceId: "ws-1",
        threadId: "thread-1",
        item: expect.objectContaining({
          id: "user-input-ws-1-thread-1-turn-1-item-1",
          kind: "userInput",
          status: "answered",
        }),
      }),
    );
    expect(dispatch).toHaveBeenNthCalledWith(2, {
      type: "removeUserInputRequest",
      requestId: "req-7",
      workspaceId: "ws-1",
    });
  });

  it.each(["ANSWERED", "UNKNOWN"] as const)("retains the original Host card binding and handles %s without inventing an answer", async (outcome) => {
    const dispatch = vi.fn();
    const { result } = renderHook(() => useThreadUserInput({ dispatch }));
    const request = { workspace_id: "ws-host", request_id: "gmpc_original",
      params: { thread_id: "original-thread", turn_id: "original-prompt", item_id: "cardOriginal",
        card_origin: "host-mcp" as const, card_state: "OPEN" as const,
        questions: [{ id: "host0", header: "Scope", question: "Which scope?" }] } };
    const response = { answers: { host0: { answers: ["This project only"] } } };
    if (outcome === "UNKNOWN") vi.mocked(respondToUserInputRequest)
      .mockRejectedValueOnce(new Error("GOGOKE_VISIBLE_UNKNOWN:original_child: original answer unsettled"));
    await act(async () => {
      const submitted = result.current.handleUserInputSubmit(request, response);
      if (outcome === "UNKNOWN") await expect(submitted).rejects.toThrow("original answer unsettled");
      else await submitted;
    });
    expect(respondToUserInputRequest).toHaveBeenCalledExactlyOnceWith("ws-host", "gmpc_original",
      response.answers, undefined, { requestRef: "gmpc_original", hostPromptRequestId: "original-prompt",
        threadId: "original-thread", cardId: "cardOriginal", cardState: "OPEN", cardOrigin: "host-mcp" });
    if (outcome === "UNKNOWN") expect(dispatch).not.toHaveBeenCalled();
    else expect(dispatch).toHaveBeenCalledWith({ type: "removeUserInputRequest",
      requestId: "gmpc_original", workspaceId: "ws-host" });
  });
});
