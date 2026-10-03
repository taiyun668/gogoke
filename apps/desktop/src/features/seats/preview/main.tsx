import { useState } from "react";
import { createRoot } from "react-dom/client";
import { createPreviewHost } from "./host";
import "../../../styles/base.css";
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
      <button className="ghost" onClick={() => { host.settle(true); refresh(); }}>模拟授权成功</button>
      <button className="ghost" onClick={() => { host.settle(false); refresh(); }}>模拟授权失败</button>
      <button className="ghost" onClick={() => setMounted(value => !value)}>{mounted ? "关闭实例页" : "重开实例页"}</button>
    </div>
    {mounted ? <Design37InstanceSection key={generation} /> : <p role="status">实例页已关闭，预览宿主持有当前登录状态。</p>}
  </main>;
}

document.body.style.overflow = "auto";
document.body.style.background = "var(--surface-card)";
document.getElementById("root")!.style.overflow = "visible";
createRoot(document.getElementById("root")!).render(<Preview />);
