import { defineConfig, devices } from '@playwright/test'

// End-to-end tests of the UI: the real app in Chromium, with Tauri's IPC mocks answering from
// fixtures (e2e/harness). The native core is tested in Rust; these check layout, keyboard use
// and accessibility at every width.
const port = 4173

export default defineConfig({
  testDir: './e2e',
  testMatch: '**/*.spec.ts',
  // Performance tests run against a production build, with their own config.
  testIgnore: 'performance/**',
  fullyParallel: true,
  forbidOnly: Boolean(process.env.CI),
  retries: 0,
  reporter: process.env.CI ? [['list'], ['github']] : 'list',
  use: {
    baseURL: `http://localhost:${port}`,
    trace: 'off',
    screenshot: 'only-on-failure',
    ...devices['Desktop Chrome'],
  },
  webServer: {
    command: `pnpm exec vite --port ${port} --strictPort`,
    url: `http://localhost:${port}/e2e/app.html`,
    reuseExistingServer: !process.env.CI,
    timeout: 60_000,
  },
})
