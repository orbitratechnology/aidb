import { useEffect } from 'react'
import { toast } from 'sonner'
import { openExternalUrl } from '@/lib/commands'
import { isExternalHref } from '@/lib/external-links'

/**
 * Sends clicks on external links to the system browser. The native navigation guard blocks the
 * webview from loading them either way; this makes the click do what the user expects.
 */
export function useExternalLinks(): void {
  useEffect(() => {
    function onClick(event: MouseEvent) {
      if (event.defaultPrevented || event.button !== 0) return
      const target = event.target instanceof Element ? event.target : null
      const anchor = target?.closest('a[href]')
      if (!(anchor instanceof HTMLAnchorElement)) return
      if (!isExternalHref(anchor.href, window.location)) return

      event.preventDefault()
      openExternalUrl(anchor.href).catch((error: unknown) => {
        toast.error('Could not open the link', {
          description: error instanceof Error ? error.message : undefined,
        })
      })
    }

    document.addEventListener('click', onClick)
    return () => document.removeEventListener('click', onClick)
  }, [])
}
