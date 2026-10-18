import { useEffect, useMemo, useState } from "react";
import { useI18n } from "@/i18n";
import type {
  RequestUserInputRequest,
  RequestUserInputResponse,
} from "../../../types";

type RequestUserInputMessageProps = {
  requests: RequestUserInputRequest[];
  activeThreadId: string | null;
  activeWorkspaceId?: string | null;
  onSubmit: (
    request: RequestUserInputRequest,
    response: RequestUserInputResponse,
  ) => void;
};

type SelectionState = Record<string, number | null>;
type NotesState = Record<string, string>;

export function RequestUserInputMessage({
  requests,
  activeThreadId,
  activeWorkspaceId,
  onSubmit,
}: RequestUserInputMessageProps) {
  const { tx } = useI18n();
  const activeRequests = useMemo(
    () =>
      requests.filter((request) => {
        if (!activeThreadId) {
          return false;
        }
        if (request.params.thread_id !== activeThreadId) {
          return false;
        }
        if (activeWorkspaceId && request.workspace_id !== activeWorkspaceId) {
          return false;
        }
        return true;
      }),
    [requests, activeThreadId, activeWorkspaceId],
  );
  const activeRequest = activeRequests[0];
  const [selections, setSelections] = useState<SelectionState>({});
  const [notes, setNotes] = useState<NotesState>({});

  useEffect(() => {
    if (!activeRequest) {
      setSelections({});
      setNotes({});
      return;
    }
    const nextSelections: SelectionState = {};
    const nextNotes: NotesState = {};
    activeRequest.params.questions.forEach((question, index) => {
      const key = question.id || `question-${index}`;
      nextSelections[key] = null;
      nextNotes[key] = "";
    });
    setSelections(nextSelections);
    setNotes(nextNotes);
  }, [activeRequest]);

  if (!activeRequest) {
    return null;
  }

  const { questions } = activeRequest.params;
  const hostQuestion = activeRequest.params.card_origin === "host-mcp";
  const hostAnswerOpen = !hostQuestion || activeRequest.params.card_state === "OPEN";
  const hostAnswerUnknown = hostQuestion && activeRequest.params.card_state === "ANSWER_UNKNOWN";
  const totalRequests = activeRequests.length;

  const buildAnswers = () => {
    const answers: RequestUserInputResponse["answers"] = {};
    questions.forEach((question, index) => {
      if (!question.id) {
        return;
      }
      const answerList: string[] = [];
      const key = question.id || `question-${index}`;
      const selectedIndex = selections[key];
      const options = question.options ?? [];
      const hasOptions = options.length > 0;
      if (hasOptions && selectedIndex !== null) {
        const selected = options[selectedIndex];
        const selectedValue = hostQuestion
          ? selected?.label ?? ""
          : selected?.label?.trim() || selected?.description?.trim() || "";
        if (selectedValue) {
          answerList.push(selectedValue);
        }
      }
      const note = (notes[key] ?? "").trim();
      if (hostQuestion) {
        // This C card accepts exactly one host0 value. A free answer and an
        // option are alternatives; Codex's multi-answer shape remains below.
        answers[question.id] = { answers: note ? [note] : answerList.slice(0, 1) };
        return;
      }
      if (note) {
        if (hasOptions) {
          answerList.push(`user_note: ${note}`);
        } else {
          answerList.push(note);
        }
      }
      answers[question.id] = { answers: answerList };
    });
    return answers;
  };

  const handleSelect = (questionId: string, optionIndex: number) => {
    setSelections((current) => ({ ...current, [questionId]: optionIndex }));
    if (hostQuestion) setNotes((current) => ({ ...current, [questionId]: "" }));
  };

  const handleNotesChange = (questionId: string, value: string) => {
    setNotes((current) => ({ ...current, [questionId]: value }));
    if (hostQuestion && value.trim()) {
      setSelections((current) => ({ ...current, [questionId]: null }));
    }
  };

  const handleSubmit = () => {
    onSubmit(activeRequest, { answers: buildAnswers() });
  };
  const hostAnswer = hostQuestion ? buildAnswers().host0?.answers[0] : undefined;
  const hostAnswerValid = hostAnswerOpen && (!hostQuestion || (Boolean(hostAnswer) &&
    new TextEncoder().encode(hostAnswer).length <= 4096));

  return (
    <div className="message request-user-input-message">
      <div
        className="bubble request-user-input-card"
        role="group"
        aria-label={tx("User input requested")}
      >
        <div className="request-user-input-header">
          <div className="request-user-input-title">{tx(hostQuestion ? "Host question" : "Input requested")}</div>
          {totalRequests > 1 ? (
            <div className="request-user-input-queue">
              {tx("Request 1 of {count}", { count: totalRequests })}
            </div>
          ) : null}
        </div>
        <div className="request-user-input-body">
          {hostAnswerUnknown ? (
            <div className="request-user-input-status" role="status">
              {tx("Answer outcome unknown. Do not submit another answer.")}
            </div>
          ) : null}
          {questions.length ? (
            questions.map((question, index) => {
              const questionId = question.id || `question-${index}`;
              const selectedIndex = selections[questionId];
              const options = question.options ?? [];
              const notePlaceholder = question.isOther
                ? tx("Type your answer (optional)")
                : hostQuestion
                  ? tx("Or type your answer")
                : options.length
                  ? tx("Add notes (optional)")
                  : tx("Type your answer (optional)");
              return (
                <section key={questionId} className="request-user-input-question">
                  {question.header ? (
                    <div className="request-user-input-question-header">
                      {question.header}
                    </div>
                  ) : null}
                  <div className="request-user-input-question-text">
                    {question.question}
                  </div>
                  {options.length ? (
                    <div className="request-user-input-options">
                      {options.map((option, optionIndex) => (
                        <button
                          key={`${questionId}-${optionIndex}`}
                          type="button"
                          className={`request-user-input-option${
                            selectedIndex === optionIndex ? " is-selected" : ""
                          }`}
                          aria-pressed={selectedIndex === optionIndex}
                          disabled={!hostAnswerOpen}
                          onClick={() => handleSelect(questionId, optionIndex)}
                        >
                          <div className="request-user-input-option-label">
                            {option.label}
                            {hostQuestion && question.recommendedIndex === optionIndex ? (
                              <span className="request-user-input-recommended"> · {tx("Recommended")}</span>
                            ) : null}
                          </div>
                          {option.description ? (
                            <div className="request-user-input-option-description">
                              {option.description}
                            </div>
                          ) : null}
                        </button>
                      ))}
                    </div>
                  ) : null}
                  {hostQuestion ? (
                    <label className="request-user-input-free-label" htmlFor={`host-answer-${questionId}`}>
                      {tx("Or enter a free answer")}
                    </label>
                  ) : null}
                  <textarea
                    id={hostQuestion ? `host-answer-${questionId}` : undefined}
                    className="request-user-input-notes"
                    placeholder={notePlaceholder}
                    value={notes[questionId] ?? ""}
                    maxLength={hostQuestion ? 4096 : undefined}
                    disabled={!hostAnswerOpen}
                    onChange={(event) =>
                      handleNotesChange(questionId, event.target.value)
                    }
                    rows={2}
                  />
                </section>
              );
            })
          ) : (
            <div className="request-user-input-empty">
              {tx("No questions provided.")}
            </div>
          )}
        </div>
        <div className="request-user-input-actions">
          <button className="primary" onClick={handleSubmit} disabled={!hostAnswerValid}>
            {tx("Submit")}
          </button>
        </div>
      </div>
    </div>
  );
}
