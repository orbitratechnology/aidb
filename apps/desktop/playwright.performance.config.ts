import { defineConfig, devices } from '@playwright/test'

// Performance tests: the real interface, production-built, on the fixture core with a large
// database (e2e/performance). One worker, so timings are not skewed by parallel tests.
const port = 4174

export default defineConfig({
  testDir: './e2e/performance',
  testMatch: '**/*.spec.ts',
  workers: 1,
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
    command:
      'pnpm exec vite build --config e2e/performance/vite.config.ts && pnpm exec vite preview --config e2e/performance/vite.config.ts',
    url: `http://localhost:${port}/e2e/app.html`,
    reuseExistingServer: !process.env.CI,
    timeout: 180_000,
  },
})
