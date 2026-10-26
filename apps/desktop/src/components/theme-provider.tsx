import { useCallback, useEffect, useLayoutEffect, useMemo, useState, type ReactNode } from 'react'
import { toast } from 'sonner'
import { useSettings, useUpdateSettings } from '@/hooks/use-settings'
import { ThemeContext } from '@/hooks/use-theme'
import {
  applyTheme,
  readStoredTheme,
  resolveTheme,
  storeTheme,
  systemTheme,
  watchSystemTheme,
  type ResolvedTheme,
  type Theme,
} from '@/lib/theme'

type ThemeProviderProps = {
  children: ReactNode
  defaultTheme?: Theme
}

/**
 * The theme saved in the local database wins. Until it loads, or when it cannot be saved, the
 * cached copy and then the user's latest choice apply for this session.
 */
export function ThemeProvider({ children, defaultTheme = 'system' }: ThemeProviderProps) {
  const settings = useSettings()
  const { mutate, isPending, variables } = useUpdateSettings()
  const [sessionTheme, setSessionTheme] = useState<Theme>(() => readStoredTheme(defaultTheme))
  const [system, setSystem] = useState<ResolvedTheme>(systemTheme)

  useEffect(() => watchSystemTheme(setSystem), [])

  const savedAppearance = settings.data?.appearance
  const savedTheme = savedAppearance?.theme
  const pendingTheme = isPending ? variables.appearance?.theme : undefined
  const theme = pendingTheme ?? savedTheme ?? sessionTheme
  const resolvedTheme = resolveTheme(theme, system)

  useLayoutEffect(() => {
    applyTheme(resolvedTheme)
  }, [resolvedTheme])

  useEffect(() => {
    if (savedTheme) storeTheme(savedTheme)
  }, [savedTheme])

  const setTheme = useCallback(
    (next: Theme) => {
      setSessionTheme(next)
      mutate(
        { appearance: { ...savedAppearance, theme: next } },
        {
          onError: (error) =>
            toast.error("Couldn't save the theme", { description: error.message }),
        },
      )
    },
    [mutate, savedAppearance],
  )

  const value = useMemo(
    () => ({ theme, resolvedTheme, setTheme }),
    [theme, resolvedTheme, setTheme],
  )

  return <ThemeContext value={value}>{children}</ThemeContext>
}
