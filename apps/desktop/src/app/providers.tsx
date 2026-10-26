import { useState, type ReactNode } from 'react'
import { QueryClientProvider } from '@tanstack/react-query'
import { ThemeProvider } from '@/components/theme-provider'
import { Toaster } from '@/components/ui/sonner'
import { TooltipProvider } from '@/components/ui/tooltip'
import { useExternalLinks } from '@/hooks/use-external-links'
import { createQueryClient } from '@/lib/query-client'

function GlobalHandlers() {
  useExternalLinks()
  return null
}

export function AppProviders({ children }: { children: ReactNode }) {
  const [queryClient] = useState(createQueryClient)

  return (
    <QueryClientProvider client={queryClient}>
      <ThemeProvider>
        <TooltipProvider>
          {children}
          <Toaster />
          <GlobalHandlers />
        </TooltipProvider>
      </ThemeProvider>
    </QueryClientProvider>
  )
}
