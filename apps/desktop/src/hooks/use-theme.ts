import { createContext, useContext } from 'react'
import type { ResolvedTheme, Theme } from '@/lib/theme'

export type ThemeContextValue = {
  /** The user's choice, including `system`. */
  theme: Theme
  /** The theme actually applied after resolving `system`. */
  resolvedTheme: ResolvedTheme
  setTheme: (theme: Theme) => void
}

export const ThemeContext = createContext<ThemeContextValue | null>(null)

export function useTheme(): ThemeContextValue {
  const value = useContext(ThemeContext)
  if (!value) {
    throw new Error('useTheme must be used inside <ThemeProvider>')
  }
  return value
}
