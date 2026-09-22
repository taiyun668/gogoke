import "vite-plus/test/config";
import { defineConfig } from "vite-plus";

/** Sparse-worktree test config: the donor root setup file is outside this task's checkout. */
export default defineConfig({
  test: {
    environment: "node",
  },
});
