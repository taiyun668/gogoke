import { useState } from "react";
import {
  runGogokeR2GoalProbe,
  type GogokeProductGoalRequest,
  type GogokeProductGoalView,
} from "../../../services/tauri";

export const R2_GOAL_FIXTURE: GogokeProductGoalRequest = Object.freeze({
  goal: Object.freeze({
    id: "goal-r2-01",
    title: "Verify the Gogoke product entry reaches native Product Authority",
  }),
  ledger: Object.freeze({
    repository: "taiyun668/gogoke",
    commit: "6765d4e11ace61c47b9aeb123e0ef4770ab072c0",
    path: "apps/desktop/test-fixtures/s1-r4/ledger/r2-02-source-reference.json",
    contentHash: "sha256:b57db8a5fec4d9a4a09ca1e356c865017f88473916c5debeefa0ca2d87b08d08",
  }),
});

export function HomeProductEntry() {
  const [result, setResult] = useState<GogokeProductGoalView | null>(null);
  const [accepted, setAccepted] = useState<GogokeProductGoalView["ledgerMerge"] | null>(null);
  const [pullNumber, setPullNumber] = useState("");
  const [mergeCommit, setMergeCommit] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [running, setRunning] = useState<"verify" | "task" | "novel" | "accepted" | null>(null);

  const run = async (mode: "verify" | "task" | "novel") => {
    if (running !== null) return;
    setRunning(mode);
    setError(null);
    setAccepted(null);
    try {
      const next = await runGogokeR2GoalProbe(mode === "verify" ? R2_GOAL_FIXTURE : {
        ...R2_GOAL_FIXTURE, runControlledTask: true, publishTestDraft: true,
        ...(mode === "novel" ? { fixtureDriverId: `mock_novel_${Array.from(
          crypto.getRandomValues(new Uint8Array(8)), (byte) => byte.toString(16).padStart(2, "0"),
        ).join("")}` } : {}),
      });
      setResult(next);
    } catch (cause) {
      setResult(null);
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setRunning(null);
    }
  };

  const readAccepted = async () => {
    const draft = result?.testLedgerDraft;
    const number = Number(pullNumber);
    if (running !== null || draft === undefined ||
        !Number.isSafeInteger(number) || number <= 0 ||
        !/^[0-9a-f]{40}$/u.test(mergeCommit)) return;
    setRunning("accepted");
    setError(null);
    setAccepted(null);
    try {
      const readback = await runGogokeR2GoalProbe({
        goal: R2_GOAL_FIXTURE.goal,
        ledger: {
          repository: draft.repository,
          commit: mergeCommit,
          path: draft.path,
          contentHash: draft.contentHash,
        },
        ledgerMergePullNumber: number,
      });
      if (readback.ledgerMerge?.state !== "PR_MERGE_ACCEPTED_FACT_VERIFIED") {
        throw new Error("Accepted Git fact readback was not verified");
      }
      setAccepted(readback.ledgerMerge);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setRunning(null);
    }
  };

  return (
    <section className="home-product-entry" aria-label="Gogoke product path" aria-busy={running !== null}>
      <div className="home-section-header">
        <div>
          <div className="home-section-title">Product path</div>
          <div className="home-product-entry-state">Construction fixture · not adopted</div>
        </div>
        <div className="home-product-entry-actions">
          <button
            className="home-product-entry-button"
            data-tauri-drag-region="false"
            disabled={running !== null}
            onClick={() => void run("verify")}
            type="button"
          >
            {running === "verify" ? "Verifying…" : "Verify local path"}
          </button>
          <button
            className="home-product-entry-button"
            data-tauri-drag-region="false"
            disabled={running !== null}
            onClick={() => void run("task")}
            type="button"
          >
            {running === "task" ? "Running test task…" : "Run test and save draft"}
          </button>
          <button
            className="home-product-entry-button"
            data-tauri-drag-region="false"
            disabled={running !== null}
            onClick={() => void run("novel")}
            type="button"
          >
            {running === "novel" ? "Running open fixture…" : "Run open fixture driver"}
          </button>
        </div>
      </div>
      <div className="home-product-entry-grid">
        <div>
          <div className="home-product-entry-label">Goal</div>
          <div className="home-product-entry-value">{R2_GOAL_FIXTURE.goal.title}</div>
          <div className="home-product-entry-code">{R2_GOAL_FIXTURE.goal.id}</div>
        </div>
        <div>
          <div className="home-product-entry-label">Git ledger reference</div>
          <div className="home-product-entry-value">
            {R2_GOAL_FIXTURE.ledger.repository}
          </div>
          <div className="home-product-entry-code">
            {R2_GOAL_FIXTURE.ledger.commit.slice(0, 12)} · {R2_GOAL_FIXTURE.ledger.path}
          </div>
        </div>
      </div>
      {result?.controlledTask ? (
        <>
          <div className="home-product-entry-result" role="status" aria-atomic="true">
            Verified test fixture report · not adopted
          </div>
          <dl className="home-product-entry-grid home-product-entry-report" aria-label="Verified test report details">
            <div>
              <dt className="home-product-entry-label">Model asset</dt>
              <dd className="home-product-entry-value">{result.controlledTask.modelId}</dd>
            </div>
            <div>
              <dt className="home-product-entry-label">Relative path</dt>
              <dd className="home-product-entry-code">{result.controlledTask.relativePath}</dd>
            </div>
            <div>
              <dt className="home-product-entry-label">Test source commit</dt>
              <dd className="home-product-entry-code">{result.controlledTask.sourceCommit}</dd>
            </div>
            <div>
              <dt className="home-product-entry-label">Test source Git blob</dt>
              <dd className="home-product-entry-code">{result.controlledTask.sourceBlob}</dd>
            </div>
            <div>
              <dt className="home-product-entry-label">Report SHA-256</dt>
              <dd className="home-product-entry-code">{result.controlledTask.reportSha256}</dd>
            </div>
            <div>
              <dt className="home-product-entry-label">Embedded bytes SHA-256</dt>
              <dd className="home-product-entry-code">{result.controlledTask.embeddedBytesSha256}</dd>
            </div>
            <div>
              <dt className="home-product-entry-label">Native Action completion</dt>
              <dd className="home-product-entry-code">{result.controlledTask.actionCompletionRef}</dd>
            </div>
            <div>
              <dt className="home-product-entry-label">Context manifest</dt>
              <dd className="home-product-entry-code">{result.controlledTask.manifestHash}</dd>
            </div>
            <div>
              <dt className="home-product-entry-label">Decision receipt</dt>
              <dd className="home-product-entry-code">{result.controlledTask.decisionReceiptId}</dd>
            </div>
            <div>
              <dt className="home-product-entry-label">Objective Outcome (test-only coordination)</dt>
              <dd className="home-product-entry-code">{result.controlledTask.objectiveOutcomeContentHash}</dd>
            </div>
            <div>
              <dt className="home-product-entry-label">Evaluation (review required)</dt>
              <dd className="home-product-entry-code">{result.controlledTask.evaluationContentHash}</dd>
            </div>
            <div>
              <dt className="home-product-entry-label">Dream proposal (draft, not activated)</dt>
              <dd className="home-product-entry-code">{result.controlledTask.dreamProposalContentHash}</dd>
            </div>
            {result.controlledTask.fixtureDriverBinding ? (
              <div>
                <dt className="home-product-entry-label">Open fixture driver (native bound)</dt>
                <dd className="home-product-entry-code">
                  {result.controlledTask.fixtureDriverBinding.driverId} · {result.controlledTask.fixtureDriverBinding.runtimeInstanceId}
                </dd>
              </div>
            ) : null}
          </dl>
        </>
      ) : result ? (
        <div className="home-product-entry-result" role="status" aria-atomic="true">
          Controller/Seat admitted by native Product Authority · Git blob {result.ledgerReadback.gitBlob.slice(0, 12)} verified, not adopted · {result.nativeHost.elapsedMicros}µs
        </div>
      ) : null}
      {result?.testLedgerDraft ? (
        <>
          <div className="home-product-entry-result" role="status" aria-atomic="true">
            Test ledger draft {result.testLedgerDraft.commit.slice(0, 12)} verified at {result.testLedgerDraft.path} · not adopted
          </div>
          <div className="home-product-entry-actions" aria-label="Accepted test fact readback">
            <input
              aria-label="Test result PR number"
              className="home-product-entry-button"
              inputMode="numeric"
              onChange={(event) => setPullNumber(event.target.value)}
              placeholder="Merged PR number"
              type="text"
              value={pullNumber}
            />
            <input
              aria-label="Test result merge commit"
              className="home-product-entry-button"
              onChange={(event) => setMergeCommit(event.target.value.trim().toLowerCase())}
              placeholder="Merge commit SHA"
              type="text"
              value={mergeCommit}
            />
            <button
              className="home-product-entry-button"
              data-tauri-drag-region="false"
              disabled={running !== null || !Number.isSafeInteger(Number(pullNumber)) ||
                Number(pullNumber) <= 0 || !/^[0-9a-f]{40}$/u.test(mergeCommit)}
              onClick={() => void readAccepted()}
              type="button"
            >
              {running === "accepted" ? "Reading accepted fact…" : "Read accepted test fact"}
            </button>
          </div>
        </>
      ) : null}
      {accepted ? (
        <div className="home-product-entry-result" role="status" aria-atomic="true">
          Test result accepted Git fact verified · PR #{accepted.pullNumber} · merge {accepted.mergeCommit.slice(0, 12)} · merged by {accepted.mergedBy}; Goal Acceptance remains separate
        </div>
      ) : null}
      {error ? (
        <div className="home-product-entry-error" role="alert">
          {error}
        </div>
      ) : null}
    </section>
  );
}
