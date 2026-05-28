import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  // 環境変数 .env.local が読み込まれる
  globalSetup: "./tests/global-setup.ts",
  testDir: "tests",
  // fullyParallel: true,
  reporter: "html",
  use: {
    launchOptions: {
      args: [
        "--use-fake-ui-for-media-stream",
        "--use-fake-device-for-media-stream",
        // "--use-file-for-fake-video-capture=/app/sample.mjpeg",
      ],
    },
  },
  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
    },

    // {
    //   name: 'firefox',
    //   use: { ...devices['Desktop Firefox'] },
    // },

    // {
    //   name: 'webkit',
    //   use: { ...devices['Desktop Safari'] },
    // },
  ],
  webServer: {
    // vite で起動している
    command: "pnpm run dev",
    url: "http://localhost:5173/",
    reuseExistingServer: !process.env.CI,
    stdout: "pipe",
    stderr: "pipe",
  },
});
