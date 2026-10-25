import type { ReactNode } from 'react'
import { ThemeProvider } from '@/components/theme-provider'
import { Toaster } from '@/components/ui/sonner'
import { TooltipProvider } from '@/components/ui/tooltip'
import { useExternalLinks } from '@/hooks/use-external-links'

function GlobalHandlers() {
  useExternalLinks()
  return null
}

export function AppProviders({ children }: { children: ReactNode }) {
  return (
    <ThemeProvider>
      <TooltipProvider>
        {children}
        <Toaster />
        <GlobalHandlers />
      </TooltipProvider>
    </ThemeProvider>
  )
}
