import {
  ensureNamedImport,
  importPathToDesignSystem,
  normalizeClassTokens,
  runCodemod,
} from "./utils.mjs";

const DEFAULT_FILES = [
  "src/features/app/components/ApprovalToasts.tsx",
  "src/features/notifications/components/ErrorToasts.tsx",
  "src/features/update/components/UpdateToast.tsx",
];

const TOAST_PRIMITIVE_NAMES = [
  "ToastViewport",
  "ToastCard",
  "ToastTitle",
  "ToastBody",
  "ToastHeader",
  "ToastActions",
  "ToastError",
];

function resolveToastDsClass(token) {
  if (token === "approval-toasts" || token === "error-toasts" || token === "update-toasts") {
    return "ds-toast-viewport";
  }
  if (token === "approval-toast" || token === "error-toast" || token === "update-toast") {
    return "ds-toast-card";
  }
  if (token.endsWith("-toast-title")) {
    return "ds-toast-title";
  }
  if (token.endsWith("-toast-body")) {
    return "ds-toast-body";
  }
  if (token.endsWith("-toast-header")) {
    return "ds-toast-header";
  }
  if (token.endsWith("-toast-actions")) {
    return "ds-toast-actions";
  }
  if (token.endsWith("-toast-error")) {
    return "ds-toast-error";
  }
  return null;
}

runCodemod({
  name: "toast-shell-codemod",
  defaultFiles: DEFAULT_FILES,
  transform(source, filePath) {
    let updated = source;
    const usedPrimitives = TOAST_PRIMITIVE_NAMES.filter((name) =>
      updated.includes(`<${name}`),
    );
    if (usedPrimitives.length) {
      const importPath = importPathToDesignSystem(
        filePath,
        "src/features/design-system/components/toast/ToastPrimitives",
      );
      updated = ensureNamedImport(updated, importPath, usedPrimitives);
    }
    const alreadyUsesToastPrimitives =
      updated.includes("<ToastViewport") &&
      updated.includes("<ToastCard") &&
      (updated.includes("<ToastTitle") || updated.includes("<ToastBody"));
    if (!alreadyUsesToastPrimitives) {
      updated = normalizeClassTokens(updated, resolveToastDsClass);
    }
    return updated;
  },
});
