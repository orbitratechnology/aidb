const EXTERNAL_PROTOCOLS = new Set(['http:', 'https:', 'mailto:'])

/**
 * Whether a link points outside the app and should open in the system browser.
 * `current` is the app's own location (`tauri://localhost` when bundled, the dev server in
 * development).
 */
export function isExternalHref(href: string, current: Pick<URL, 'protocol' | 'host'>): boolean {
  let url: URL
  try {
    url = new URL(href)
  } catch {
    return false
  }
  if (!EXTERNAL_PROTOCOLS.has(url.protocol)) {
    return false
  }
  return (
    url.protocol === 'mailto:' || url.protocol !== current.protocol || url.host !== current.host
  )
}
