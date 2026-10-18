import type { ThreadAction, ThreadState } from "../useThreadsReducer";

export function reduceThreadQueue(state: ThreadState, action: ThreadAction): ThreadState {
  switch (action.type) {
    case "addApproval": {
      const exists = state.approvals.some(
        (item) =>
          item.request_id === action.approval.request_id &&
          item.workspace_id === action.approval.workspace_id,
      );
      if (exists) {
        return state;
      }
      return { ...state, approvals: [...state.approvals, action.approval] };
    }
    case "removeApproval":
      return {
        ...state,
        approvals: state.approvals.filter(
          (item) =>
            item.request_id !== action.requestId ||
            item.workspace_id !== action.workspaceId,
        ),
      };
    case "addUserInputRequest": {
      const index = state.userInputRequests.findIndex(
        (item) =>
          item.request_id === action.request.request_id &&
          item.workspace_id === action.request.workspace_id,
      );
      const incoming=action.request;
      const current=index >= 0 ? state.userInputRequests[index] : null;
      const priorRevision=current?.params.card_revision ?? "0";
      const nextRevision=incoming.params.card_revision ?? "0";
      const stale=nextRevision.length < priorRevision.length ||
        (nextRevision.length === priorRevision.length && nextRevision < priorRevision);
      if (incoming.params.card_origin === "host-mcp" &&
          (incoming.params.card_state === "EXPIRED" ||
            incoming.params.card_state === "ANSWERED")) {
        if (index < 0) return state;
        const prior=state.userInputRequests[index];
        if (prior.params.card_origin !== "host-mcp" ||
            prior.params.thread_id !== incoming.params.thread_id ||
            prior.params.turn_id !== incoming.params.turn_id ||
            prior.params.item_id !== incoming.params.item_id || stale) return state;
        return { ...state, userInputRequests: state.userInputRequests.filter((_, at) => at !== index) };
      }
      if (index >= 0) {
        const prior=state.userInputRequests[index];
        if (prior.params.card_origin !== "host-mcp" ||
            incoming.params.card_origin !== "host-mcp" ||
            prior.params.thread_id !== incoming.params.thread_id ||
            prior.params.turn_id !== incoming.params.turn_id ||
            prior.params.item_id !== incoming.params.item_id ||
            incoming.params.card_state !== "ANSWER_UNKNOWN" ||
            prior.params.card_state === "ANSWER_UNKNOWN" || stale) {
          return state;
        }
        return { ...state, userInputRequests: state.userInputRequests.map((item, at) =>
          at === index ? incoming : item) };
      }
      if (action.request.params.card_origin === "host-mcp" &&
          action.request.params.card_state !== "OPEN" &&
          action.request.params.card_state !== "ANSWER_UNKNOWN") {
        return state;
      }
      return {
        ...state,
        userInputRequests: [...state.userInputRequests, action.request],
      };
    }
    case "removeUserInputRequest":
      return {
        ...state,
        userInputRequests: state.userInputRequests.filter(
          (item) =>
            item.request_id !== action.requestId ||
            item.workspace_id !== action.workspaceId,
        ),
      };
    default:
      return state;
  }
}
