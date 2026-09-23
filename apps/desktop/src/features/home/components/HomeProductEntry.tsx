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
  const [error, setError] = useState<string | null>(null);
  const [running, setRunning] = useState<"verify" | "task" | null>(null);

  const run = async (mode: "verify" | "task") => {
    if (running !== null) return;
    setRunning(mode);
    setError(null);
    try {
      const next = await runGogokeR2GoalProbe(mode === "task"
        ? { ...R2_GOAL_FIXTURE, runControlledTask: true }
        : R2_GOAL_FIXTURE);
      setResult(next);
    } catch (cause) {
      setResult(null);
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
            {running === "task" ? "Running test task…" : "Run controlled test task"}
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
      {result ? (
        <div className="home-product-entry-result" role="status" aria-atomic="true">
          {result.controlledTask
            ? `Test-only report verified from ${result.controlledTask.modelId}; not adopted · ${result.controlledTask.reportSha256.slice(0, 12)}`
            : `Controller/Seat admitted by native Product Authority · Git blob ${result.ledgerReadback.gitBlob.slice(0, 12)} verified, not adopted · ${result.nativeHost.elapsedMicros}µs`}
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
