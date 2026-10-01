import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { SettingsSection } from "@/features/design-system/components/settings/SettingsPrimitives";
import {
  design37InstanceStateLabel,
  design37LoginStateLabel,
  DESIGN37_TEST_INSTANCE_ID,
  readDesign37InstancesSnapshot,
  type Design37Instance,
  type Design37InstancesSnapshot,
} from "./design37Instances";

function errorText(cause: unknown): string {
  return cause instanceof Error ? cause.message : String(cause);
}

function loginSummary(instance: Design37Instance): string | null {
  const login = instance.login;
  if (!login) return null;
  if (login.state === "PENDING") {
    return login.browserState === "FAILED"
      ? "登录流程正在进行，但宿主打开浏览器失败。"
      : "宿主正在推进登录流程；此页面关闭后可重新打开查看进度。";
  }
  if (login.state === "CANCELLED") return "此实例的登录请求已取消。";
  if (login.state === "ERROR") return login.error || login.output || "登录失败，宿主未提供错误详情。";
  if (login.state === "LOGGED_IN") return "宿主已检测到登录成功。";
  if (login.state === "LOGGED_OUT") return "宿主检测到实例当前未登录。";
  return login.error || login.output || "宿主尚未确认登录状态。";
}

/** Minimal G.0 instance list. The host owns the login session and advances it independently of this view. */
export function Design37InstanceSection() {
  const [snapshot, setSnapshot] = useState<Design37InstancesSnapshot | null>(null);
  const [busyInstance, setBusyInstance] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const requestGeneration = useRef(0);
  const pollingRef = useRef(false);
  const busyRef = useRef(false);

  const readSnapshot = useCallback(async (generation: number) => {
    const result = await invoke<unknown>("gogoke_design37_instances");
    const parsed = readDesign37InstancesSnapshot(result);
    if (generation === requestGeneration.current) {
      setSnapshot(parsed);
      setError(null);
    }
  }, []);

  useEffect(() => {
    let active = true;
    const load = async () => {
      if (!active || pollingRef.current || busyRef.current) return;
      pollingRef.current = true;
      const generation = requestGeneration.current;
      try {
        await readSnapshot(generation);
      } catch (cause) {
        if (active && generation === requestGeneration.current) setError(errorText(cause));
      } finally {
        pollingRef.current = false;
        if (active) setLoading(false);
      }
    };

    void load();
    const timer = window.setInterval(() => void load(), 1000);
    return () => {
      active = false;
      window.clearInterval(timer);
    };
  }, [readSnapshot]);

  async function runAction(instanceId: string, command: string) {
    if (busyRef.current) return;
    busyRef.current = true;
    requestGeneration.current += 1;
    const generation = requestGeneration.current;
    setBusyInstance(instanceId);
    setError(null);
    setMessage(null);
    try {
      const result = await invoke<unknown>(command, { instanceId });
      const parsed = readDesign37InstancesSnapshot(result);
      if (generation === requestGeneration.current) setSnapshot(parsed);
    } catch (cause) {
      if (generation === requestGeneration.current) setError(errorText(cause));
    } finally {
      busyRef.current = false;
      if (generation === requestGeneration.current) setBusyInstance(null);
    }
  }

  async function copyDeviceCode(instance: Design37Instance) {
    const code = instance.login?.deviceCode;
    if (!code) return;
    setMessage(null);
    setError(null);
    try {
      await navigator.clipboard.writeText(code);
      setMessage(`已复制 ${instance.instanceId} 的设备码。`);
    } catch (cause) {
      setError(`复制设备码失败：${errorText(cause)}`);
    }
  }

  const instances = snapshot?.instances ?? [];
  const testInstance = instances.find((instance) => instance.instanceId === DESIGN37_TEST_INSTANCE_ID);

  return (
    <SettingsSection
      title="实例"
      subtitle="实例登录由宿主持续管理。可关闭此页面，之后重新打开即可读取同一实例的进度和结果。"
    >
      <div className="settings-field">
        <div className="settings-field-label settings-field-label--section">实例列表</div>
        {loading && !snapshot ? <div className="settings-help" role="status">正在读取实例…</div> : null}
        {snapshot && instances.length === 0 ? (
          <div className="settings-help">当前没有实例。</div>
        ) : null}

        {instances.map((instance) => {
          const busy = busyInstance === instance.instanceId;
          const pending = instance.login?.state === "PENDING";
          const unsettled = instance.login !== undefined && !instance.login.settled;
          const summary = loginSummary(instance);
          return (
            <div className="settings-toggle-row" key={instance.instanceId}>
              <div style={{ minWidth: 0, flex: 1, overflowWrap: "anywhere" }}>
                <div className="settings-toggle-title">
                  {instance.instanceId === DESIGN37_TEST_INSTANCE_ID
                    ? "Codex 测试实例"
                    : instance.instanceId}
                </div>
                <div className="settings-toggle-subtitle">
                  {instance.driverId} · {instance.version} · 修订 {instance.revision}
                </div>
                <div className="settings-help" role="status">
                  状态：{design37InstanceStateLabel(instance.state)}
                  {instance.login ? ` · 登录：${design37LoginStateLabel(instance.login.state)}` : ""}
                </div>
                {summary ? (
                  <div className={instance.login?.state === "ERROR" ? "settings-help settings-help-error" : "settings-help"}
                    role={instance.login?.state === "ERROR" ? "alert" : "status"}>
                    {summary}
                  </div>
                ) : null}
                {instance.login?.error && instance.login.state !== "ERROR" ? (
                  <div className="settings-help settings-help-error" role="alert">
                    {instance.login.error}
                  </div>
                ) : null}
                {instance.login?.authorizationUrl ? (
                  <div className="settings-help">
                    宿主授权地址：<code>{instance.login.authorizationUrl}</code>
                  </div>
                ) : null}
                {instance.login?.output ? (
                  <pre className="settings-help" aria-label={`${instance.instanceId} 登录输出`}
                    style={{ whiteSpace: "pre-wrap", overflowWrap: "anywhere", font: "inherit" }}>
                    {instance.login.output}
                  </pre>
                ) : null}
                {instance.login?.deviceCode ? (
                  <div className="settings-field-actions">
                    <span className="settings-help">设备码：<code>{instance.login.deviceCode}</code></span>
                    <button type="button" className="ghost settings-button-compact"
                      onClick={() => void copyDeviceCode(instance)}>
                      复制设备码
                    </button>
                  </div>
                ) : null}
                <div className="settings-field-actions">
                  <button type="button" className="primary settings-button-compact"
                    disabled={busyInstance !== null || unsettled || instance.state === "LOGGED_IN" || instance.state === "NOT_INSTALLED"}
                    onClick={() => void runAction(instance.instanceId, "gogoke_design37_instance_login")}>
                    {busy ? "正在启动…" : pending ? "登录进行中" : "一键登录"}
                  </button>
                  {unsettled ? (
                    <button type="button" className="ghost settings-button-compact" disabled={busyInstance !== null}
                      onClick={() => void runAction(instance.instanceId, "gogoke_design37_instance_cancel")}>
                      {busy ? "正在取消…" : "取消登录"}
                    </button>
                  ) : null}
                </div>
              </div>
            </div>
          );
        })}

        {snapshot ? (
          !testInstance ? (
            <div className="settings-field">
              <div className="settings-help">新建入口默认使用 {DESIGN37_TEST_INSTANCE_ID}。</div>
              <button type="button" className="primary settings-button-compact"
                disabled={busyInstance !== null || loading}
                onClick={() => void runAction(DESIGN37_TEST_INSTANCE_ID, "gogoke_design37_instance_register")}>
                {busyInstance === DESIGN37_TEST_INSTANCE_ID ? "正在创建…" : "创建 Codex 测试实例"}
              </button>
            </div>
          ) : testInstance.state === "NOT_INSTALLED" ? (
            <div className="settings-help">Codex 测试实例已登记，当前尚未安装。</div>
          ) : null
        ) : null}
      </div>
      {message ? <div className="settings-help" role="status">{message}</div> : null}
      {error ? <div className="settings-help settings-help-error" role="alert">{error}</div> : null}
    </SettingsSection>
  );
}
