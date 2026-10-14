import { defineConfig, devices } from '@playwright/test'

export default defineConfig({
  testDir: './tests',
  timeout: 30000,
  use: { baseURL: 'http://127.0.0.1:4173', ...devices['Desktop Chrome'] },
  webServer: { command: 'npm run preview', url: 'http://127.0.0.1:4173/docs/', reuseExistingServer: true },
  reporter: [['list'], ['json', { outputFile: 'verification/browser-results.json' }]]
})
