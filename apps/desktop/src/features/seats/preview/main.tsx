import { useState } from "react";
import { createRoot } from "react-dom/client";
import { createPreviewHost } from "./host";
import { Design37OwnerNoticePresenter } from "../Design37OwnerNoticePresenter";
import { Design37OwnerNoticeHost } from "../Design37OwnerNoticeHost";
import { requestDesign37OwnerNotices, type Design37OwnerNotice } from "../design37OwnerNotices";
import "../../../styles/base.css";
import "../../../styles/ds-tokens.css";
import "../../../styles/ds-modal.css";
import "../../../styles/ds-toast.css";
import "../../../styles/error-toasts.css";
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
  const [useNoticeHost, setUseNoticeHost] = useState(false);
  const [noticeHostMounted, setNoticeHostMounted] = useState(true);
  const [noticeReadStats, setNoticeReadStats] = useState(host.ownerNoticeReadStats());
  const showOwnerNotice = async (visible: boolean, newCause = false) => {
    host.setOwnerNotice(visible, newCause);
    if (useNoticeHost) return;
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
      <p role="status">当前投影：{useNoticeHost ? "由 Host 自动读取" : ownerNotices.length ? "1 条 PENDING 消息" : "无通知"}</p>
      {ownerNoticeError && <p role="alert">{ownerNoticeError}</p>}
      <h3>通知 Host 生命周期（假）</h3>
      <p>Host 接入相同的假 User 源，按现有实例页节奏读取；不会调用真实 native 桥。</p>
      <div className="settings-field-actions">
        <button className="ghost" onClick={() => setUseNoticeHost(value => !value)}>
          {useNoticeHost ? "切回纯 Presenter（假）" : "使用 Host 自动刷新（假）"}
        </button>
        <button className="ghost" onClick={() => setNoticeHostMounted(value => !value)}>
          {noticeHostMounted ? "卸载通知 Host（假）" : "重开通知 Host（假）"}
        </button>
        <button className="ghost" onClick={() => host.setOwnerNoticeFailure("GOGOKE_DESIGN37_USER_HOST_NOT_STARTED")}>宿主未启动（假）</button>
        <button className="ghost" onClick={() => host.setOwnerNoticeFailure("PREVIEW_OWNER_SOURCE_FAILED: synthetic original error")}>原始读取错误（假）</button>
        <button className="ghost" onClick={() => host.setOwnerNoticeFailure(null)}>恢复源（假）</button>
        <button className="ghost" onClick={() => host.setOwnerNoticeDelay(2200)}>启用慢响应（假）</button>
        <button className="ghost" onClick={() => host.setOwnerNoticeDelay(0)}>恢复即时响应（假）</button>
        <button className="ghost" onClick={() => setNoticeReadStats(host.ownerNoticeReadStats())}>读取假源调用记录</button>
        <button className="ghost" onClick={() => {
          host.setOwnerNotice(true, true);
          host.setOwnerNoticeDelay(2200);
          setTimeout(() => setNoticeHostMounted(false), 1300);
        }}>慢读取开始后卸载（假）</button>
      </div>
      <p role="status">假源调用记录：请求 {noticeReadStats.reads}，完成 {noticeReadStats.completed}，在途 {noticeReadStats.inFlight}，最大并发 {noticeReadStats.maxInFlight}</p>
    </section>
    {useNoticeHost
      ? noticeHostMounted && <Design37OwnerNoticeHost key={noticeUiLifetime} executeSourceOperation={host.executeUserSourceOperation} />
      : <Design37OwnerNoticePresenter key={noticeUiLifetime} notices={ownerNotices} />}
  </main>;
}

document.body.style.overflow = "auto";
document.body.style.background = "var(--surface-card)";
document.getElementById("root")!.style.overflow = "visible";
createRoot(document.getElementById("root")!).render(<Preview />);
