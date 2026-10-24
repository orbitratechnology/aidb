import { useLocation } from 'react-router'
import { AppMark } from '@/components/shared/app-mark'
import { ConnectionStatus } from '@/components/shared/connection-status'
import { Separator } from '@/components/ui/separator'
import { SidebarTrigger } from '@/components/ui/sidebar'
import { AppMenu } from './app-menu'
import { destinationTitle } from './navigation'

export function TopBar() {
  const { pathname } = useLocation()

  return (
    <header className="flex min-h-14 shrink-0 items-center gap-2 border-b bg-background px-3 pt-[env(safe-area-inset-top)] sm:px-4">
      <SidebarTrigger className="hidden sm:inline-flex" />
      <Separator orientation="vertical" className="hidden data-[orientation=vertical]:h-4 sm:block" />
      <AppMark className="sm:hidden" />
      <h1 className="truncate text-base font-medium">{destinationTitle(pathname)}</h1>
      <div className="ml-auto flex min-w-0 items-center gap-1">
        <ConnectionStatus />
        <div className="sm:hidden">
          <AppMenu />
        </div>
      </div>
    </header>
  )
}
