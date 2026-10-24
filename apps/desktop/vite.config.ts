/// <reference types="vitest/config" />
import { fileURLToPath, URL } from 'node:url'
import tailwindcss from '@tailwindcss/vite'
import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

// Tauri sets these when it runs the dev server or build.
const host = process.env.TAURI_DEV_HOST
const isDebug = Boolean(process.env.TAURI_ENV_DEBUG)

// https://vite.dev/config/
export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      '@': fileURLToPath(new URL('./src', import.meta.url)),
    },
  },
  // Keep Rust compiler output visible in the terminal.
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    host: host ?? 'localhost',
    hmr: host ? { protocol: 'ws', host, port: 5174 } : undefined,
  },
  envPrefix: ['VITE_', 'TAURI_ENV_'],
  build: {
    // Tailwind CSS v4 needs Safari 16.4+ / Chromium 111+ webviews.
    target: process.env.TAURI_ENV_PLATFORM === 'windows' ? 'chrome111' : 'safari16.4',
    minify: !isDebug,
    sourcemap: isDebug,
    // Assets load from local disk, not the network. The shell (React, router, Radix) is ~520 kB;
    // feature screens are split per route. Raise this only with a reason.
    chunkSizeWarningLimit: 600,
  },
  test: {
    // Unit tests cover pure logic only; UI behavior is checked end to end.
    environment: 'node',
    include: ['src/**/*.test.ts'],
  },
})
