import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import './index.css'
import App from './App.tsx'
import { AppProviders } from '@/app/providers'
import { applyTheme, readStoredTheme, resolveTheme, systemTheme } from '@/lib/theme'

// Apply the saved theme before the first render to avoid a light flash in dark mode.
applyTheme(resolveTheme(readStoredTheme(), systemTheme()))

const root = document.getElementById('root')
if (!root) {
  throw new Error('missing #root element')
}

createRoot(root).render(
  <StrictMode>
    <AppProviders>
      <App />
    </AppProviders>
  </StrictMode>,
)
