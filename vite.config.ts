import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  test: {
    environment: "jsdom",
    setupFiles: "./src/test/setup.ts",
    // references/ 下的第三方仓库带大量同名测试文件，必须把范围锁在本项目 src
    include: ["src/**/*.test.{ts,tsx}"],
  },
});
