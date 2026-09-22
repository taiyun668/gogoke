import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { isMobilePlatform } from "./utils/platformPaths";

function disableMobileZoomGestures() {
  if (!isMobilePlatform() || typeof document === "undefined") {
    return;
  }
  const preventGesture = (event: Event) => event.preventDefault();
  const preventPinch = (event: TouchEvent) => {
    if (event.touches.length > 1) {
      event.preventDefault();
    }
  };

  document.addEventListener("gesturestart", preventGesture, { passive: false });
  document.addEventListener("gesturechange", preventGesture, { passive: false });
  document.addEventListener("gestureend", preventGesture, { passive: false });
  document.addEventListener("touchmove", preventPinch, { passive: false });
}

function syncMobileViewportHeight() {
  if (!isMobilePlatform() || typeof window === "undefined" || typeof document === "undefined") {
    return;
  }

  let rafHandle = 0;

  const setViewportHeight = () => {
    const visualViewport = window.visualViewport;
    const viewportHeight = visualViewport
      ? visualViewport.height + visualViewport.offsetTop
      : window.innerHeight;
    const nextHeight = Math.round(viewportHeight);
    document.documentElement.style.setProperty("--app-height", `${nextHeight}px`);
  };

  const scheduleViewportHeight = () => {
    if (rafHandle) {
      return;
    }
    rafHandle = window.requestAnimationFrame(() => {
      rafHandle = 0;
      setViewportHeight();
    });
  };

  const setComposerFocusState = () => {
    const activeElement = document.activeElement;
    const isComposerTextareaFocused =
      activeElement instanceof HTMLTextAreaElement &&
      activeElement.closest(".composer") !== null;
    document.documentElement.dataset.mobileComposerFocus = isComposerTextareaFocused
      ? "true"
      : "false";
  };

  setViewportHeight();
  setComposerFocusState();
  window.addEventListener("resize", scheduleViewportHeight, { passive: true });
  window.addEventListener("orientationchange", scheduleViewportHeight, { passive: true });
  window.visualViewport?.addEventListener("resize", scheduleViewportHeight, { passive: true });
  window.visualViewport?.addEventListener("scroll", scheduleViewportHeight, { passive: true });
  document.addEventListener("focusin", setComposerFocusState);
  document.addEventListener("focusout", () => {
    requestAnimationFrame(setComposerFocusState);
  });
}

disableMobileZoomGestures();
syncMobileViewportHeight();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
