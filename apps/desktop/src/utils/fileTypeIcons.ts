import { getIconUrlForFilePath } from "vscode-material-icons";

const iconUrlCache = new Map<string, string>();

export function getFileTypeIconUrl(path: string): string {
  const normalizedPath = path.replace(/\\/g, "/");
  const cached = iconUrlCache.get(normalizedPath);
  if (cached) {
    return cached;
  }
  const baseUrl = new URL("./assets/material-icons", window.location.href).pathname;
  const iconUrl = getIconUrlForFilePath(normalizedPath, baseUrl);
  iconUrlCache.set(normalizedPath, iconUrl);
  return iconUrl;
}
