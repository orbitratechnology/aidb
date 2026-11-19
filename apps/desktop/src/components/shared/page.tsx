import type { ComponentProps } from 'react'
import { cn } from '@/lib/utils'

/** Standard padded, width-limited container for a page's content. */
export function PageContainer({ className, ...props }: ComponentProps<'div'>) {
  return (
    <div
      className={cn('mx-auto flex w-full max-w-5xl flex-1 flex-col gap-6 p-4 sm:p-6', className)}
      {...props}
    />
  )
}
