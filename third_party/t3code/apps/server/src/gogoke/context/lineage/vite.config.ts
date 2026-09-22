import "vite-plus/test/config";
import { defineConfig } from "vite-plus";

export default defineConfig({
  test: {
    environment: "node",
  },
  staged: {
    "*": "vp fmt --no-error-on-unmatched-pattern",
  },
});
