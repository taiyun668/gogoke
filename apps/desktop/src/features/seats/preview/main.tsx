import { useState } from "react";
import { createRoot } from "react-dom/client";
import { createPreviewHost } from "./host";
import { Design37OwnerNoticePresenter } from "../Design37OwnerNoticePresenter";
import { requestDesign37OwnerNotices, type Design37OwnerNotice } from "../design37OwnerNotices";
import "../../../styles/base.css";
import "../../../styles/ds-tokens.css";
import "../../../styles/ds-modal.css";
import "../../../styles/buttons.css";
import "../../../styles/settings.css";

if (!import.meta.env.DEV || "__TAURI_INTERNALS__" in window) {
  throw new Error("INSTANCE_PREVIEW_REQUIRES_DEV_BROWSER_WITHOUT_NATIVE_BRIDGE");
}
const host = createPreviewHost();
Object.defineProperty(window, "__TAURI_INTERNALS__", { value: { invoke: host.invoke } });
const { Design37InstanceSection } = await import("../Design37InstanceSection");

function Preview() {
  const [mounted, setMounted] = useState(true);
  const [generation, setGeneration] = useState(0);
  const [showNewVersion, setShowNewVersion] = useState(false);
  const [showRuntimeIssues, setShowRuntimeIssues] = useState(false);
  const [ownerNotices, setOwnerNotices] = useState<Design37OwnerNotice[]>([]);
  const [noticeUiLifetime, setNoticeUiLifetime] = useState(0);
  const [ownerNoticeError, setOwnerNoticeError] = useState<string | null>(null);
  const showOwnerNotice = async (visible: boolean, newCause = false) => {
    host.setOwnerNotice(visible, newCause);
    try {
      setOwnerNotices(await requestDesign37OwnerNotices(host.executeUserSourceOperation, `previewOwner_${crypto.randomUUID()}`));
      setOwnerNoticeError(null);
    } catch (cause) {
      setOwnerNotices([]);
      setOwnerNoticeError(cause instanceof Error ? cause.message : String(cause));
    }
  };
  const refresh = () => setGeneration(value => value + 1);
  const choose: typeof host.setState = next => { host.setState(next); refresh(); };
  return <main style={{ margin: "0 auto", maxWidth: 980, padding: 24, minHeight: "100dvh" }}>
    <h1>实例页预览</h1>
    <p>这里只用 K-UI 假实现预览交互，不会登录真实账号，也不读取安装或凭据。</p>
    <div className="settings-field-actions" aria-label="预览场景">
      <button className="ghost" onClick={() => choose("ABSENT")}>未登记</button>
      <button className="ghost" onClick={() => choose("NOT_INSTALLED")}>未安装</button>
      <button className="ghost" onClick={() => choose("NOT_LOGGED_IN")}>未登录</button>
      <button className="ghost" onClick={() => choose("LOGGED_IN")}>已登录</button>
      <button className="ghost" onClick={() => choose("ERROR")}>出错</button>
      <button className="ghost" onClick={() => {
        host.setNewVersion(!showNewVersion);
        setShowNewVersion(!showNewVersion);
        refresh();
      }}>{showNewVersion ? "隐藏较新版本提示" : "显示较新版本提示"}</button>
      <button className="ghost" onClick={() => {
        host.setRuntimeIssues(!showRuntimeIssues);
        setShowRuntimeIssues(!showRuntimeIssues);
        refresh();
      }}>{showRuntimeIssues ? "隐藏 CLI 错误（假）" : "显示两席位 CLI 错误（假）"}</button>
      <button className="ghost" onClick={() => { host.settle(true); refresh(); }}>模拟授权成功</button>
      <button className="ghost" onClick={() => { host.settle(false); refresh(); }}>模拟授权失败</button>
      <button className="ghost" onClick={() => setMounted(value => !value)}>{mounted ? "关闭实例页" : "重开实例页"}</button>
    </div>
    {mounted ? <Design37InstanceSection key={generation} /> : <p role="status">实例页已关闭，预览宿主持有当前登录状态。</p>}
    <section style={{ marginTop: 32 }} aria-label="Owner 通知预览">
      <h2>Owner 通知预览（假）</h2>
      <p>通知来自假 User 投影；关闭只隐藏当前界面。C 消息仍为 PENDING，此预览不产生持久 Owner ACK。</p>
      <div className="settings-field-actions">
        <button className="ghost" onClick={() => void showOwnerNotice(true)}>显示同一原因（假）</button>
        <button className="ghost" onClick={() => void showOwnerNotice(false)}>撤销路由（假）</button>
        <button className="ghost" onClick={() => {
          void showOwnerNotice(true, true);
          setTimeout(() => void showOwnerNotice(false), 3000);
        }}>显示后自动撤销路由（假）</button>
        <button className="ghost" onClick={() => void showOwnerNotice(true, true)}>显示新原因（假）</button>
        <button className="ghost" onClick={() => setNoticeUiLifetime(value => value + 1)}>重启通知界面（假）</button>
      </div>
      <p role="status">当前投影：{ownerNotices.length ? "1 条 PENDING 消息" : "无通知"}</p>
      {ownerNoticeError && <p role="alert">{ownerNoticeError}</p>}
    </section>
    <Design37OwnerNoticePresenter key={noticeUiLifetime} notices={ownerNotices} />
  </main>;
}

document.body.style.overflow = "auto";
document.body.style.background = "var(--surface-card)";
document.getElementById("root")!.style.overflow = "visible";
createRoot(document.getElementById("root")!).render(<Preview />);
