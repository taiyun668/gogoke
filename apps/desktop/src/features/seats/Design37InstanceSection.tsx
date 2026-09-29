import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { SettingsSection } from "@/features/design-system/components/settings/SettingsPrimitives";

type InstanceReceipt = {
  schema: string;
  family: string;
  operation: string;
  requestId: string;
  targetId: string;
  status: string;
  revision: string;
  result: { state?: string; reason?: string };
};

type LoginReply = {
  schema: string;
  instanceId: string;
  requestId: string;
  state: "PENDING" | "LOGGED_IN" | "LOGGED_OUT" | "UNKNOWN";
  output: string;
};

const INSTANCE_ID = /^[A-Za-z][A-Za-z0-9_-]{0,63}$/;

function nextRequestId(): string {
  return `owner_${crypto.randomUUID().replace(/-/g, "")}`;
}

function receiptRevision(value: string): number {
  if (!/^[1-9]\d*$/.test(value)) {
    throw new Error("Instance status returned an invalid revision.");
  }
  const revision = Number(value);
  if (!Number.isSafeInteger(revision)) {
    throw new Error("Instance status revision is outside the supported range.");
  }
  return revision;
}

function readReceipt(raw: string, operation: "install-state" | "login-state", instanceId: string, requestId: string): InstanceReceipt {
  const receipt: InstanceReceipt = JSON.parse(raw);
  if (receipt.schema !== "gogoke.37.operations.v1" || receipt.family !== "K-INSTANCE" ||
      receipt.operation !== operation || receipt.targetId !== instanceId ||
      receipt.requestId !== requestId ||
      typeof receipt.status !== "string" ||
      !receipt.result || typeof receipt.result !== "object") {
    throw new Error("Instance status reply did not match this instance.");
  }
  return receipt;
}

function readLoginReply(raw: string, instanceId: string, requestId: string): LoginReply {
  const reply: LoginReply = JSON.parse(raw);
  if (reply.schema !== "gogoke.37.owner-login.v1" || reply.instanceId !== instanceId ||
      reply.requestId !== requestId || !["PENDING", "LOGGED_IN", "LOGGED_OUT", "UNKNOWN"].includes(reply.state) ||
      typeof reply.output !== "string") {
    throw new Error("Owner login reply did not match this request.");
  }
  return reply;
}

/** Explicit Owner actions for one independent Codex test instance. */
export function Design37InstanceSection() {
  const [instanceId, setInstanceId] = useState("codexTestM1");
  const [revision, setRevision] = useState<number | null>(null);
  const [loginRequestId, setLoginRequestId] = useState<string | null>(null);
  const [loginRevision, setLoginRevision] = useState<number | null>(null);
  const [loginState, setLoginState] = useState<string | null>(null);
  const [cliState, setCliState] = useState<string | null>(null);
  const [deviceOutput, setDeviceOutput] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const validId = INSTANCE_ID.test(instanceId);

  // Observe this same login Job while the Owner reads its instructions. This
  // only reads status; it never starts a second login or resends a device code.
  useEffect(() => {
    if (busy || error || loginState !== "PENDING" || !loginRequestId || loginRevision === null) return;
    const timer = window.setTimeout(() => {
      setBusy("status");
      void invoke<string>("gogoke_design37_user_operation", {
        frame: JSON.stringify({ schema: "gogoke.37.owner-login.v1", action: "status",
          instanceId, requestId: loginRequestId, expectedRevision: loginRevision }),
      }).then((raw) => {
        const reply = readLoginReply(raw, instanceId, loginRequestId);
        setLoginState(reply.state);
        setDeviceOutput(reply.output);
      }).catch((cause: unknown) => {
        setError(cause instanceof Error ? cause.message : String(cause));
        setLoginState("UNKNOWN");
      }).finally(() => setBusy(null));
    }, 500);
    return () => window.clearTimeout(timer);
  }, [busy, error, instanceId, loginRequestId, loginRevision, loginState]);

  async function register() {
    if (!validId || busy) return;
    setBusy("register");
    setError(null);
    setMessage(null);
    try {
      // The fixed request ID lets this registration replay after the settings view remounts.
      const receipt = await invoke<InstanceReceipt>("gogoke_design37_register_codex_instance", {
        request: { instanceId, requestId: instanceId },
      });
      if (receipt.schema !== "gogoke.37.operations.v1" || receipt.family !== "K-INSTANCE" ||
          receipt.operation !== "register" || receipt.targetId !== instanceId ||
          receipt.requestId !== instanceId || !["APPLIED", "REPLAYED"].includes(receipt.status)) {
        throw new Error(`Registration did not complete: ${receipt.status ?? "invalid reply"}`);
      }
      const registeredRevision = receiptRevision(receipt.revision);
      // Replay returns the original registration revision, not the row's current
      // revision. A read returns the current revision even when it is STALE.
      const requestId = nextRequestId();
      const raw = await invoke<string>("gogoke_design37_user_operation", {
        frame: JSON.stringify({
          schema: "gogoke.37.operations.v1", family: "K-INSTANCE",
          operation: "install-state", requestId, domainId: "global",
          targetId: instanceId, expectedRevision: String(registeredRevision), payload: {},
        }),
      });
      const current = readReceipt(raw, "install-state", instanceId, requestId);
      if (current.status !== "APPLIED" && current.status !== "STALE") {
        throw new Error(`Instance revision unavailable: ${current.status}`);
      }
      setRevision(receiptRevision(current.revision));
      setCliState(null);
      setMessage(receipt.status === "REPLAYED" ? "Existing instance registration confirmed." : "Independent instance registered.");
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(null);
    }
  }

  async function ownerLogin(action: "begin" | "cancel") {
    if (revision === null || busy) return;
    const requestId = action === "begin" ? nextRequestId() : loginRequestId;
    const expectedRevision = action === "begin" ? revision : loginRevision;
    if (!requestId || expectedRevision === null) return;
    setBusy(action);
    setError(null);
    setMessage(null);
    if (action === "begin") {
      setLoginRequestId(requestId);
      setLoginRevision(expectedRevision);
      setDeviceOutput("");
    }
    try {
      const raw = await invoke<string>("gogoke_design37_user_operation", {
        frame: JSON.stringify({
          schema: "gogoke.37.owner-login.v1", action, instanceId,
          requestId, expectedRevision,
        }),
      });
      const reply = readLoginReply(raw, instanceId, requestId);
      setLoginState(reply.state);
      setDeviceOutput(reply.output);
      if (action === "cancel") setMessage("Login request cancelled.");
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(null);
    }
  }

  async function refreshStatus() {
    if (revision === null || busy) return;
    setBusy("refresh");
    setError(null);
    setMessage(null);
    try {
      if (loginRequestId && loginRevision !== null) {
        const rawLogin = await invoke<string>("gogoke_design37_user_operation", {
          frame: JSON.stringify({
            schema: "gogoke.37.owner-login.v1", action: "status", instanceId,
            requestId: loginRequestId, expectedRevision: loginRevision,
          }),
        });
        const reply = readLoginReply(rawLogin, instanceId, loginRequestId);
        setLoginState(reply.state);
        setDeviceOutput(reply.output);
        if (reply.state === "PENDING") return;
      } else {
        const requestId = nextRequestId();
        const raw = await invoke<string>("gogoke_design37_user_operation", {
          frame: JSON.stringify({ schema: "gogoke.37.owner-login.v1", action: "refresh",
            instanceId, requestId, expectedRevision: revision }),
        });
        setCliState(readLoginReply(raw, instanceId, requestId).state);
      }
      const requestId = nextRequestId();
      const raw = await invoke<string>("gogoke_design37_user_operation", {
        frame: JSON.stringify({
          schema: "gogoke.37.operations.v1", family: "K-INSTANCE",
          operation: "login-state", requestId, domainId: "global",
          targetId: instanceId, expectedRevision: String(revision), payload: {},
        }),
      });
      const receipt = readReceipt(raw, "login-state", instanceId, requestId);
      if (["APPLIED", "REPLAYED", "STALE"].includes(receipt.status)) {
        setRevision(receiptRevision(receipt.revision));
      }
      setCliState(["APPLIED", "REPLAYED"].includes(receipt.status) ? receipt.result.state ?? "UNKNOWN" : "UNKNOWN");
      if (receipt.status === "STALE") setMessage("Instance revision changed. Refresh status again.");
      else if (!["APPLIED", "REPLAYED", "UNKNOWN"].includes(receipt.status)) {
        setError(`Instance status unavailable: ${receipt.status}`);
      }
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(null);
    }
  }

  return (
    <SettingsSection title="Independent Codex test instance" subtitle="Register this instance, then start its own CLI sign-in when you are ready. This does not change daily Codex settings or accounts.">
      <div className="settings-field">
        <label className="settings-field-label" htmlFor="design37-instance-id">Instance ID</label>
        <input id="design37-instance-id" className="settings-input" value={instanceId}
          onChange={(event) => {
            setInstanceId(event.target.value);
            setRevision(null);
            setLoginRequestId(null);
            setLoginRevision(null);
            setLoginState(null);
            setCliState(null);
            setDeviceOutput("");
            setError(null);
            setMessage(null);
          }} disabled={busy !== null || loginState === "PENDING"} aria-invalid={!validId} aria-describedby="design37-instance-help" />
        <div id="design37-instance-help" className={validId ? "settings-help" : "settings-help settings-help-error"}>
          {validId ? "Letters, digits, _ and -; start with a letter." : "Enter an ID starting with a letter, up to 64 characters."}
        </div>
        <div className="settings-field-actions">
          <button type="button" className="primary settings-button-compact" onClick={() => void register()} disabled={!validId || busy !== null || loginState === "PENDING"}>
            {busy === "register" ? "Registering…" : "Register instance"}
          </button>
          <button type="button" className="ghost settings-button-compact" onClick={() => void ownerLogin("begin")}
            disabled={revision === null || busy !== null || loginState === "PENDING"}>
            {busy === "begin" ? "Starting…" : "Start login"}
          </button>
          <button type="button" className="ghost settings-button-compact" onClick={() => void refreshStatus()}
            disabled={revision === null || busy !== null}>
            {busy === "refresh" ? "Refreshing…" : "Refresh status"}
          </button>
          <button type="button" className="ghost settings-button-compact" onClick={() => void ownerLogin("cancel")}
            disabled={revision === null || !loginRequestId || loginRevision === null || loginState !== "PENDING" || busy !== null}>
            {busy === "cancel" ? "Cancelling…" : "Cancel login"}
          </button>
        </div>
        {revision !== null ? <div className="settings-help">Instance revision: {revision}</div> : null}
        {loginState ? <div className="settings-help" role="status">Login request: {loginState}</div> : null}
        {cliState ? <div className="settings-help" role="status">CLI login observation: {cliState}. This does not verify account validity.</div> : null}
        {message ? <div className="settings-help" role="status">{message}</div> : null}
        {error ? <div className="settings-help settings-help-error" role="alert">{error}</div> : null}
        {deviceOutput ? (
          <div className="settings-field">
            <div className="settings-field-label">CLI sign-in instructions</div>
            <div className="settings-help" aria-live="polite">{deviceOutput.split(/\r?\n/).map((line, index) => (
              <div key={index}>{line || "\u00a0"}</div>
            ))}</div>
            <button type="button" className="ghost settings-button-compact" onClick={() => {
              void navigator.clipboard.writeText(deviceOutput).catch((cause: unknown) => {
                setError(cause instanceof Error ? cause.message : String(cause));
              });
            }}>Copy instructions</button>
          </div>
        ) : null}
      </div>
    </SettingsSection>
  );
}
