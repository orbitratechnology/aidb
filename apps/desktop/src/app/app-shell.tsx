import { useState } from 'react'
import { Outlet } from 'react-router'
import { Button } from '@/components/ui/button'
import { SidebarInset, SidebarProvider } from '@/components/ui/sidebar'
import { useMediaQuery } from '@/hooks/use-media-query'
import { AppSidebar } from './app-sidebar'
import { BottomNav } from './bottom-nav'
import { TopBar } from './top-bar'

const DESKTOP_QUERY = '(min-width: 1024px)'

function focusMain() {
  document.getElementById('main-content')?.focus()
}

/**
 * Layout for every screen. Desktop (1024px+) shows an expanded sidebar, medium widths an
 * icon sidebar, and compact widths (below 640px) a bottom navigation bar.
 */
export function AppShell() {
  const isDesktop = useMediaQuery(DESKTOP_QUERY)
  const [sidebarOpen, setSidebarOpen] = useState(isDesktop)
  const [wasDesktop, setWasDesktop] = useState(isDesktop)

  // Re-sync the sidebar when the window crosses the desktop breakpoint; manual toggles
  // stick until then.
  if (wasDesktop !== isDesktop) {
    setWasDesktop(isDesktop)
    setSidebarOpen(isDesktop)
  }

  return (
    <SidebarProvider open={sidebarOpen} onOpenChange={setSidebarOpen}>
      <Button
        variant="outline"
        onClick={focusMain}
        className="sr-only focus:not-sr-only focus:fixed focus:top-2 focus:left-2"
      >
        Skip to content
      </Button>
      <AppSidebar />
      <SidebarInset className="h-dvh min-w-0 overflow-hidden">
        <TopBar />
        <main
          id="main-content"
          tabIndex={-1}
          className="flex min-h-0 flex-1 flex-col overflow-y-auto outline-none"
        >
          <Outlet />
        </main>
        <BottomNav />
      </SidebarInset>
    </SidebarProvider>
  )
}
