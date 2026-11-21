import { fileURLToPath, URL } from 'node:url'
import { defineConfig, mergeConfig } from 'vite'
import base from '../../vite.config'

// The test harness built like the app ships (production React, minified), so the
// performance tests measure what users run rather than the development server.
export default mergeConfig(
  base,
  defineConfig({
    root: fileURLToPath(new URL('../..', import.meta.url)),
    build: {
      outDir: 'dist-e2e',
      emptyOutDir: true,
      rolldownOptions: {
        input: fileURLToPath(new URL('../app.html', import.meta.url)),
      },
    },
    preview: { port: 4174, strictPort: true },
  }),
)
