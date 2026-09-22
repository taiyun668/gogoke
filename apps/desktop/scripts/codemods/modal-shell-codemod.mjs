import {
  ensureNamedImport,
  importPathToDesignSystem,
  normalizeClassTokens,
  runCodemod,
} from "./utils.mjs";

const DEFAULT_FILES = [
  "src/features/workspaces/components/ClonePrompt.tsx",
  "src/features/workspaces/components/WorktreePrompt.tsx",
  "src/features/git/components/BranchSwitcherPrompt.tsx",
  "src/features/threads/components/RenameThreadPrompt.tsx",
  "src/features/settings/components/SettingsView.tsx",
];

const TOKEN_TO_DS_CLASS = new Map([
  ["worktree-modal-title", "ds-modal-title"],
  ["worktree-modal-subtitle", "ds-modal-subtitle"],
  ["worktree-modal-label", "ds-modal-label"],
  ["worktree-modal-input", "ds-modal-input"],
  ["worktree-modal-textarea", "ds-modal-textarea"],
  ["worktree-modal-divider", "ds-modal-divider"],
  ["worktree-modal-actions", "ds-modal-actions"],
  ["worktree-modal-error", "ds-modal-error"],
  ["worktree-modal-button", "ds-modal-button"],
  ["clone-modal-title", "ds-modal-title"],
  ["clone-modal-subtitle", "ds-modal-subtitle"],
  ["clone-modal-label", "ds-modal-label"],
  ["clone-modal-input", "ds-modal-input"],
  ["clone-modal-actions", "ds-modal-actions"],
  ["clone-modal-error", "ds-modal-error"],
  ["clone-modal-button", "ds-modal-button"],
]);

function resolveModalDsClass(token) {
  return TOKEN_TO_DS_CLASS.get(token) ?? null;
}

runCodemod({
  name: "modal-shell-codemod",
  defaultFiles: DEFAULT_FILES,
  transform(source, filePath) {
    let updated = source;
    const usesModalShell = updated.includes("<ModalShell");
    if (usesModalShell) {
      const importPath = importPathToDesignSystem(
        filePath,
        "src/features/design-system/components/modal/ModalShell",
      );
      updated = ensureNamedImport(updated, importPath, ["ModalShell"]);
    }
    const hasDsModalClasses = /className="[^"]*ds-modal-/.test(updated);
    if (!hasDsModalClasses) {
      updated = normalizeClassTokens(updated, resolveModalDsClass);
    }
    return updated;
  },
});
