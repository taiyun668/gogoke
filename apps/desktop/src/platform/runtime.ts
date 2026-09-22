import { isTauri } from "@tauri-apps/api/core";

export type AppRuntime = "tauri" | "browser-preview";

export function appRuntime(): AppRuntime {
  return isTauri() ? "tauri" : "browser-preview";
}

export function hasNativeBackendTransport(): boolean {
  return appRuntime() === "tauri";
}
