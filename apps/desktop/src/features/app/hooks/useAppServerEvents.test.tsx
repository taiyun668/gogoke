// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AppServerEvent } from "../../../types";
import { subscribeAppServerEvents } from "../../../services/events";
import { useAppServerEvents } from "./useAppServerEvents";
import { RequestUserInputMessage } from "../components/RequestUserInputMessage";

vi.mock("../../../services/events", () => ({
  subscribeAppServerEvents: vi.fn(),
}));

type Handlers = Parameters<typeof useAppServerEvents>[0];

function TestHarness({ handlers }: { handlers: Handlers }) {
  useAppServerEvents(handlers);
  return null;
}

let listener: ((event: AppServerEvent) => void) | null = null;
const unlisten = vi.fn();

beforeEach(() => {
  listener = null;
  unlisten.mockReset();
  vi.mocked(subscribeAppServerEvents).mockImplementation((cb) => {
    listener = cb;
    return unlisten;
  });
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

async function mount(handlers: Handlers) {
  const container = document.createElement("div");
  const root = createRoot(container);
  await act(async () => {
    root.render(<TestHarness handlers={handlers} />);
  });
  return { root };
}

describe("useAppServerEvents", () => {
  it("submits one host option or one free answer from the visible card", () => {
    const submit = vi.fn();
    const request = { workspace_id: "ws", request_id: "gmpc_abc", params: {
      thread_id: "nativeS", turn_id: "sendA", item_id: "cardabc",
      card_origin: "host-mcp" as const, card_state: "OPEN" as const, card_revision: "1",
      questions: [{ id: "host0", header: "Choose", question: "Which?",
        recommendedIndex: 1, options: [
          { label: "A", description: "First" }, { label: " B ", description: "Second" },
        ] }],
    } };
    const { container, rerender } = render(<RequestUserInputMessage requests={[request]}
      activeThreadId="nativeS" activeWorkspaceId="ws" onSubmit={submit} />);
    expect(container.textContent).toContain("Host question");
    expect(container.textContent).toContain("Recommended");
    const send = screen.getByRole("button", { name: "Submit" });
    expect((send as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: /B/ }));
    fireEvent.click(send);
    expect(submit).toHaveBeenLastCalledWith(request, { answers: {
      host0: { answers: [" B "] },
    } });
    fireEvent.change(container.querySelector("textarea")!, { target: { value: "Custom answer" } });
    fireEvent.click(send);
    expect(submit).toHaveBeenLastCalledWith(request, { answers: {
      host0: { answers: ["Custom answer"] },
    } });
    const unknown = { ...request, params: { ...request.params,
      card_state: "ANSWER_UNKNOWN" as const, card_revision: "2" } };
    rerender(<RequestUserInputMessage requests={[unknown]}
      activeThreadId="nativeS" activeWorkspaceId="ws" onSubmit={submit} />);
    expect(container.textContent).toContain("Answer outcome unknown");
    expect((screen.getByRole("button", { name: "Submit" }) as HTMLButtonElement).disabled).toBe(true);
    expect((container.querySelector("textarea") as HTMLTextAreaElement).disabled).toBe(true);
  });

  it("projects only source-bound Grok C host questions with recommendation", async () => {
    const handlers: Handlers = { onRequestUserInput: vi.fn() };
    const { root } = await mount(handlers);
    const event: AppServerEvent = {
      workspace_id: "ws",
      nativeAssociation: { sessionId: "session", bindingGeneration: "2" },
      nativeSourceRef: { operationId: "op", sourceEpoch: "nonce", sourceCursor: "4" },
      message: { method: "gogoke/hostQuestion", params: {
        cardId: "cardabc", requestRef: "gmpc_abc", threadId: "nativeS", turnId: "sendA",
        cardState: "OPEN", cardRevision: "1",
        nativeQuestion: { provider: "grok", cardOrigin: "host-mcp", bindingVersion: 1, allowFreeAnswer: true,
          recommendedIndex: 1, questions: [{ id: "host0", header: "Choose",
            question: "Which?", options: [{label:"A",description:"First"},
              {label:"B",description:"Second"}] }],
          binding: { hostSessionId: "session", generation: "2", hostPromptRequestId: "sendA",
            processOperationId: "op", sourceEpoch: "nonce", sourceCursor: "4" } },
      } },
    };
    act(() => { listener?.(event); });
    expect(handlers.onRequestUserInput).toHaveBeenCalledWith({
      workspace_id: "ws", request_id: "gmpc_abc", params: {
        thread_id: "nativeS", turn_id: "sendA", item_id: "cardabc",
        card_origin: "host-mcp", card_state: "OPEN", card_revision: "1",
        questions: [{ id: "host0", header: "Choose",
          question: "Which?", options: [{label:"A",description:"First"},
            {label:"B",description:"Second"}], recommendedIndex: 1 }],
      },
    });
    const changed = structuredClone(event);
    changed.nativeSourceRef!.sourceCursor = "5";
    act(() => { listener?.(changed); });
    expect(handlers.onRequestUserInput).toHaveBeenCalledTimes(1);
    const unknown=structuredClone(event);
    unknown.message.method="gogoke/hostQuestionState";
    (unknown.message.params as Record<string, unknown>).cardState="ANSWER_UNKNOWN";
    (unknown.message.params as Record<string, unknown>).cardRevision="2";
    act(() => {listener?.(unknown);});
    expect(handlers.onRequestUserInput).toHaveBeenLastCalledWith(expect.objectContaining({
      request_id:"gmpc_abc",params:expect.objectContaining({
        card_state:"ANSWER_UNKNOWN",card_revision:"2",
      }),
    }));
    expect(handlers.onRequestUserInput).toHaveBeenCalledTimes(2);
    const expired=structuredClone(unknown);
    (expired.message.params as Record<string, unknown>).cardState="EXPIRED";
    (expired.message.params as Record<string, unknown>).cardRevision="3";
    act(() => {listener?.(expired);});
    expect(handlers.onRequestUserInput).toHaveBeenLastCalledWith(expect.objectContaining({
      request_id:"gmpc_abc",params:expect.objectContaining({card_state:"EXPIRED"}),
    }));
    expect(handlers.onRequestUserInput).toHaveBeenCalledTimes(3);
    act(() => { listener?.({workspace_id:"ws",message:{method:"x.ai/mcp/sdk_call",id:1,
      params:{}}}); });
    expect(handlers.onRequestUserInput).toHaveBeenCalledTimes(3);
    await act(async () => { root.unmount(); });
  });

  it("routes app-server events to handlers", async () => {
    const handlers: Handlers = {
      onAppServerEvent: vi.fn(),
      onWorkspaceConnected: vi.fn(),
      onHookStarted: vi.fn(),
      onHookCompleted: vi.fn(),
      onThreadStarted: vi.fn(),
      onThreadNameUpdated: vi.fn(),
      onThreadStatusChanged: vi.fn(),
      onThreadClosed: vi.fn(),
      onThreadArchived: vi.fn(),
      onThreadUnarchived: vi.fn(),
      onBackgroundThreadAction: vi.fn(),
      onAgentMessageDelta: vi.fn(),
      onReasoningSummaryBoundary: vi.fn(),
      onPlanDelta: vi.fn(),
      onApprovalRequest: vi.fn(),
      onRequestUserInput: vi.fn(),
      onItemCompleted: vi.fn(),
      onAgentMessageCompleted: vi.fn(),
      onAccountRateLimitsUpdated: vi.fn(),
      onAccountUpdated: vi.fn(),
      onAccountLoginCompleted: vi.fn(),
    };
    const { root } = await mount(handlers);

    expect(listener).toBeTypeOf("function");

    act(() => {
      listener?.({ workspace_id: "ws-1", message: { method: "codex/connected" } });
    });
    expect(handlers.onWorkspaceConnected).toHaveBeenCalledWith("ws-1");

    act(() => {
      listener?.({
        workspace_id: "ws-1",
        message: {
          method: "item/agentMessage/delta",
          params: { threadId: "thread-1", itemId: "item-1", delta: "Hello" },
        },
      });
    });
    expect(handlers.onAgentMessageDelta).toHaveBeenCalledWith({
      workspaceId: "ws-1",
      threadId: "thread-1",
      itemId: "item-1",
      delta: "Hello",
    });

    act(() => {
      listener?.({
        workspace_id: "ws-1",
        message: {
          method: "item/reasoning/summaryPartAdded",
          params: { threadId: "thread-1", itemId: "reasoning-1", summaryIndex: 1 },
        },
      });
    });
    expect(handlers.onReasoningSummaryBoundary).toHaveBeenCalledWith(
      "ws-1",
      "thread-1",
      "reasoning-1",
    );

    act(() => {
      listener?.({
        workspace_id: "ws-1",
        message: {
          method: "item/plan/delta",
          params: { threadId: "thread-1", itemId: "plan-1", delta: "- Step 1" },
        },
      });
    });
    expect(handlers.onPlanDelta).toHaveBeenCalledWith(
      "ws-1",
      "thread-1",
      "plan-1",
      "- Step 1",
    );

    act(() => {
      listener?.({
        workspace_id: "ws-1",
        message: {
          method: "hook/started",
          params: {
            threadId: "thread-1",
            turnId: "turn-1",
            run: { id: "hook-1", eventName: "session-start", statusMessage: "Preparing" },
          },
        },
      });
    });
    expect(handlers.onHookStarted).toHaveBeenCalledWith({
      workspaceId: "ws-1",
      threadId: "thread-1",
      turnId: "turn-1",
      run: { id: "hook-1", eventName: "session-start", statusMessage: "Preparing" },
    });

    act(() => {
      listener?.({
        workspace_id: "ws-1",
        message: {
          method: "hook/completed",
          params: {
            threadId: "thread-1",
            run: { id: "hook-1", eventName: "session-start", status: "completed" },
          },
        },
      });
    });
    expect(handlers.onHookCompleted).toHaveBeenCalledWith({
      workspaceId: "ws-1",
      threadId: "thread-1",
      turnId: null,
      run: { id: "hook-1", eventName: "session-start", status: "completed" },
    });

    act(() => {
      listener?.({
        workspace_id: "ws-1",
        message: {
          method: "thread/started",
          params: { thread: { id: "thread-2", preview: "New thread" } },
        },
      });
    });
    expect(handlers.onThreadStarted).toHaveBeenCalledWith("ws-1", {
      id: "thread-2",
      preview: "New thread",
    });

    act(() => {
      listener?.({
        workspace_id: "ws-1",
        message: {
          method: "thread/name/updated",
          params: { threadId: "thread-2", threadName: "Renamed from server" },
        },
      });
    });
    expect(handlers.onThreadNameUpdated).toHaveBeenCalledWith("ws-1", {
      threadId: "thread-2",
      threadName: "Renamed from server",
    });

    act(() => {
      listener?.({
        workspace_id: "ws-1",
        message: {
          method: "thread/status/changed",
          params: { threadId: "thread-2", status: { type: "active" } },
        },
      });
    });
    expect(handlers.onThreadStatusChanged).toHaveBeenCalledWith(
      "ws-1",
      "thread-2",
      { type: "active" },
    );

    act(() => {
      listener?.({
        workspace_id: "ws-1",
        message: {
          method: "thread/closed",
          params: { threadId: "thread-2" },
        },
      });
    });
    expect(handlers.onThreadClosed).toHaveBeenCalledWith("ws-1", "thread-2");

    act(() => {
      listener?.({
        workspace_id: "ws-1",
        message: {
          method: "thread/archived",
          params: { thread_id: "thread-2" },
        },
      });
    });
    expect(handlers.onThreadArchived).toHaveBeenCalledWith("ws-1", "thread-2");

    act(() => {
      listener?.({
        workspace_id: "ws-1",
        message: {
          method: "thread/unarchived",
          params: { threadId: "thread-2" },
        },
      });
    });
    expect(handlers.onThreadUnarchived).toHaveBeenCalledWith("ws-1", "thread-2");

    act(() => {
      listener?.({
        workspace_id: "ws-1",
        message: {
          method: "codex/backgroundThread",
          params: { threadId: "thread-2", action: "hide" },
        },
      });
    });
    expect(handlers.onBackgroundThreadAction).toHaveBeenCalledWith(
      "ws-1",
      "thread-2",
      "hide",
    );

    act(() => {
      listener?.({
        workspace_id: "ws-1",
        message: {
          method: "item/permissions/requestApproval",
          id: 7,
          params: { mode: "full", threadId: "thread-2" },
        },
      });
    });
    expect(handlers.onApprovalRequest).toHaveBeenCalledWith({
      workspace_id: "ws-1",
      request_id: 7,
      method: "item/permissions/requestApproval",
      params: { mode: "full", threadId: "thread-2" },
    });

    act(() => {
      listener?.({
        workspace_id: "ws-1",
        message: {
          method: "item/tool/requestUserInput",
          id: 11,
          params: {
            thread_id: "thread-1",
            turn_id: "turn-1",
            item_id: "call-1",
            questions: [
              {
                id: "confirm_path",
                header: "Confirm",
                question: "Proceed?",
                options: [
                  { label: "Yes", description: "Continue." },
                  { label: "No", description: "Stop." },
                ],
              },
            ],
          },
        },
      });
    });
    expect(handlers.onRequestUserInput).toHaveBeenCalledWith({
      workspace_id: "ws-1",
      request_id: 11,
      params: {
        thread_id: "thread-1",
        turn_id: "turn-1",
        item_id: "call-1",
        questions: [
          {
            id: "confirm_path",
            header: "Confirm",
            question: "Proceed?",
            isOther: false,
            options: [
              { label: "Yes", description: "Continue." },
              { label: "No", description: "Stop." },
            ],
          },
        ],
      },
    });

    act(() => {
      listener?.({
        workspace_id: "ws-1",
        message: {
          method: "item/completed",
          params: {
            threadId: "thread-1",
            item: { type: "agentMessage", id: "item-2", text: "Done" },
          },
        },
      });
    });
    expect(handlers.onItemCompleted).toHaveBeenCalledWith("ws-1", "thread-1", {
      type: "agentMessage",
      id: "item-2",
      text: "Done",
    });
    expect(handlers.onAgentMessageCompleted).toHaveBeenCalledWith({
      workspaceId: "ws-1",
      threadId: "thread-1",
      itemId: "item-2",
      text: "Done",
    });

    act(() => {
      listener?.({
        workspace_id: "ws-1",
        message: {
          method: "account/rateLimits/updated",
          params: {
            rateLimits: { primary: { usedPercent: 25 } },
          },
        },
      });
    });
    expect(handlers.onAccountRateLimitsUpdated).toHaveBeenCalledWith("ws-1", {
      primary: { usedPercent: 25 },
    });

    act(() => {
      listener?.({
        workspace_id: "ws-1",
        message: {
          method: "account/rateLimits/updated",
          params: {
            rate_limits: { primary: { used_percent: 30 } },
          },
        },
      });
    });
    expect(handlers.onAccountRateLimitsUpdated).toHaveBeenCalledWith("ws-1", {
      primary: { used_percent: 30 },
    });

    act(() => {
      listener?.({
        workspace_id: "ws-1",
        message: {
          method: "account/updated",
          params: { authMode: "chatgpt" },
        },
      });
    });
    expect(handlers.onAccountUpdated).toHaveBeenCalledWith("ws-1", "chatgpt");

    act(() => {
      listener?.({
        workspace_id: "ws-1",
        message: {
          method: "account/login/completed",
          params: { loginId: "login-1", success: true, error: null },
        },
      });
    });
    expect(handlers.onAccountLoginCompleted).toHaveBeenCalledWith("ws-1", {
      loginId: "login-1",
      success: true,
      error: null,
    });

    await act(async () => {
      root.unmount();
    });
    expect(unlisten).toHaveBeenCalledTimes(1);
  });

  it("normalizes request user input questions and options", async () => {
    const handlers: Handlers = {
      onRequestUserInput: vi.fn(),
    };
    const { root } = await mount(handlers);

    act(() => {
      listener?.({
        workspace_id: "ws-9",
        message: {
          method: "item/tool/requestUserInput",
          id: 55,
          params: {
            threadId: "thread-9",
            turnId: "turn-9",
            itemId: "item-9",
            questions: [
              {
                id: "",
                header: "",
                question: "",
                options: [
                  { label: "", description: "" },
                  { label: "  ", description: " " },
                ],
              },
              {
                id: "q-1",
                header: "",
                question: "Choose",
                options: [
                  { label: "", description: "" },
                  { label: "Yes", description: "" },
                  { label: "", description: "No label" },
                ],
              },
            ],
          },
        },
      });
    });

    expect(handlers.onRequestUserInput).toHaveBeenCalledWith({
      workspace_id: "ws-9",
      request_id: 55,
      params: {
        thread_id: "thread-9",
        turn_id: "turn-9",
        item_id: "item-9",
        questions: [
          {
            id: "q-1",
            header: "",
            question: "Choose",
            isOther: false,
            options: [
              { label: "Yes", description: "" },
              { label: "", description: "No label" },
            ],
          },
        ],
      },
    });

    await act(async () => {
      root.unmount();
    });
  });

  it("ignores delta events missing required fields", async () => {
    const handlers: Handlers = {
      onAgentMessageDelta: vi.fn(),
    };
    const { root } = await mount(handlers);

    act(() => {
      listener?.({
        workspace_id: "ws-1",
        message: {
          method: "item/agentMessage/delta",
          params: { threadId: "", itemId: "item-1", delta: "Hello" },
        },
      });
    });
    act(() => {
      listener?.({
        workspace_id: "ws-1",
        message: {
          method: "item/agentMessage/delta",
          params: { threadId: "thread-1", itemId: "", delta: "Hello" },
        },
      });
    });
    act(() => {
      listener?.({
        workspace_id: "ws-1",
        message: {
          method: "item/agentMessage/delta",
          params: { threadId: "thread-1", itemId: "item-1", delta: "" },
        },
      });
    });

    expect(handlers.onAgentMessageDelta).not.toHaveBeenCalled();

    await act(async () => {
      root.unmount();
    });
  });

  it("coerces string thread status payloads to object form", async () => {
    const handlers: Handlers = {
      onThreadStatusChanged: vi.fn(),
    };
    const { root } = await mount(handlers);

    act(() => {
      listener?.({
        workspace_id: "ws-1",
        message: {
          method: "thread/status/changed",
          params: { thread_id: "thread-1", status: "idle" },
        },
      });
    });

    expect(handlers.onThreadStatusChanged).toHaveBeenCalledWith(
      "ws-1",
      "thread-1",
      { type: "idle" },
    );

    await act(async () => {
      root.unmount();
    });
  });
});
