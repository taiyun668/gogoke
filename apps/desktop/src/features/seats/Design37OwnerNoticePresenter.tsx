import { useId, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { ModalShell } from "@/features/design-system/components/modal/ModalShell";
import { design37OwnerNoticeCauseKey, type Design37OwnerNotice } from "./design37OwnerNotices";

type Design37OwnerNoticePresenterProps = { notices: readonly Design37OwnerNotice[] };

/** Keep mounted across route changes. Hiding a cause is local to this UI lifetime. */
export function Design37OwnerNoticePresenter({ notices }: Design37OwnerNoticePresenterProps) {
  const presentedCauses = useRef(new Set<string>());
  const [activeCause, setActiveCause] = useState<string | null>(null);
  const notice = notices.find(item => design37OwnerNoticeCauseKey(item) === activeCause) ??
    notices.find(item => !presentedCauses.current.has(design37OwnerNoticeCauseKey(item)));
  const causeKey = notice ? design37OwnerNoticeCauseKey(notice) : null;
  useLayoutEffect(() => {
    if (causeKey) presentedCauses.current.add(causeKey);
    setActiveCause(causeKey);
  }, [causeKey]);
  if (!notice) return null;
  return <OwnerNoticeDialog key={causeKey} notice={notice} onClose={() => setActiveCause(null)} />;
}

function OwnerNoticeDialog({ notice, onClose }: { notice: Design37OwnerNotice; onClose: () => void }) {
  const titleId = useId();
  const container = useRef<HTMLDivElement>(null);
  const title = useRef<HTMLHeadingElement>(null);
  const close = useRef<HTMLButtonElement>(null);
  const closeHandler = useRef(onClose);
  closeHandler.current = onClose;

  useLayoutEffect(() => {
    const root = container.current!;
    const previousFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const siblings = Array.from(document.body.children).filter(
      (element): element is HTMLElement => element instanceof HTMLElement && element !== root,
    );
    const previousInert = siblings.map(element => element.inert);
    siblings.forEach(element => { element.inert = true; });
    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    title.current?.focus();
    const keydown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopImmediatePropagation();
        closeHandler.current();
      } else if (event.key === "Tab") {
        // This information dialog has one interactive control.
        event.preventDefault();
        event.stopImmediatePropagation();
        close.current?.focus();
      }
    };
    const focusin = (event: FocusEvent) => {
      if (event.target instanceof Node && !root.contains(event.target)) close.current?.focus();
    };
    document.addEventListener("keydown", keydown, true);
    document.addEventListener("focusin", focusin, true);
    return () => {
      document.removeEventListener("keydown", keydown, true);
      document.removeEventListener("focusin", focusin, true);
      siblings.forEach((element, index) => { element.inert = previousInert[index]; });
      document.body.style.overflow = previousOverflow;
      if (previousFocus?.isConnected && !previousFocus.closest("[inert]")) previousFocus.focus();
    };
  }, []);

  return createPortal(<div ref={container}>
    <ModalShell ariaLabelledBy={titleId} onBackdropClick={onClose}>
      <div style={{ width: "min(520px, calc(100vw - 32px))", padding: 24 }}>
        <h2 ref={title} id={titleId} tabIndex={-1} className="ds-modal-title" style={{ margin: "0 0 16px" }}>
          席位需要处理
        </h2>
        <div style={{ maxHeight: "min(60dvh, 480px)", overflow: "auto", overflowWrap: "anywhere" }}>
          <p className="ds-modal-subtitle" style={{ margin: "0 0 12px" }}>来自席位 {notice.sourceSeatId}</p>
          <p style={{ whiteSpace: "pre-wrap", margin: 0, lineHeight: 1.6 }}>{notice.body}</p>
        </div>
        <div className="ds-modal-actions" style={{ marginTop: 20 }}>
          <button ref={close} type="button" className="ghost" onClick={onClose}
            style={{ outline: "2px solid var(--ds-modal-focus-ring)", outlineOffset: 2 }}>关闭</button>
        </div>
      </div>
    </ModalShell>
  </div>, document.body);
}
