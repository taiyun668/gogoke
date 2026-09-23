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
    repository: "fixture/gogoke-r2-01",
    commit: "0123456789abcdef0123456789abcdef01234567",
    path: "goals/r2-01.json",
    contentHash: "sha256:" + "a".repeat(64),
  }),
});

export function HomeProductEntry() {
  const [result, setResult] = useState<GogokeProductGoalView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [running, setRunning] = useState(false);

  const run = async () => {
    if (running) return;
    setRunning(true);
    setError(null);
    try {
      const next = await runGogokeR2GoalProbe(R2_GOAL_FIXTURE);
      setResult(next);
    } catch (cause) {
      setResult(null);
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setRunning(false);
    }
  };

  return (
    <section className="home-product-entry" aria-label="Gogoke product path">
      <div className="home-section-header">
        <div>
          <div className="home-section-title">Product path</div>
          <div className="home-product-entry-state">Construction fixture · not adopted</div>
        </div>
        <button
          className="home-product-entry-button"
          data-tauri-drag-region="false"
          disabled={running}
          onClick={() => void run()}
          type="button"
        >
          {running ? "Verifying…" : "Verify local path"}
        </button>
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
        <div className="home-product-entry-result" role="status">
          Controller/Seat admitted by native Product Authority · {result.nativeHost.elapsedMicros}µs
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
