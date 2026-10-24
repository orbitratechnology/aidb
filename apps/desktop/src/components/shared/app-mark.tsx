import { cn } from '@/lib/utils'
import { DatabaseIcon } from 'lucide-react'

export function AppMark({ className }: { className?: string }) {
  return (
    <div
      aria-hidden
      className={cn(
        'flex aspect-square size-8 shrink-0 items-center justify-center rounded-lg bg-sidebar-primary text-sidebar-primary-foreground',
        className,
      )}
    >
      <DatabaseIcon className="size-4" />
    </div>
  )
}
