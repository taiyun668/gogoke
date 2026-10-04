import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  ToastActions, ToastBody, ToastCard, ToastTitle, ToastViewport,
} from "@/features/design-system/components/toast/ToastPrimitives";
import { Design37OwnerNoticePresenter } from "./Design37OwnerNoticePresenter";
import {
  requestDesign37OwnerNotices,
  type Design37OwnerNotice,
  type Design37OwnerNoticesSourceOperation,
} from "./design37OwnerNotices";

const HOST_NOT_STARTED = "GOGOKE_DESIGN37_USER_HOST_NOT_STARTED";

function errorText(cause: unknown): string {
  return cause instanceof Error ? cause.message : String(cause);
}

const executeUserSourceOperation: Design37OwnerNoticesSourceOperation = async request => {
  const raw = await invoke<string>("gogoke_design37_user_operation", { frame: JSON.stringify(request) });
  try {
    return JSON.parse(raw) as unknown;
  } catch (cause) {
    throw new Error(`OWNER_NOTICE_USER_RECEIPT_JSON_FAILED: ${errorText(cause)}; response: ${raw}`);
  }
};

type Design37OwnerNoticeHostProps = {
  /** Browser preview supplies only its explicit fake source. Production uses the retained User pipe. */
  executeSourceOperation?: Design37OwnerNoticesSourceOperation;
};

/** App mounts this once in its main window, outside individual app routes. */
export function Design37OwnerNoticeHost({ executeSourceOperation = executeUserSourceOperation }: Design37OwnerNoticeHostProps) {
  const [notices, setNotices] = useState<Design37OwnerNotice[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const inFlight = useRef(false);
  const refresh = useRef<(() => Promise<void>) | null>(null);

  useEffect(() => {
    let active = true;
    const load = async () => {
      if (!active || inFlight.current) return;
      inFlight.current = true;
      setRefreshing(true);
      try {
        const current = await requestDesign37OwnerNotices(executeSourceOperation, `ownerNotice_${crypto.randomUUID()}`);
        if (active) {
          setNotices(current);
          setError(null);
        }
      } catch (cause) {
        if (active) {
          setNotices([]);
          setError(errorText(cause));
        }
      } finally {
        inFlight.current = false;
        if (active) setRefreshing(false);
      }
    };
    refresh.current = load;
    void load();
    // Match the existing instance page's read cadence, without concurrent reads.
    const timer = window.setInterval(() => void load(), 1000);
    return () => {
      active = false;
      window.clearInterval(timer);
      if (refresh.current === load) refresh.current = null;
    };
  }, [executeSourceOperation]);

  const unavailable = error === HOST_NOT_STARTED;
  return <>
    <Design37OwnerNoticePresenter notices={notices} />
    {error && <ToastViewport className="error-toasts" aria-label="Owner 通知状态">
      <ToastCard className="error-toast" role={unavailable ? "status" : "alert"}>
        <ToastTitle className="error-toast-title">{unavailable ? "Owner 通知暂不可用" : "Owner 通知读取失败"}</ToastTitle>
        <ToastBody className="error-toast-body">
          {unavailable && <p>宿主尚未启动，当前没有可读取的通知投影。</p>}
          <div style={{ whiteSpace: "pre-wrap" }}>{error}</div>
        </ToastBody>
        <ToastActions style={{ marginTop: 8 }}>
          <button type="button" className="ghost" disabled={refreshing} onClick={() => void refresh.current?.()}>
            {refreshing ? "正在刷新…" : "刷新"}
          </button>
        </ToastActions>
      </ToastCard>
    </ToastViewport>}
  </>;
}
