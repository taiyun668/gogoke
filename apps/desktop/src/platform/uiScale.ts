import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";

export async function applyUiScale(scale: number): Promise<void> {
  if (!isTauri()) {
    document.documentElement.style.zoom = String(scale);
    return;
  }
  await getCurrentWebview().setZoom(scale);
}
