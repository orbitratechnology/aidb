import AxeBuilder from '@axe-core/playwright'
import { test as base, expect, type Page } from '@playwright/test'

/**
 * The Playwright test, failing any test whose page threw an error or sent a command the fixture
 * core does not answer.
 */
export const test = base.extend({
  page: async ({ page }, provide) => {
    const errors: string[] = []
    page.on('pageerror', (error) => errors.push(error.message))
    await provide(page)
    expect(errors, 'uncaught errors in the page').toEqual([])
    expect(await unhandled(page), 'commands the fixture core does not answer').toEqual([])
  },
})

export { expect }

/** Widths from the smallest phone to a large desktop, with a height to match. */
export const WIDTHS = [
  { width: 320, height: 640 },
  { width: 390, height: 844 },
  { width: 768, height: 1024 },
  { width: 1024, height: 768 },
  { width: 1440, height: 900 },
  { width: 1920, height: 1080 },
] as const

/** Every top-level screen and settings section. */
export const ROUTES = [
  '/ask',
  '/data',
  '/data/public/orders',
  '/sql',
  '/changes',
  '/migrations',
  '/settings',
  '/settings/connections',
  '/settings/ai',
  '/settings/safety',
  '/settings/appearance',
  '/settings/storage',
  '/settings/diagnostics',
] as const

type Options = { scenario?: 'connected' | 'disconnected'; theme?: 'light' | 'dark' }

/** Opens a screen of the app running on the fixture core, and waits for it to settle. */
export async function open(page: Page, route: string, options: Options = {}): Promise<void> {
  const params = new URLSearchParams({
    scenario: options.scenario ?? 'connected',
    theme: options.theme ?? 'light',
  })
  // A fresh document every time: a goto that changes only the hash would keep the old app
  // (and the old fixture state) running.
  await page.goto('about:blank')
  await page.goto(`/e2e/app.html?${params}#${route}`)
  await expect(page.locator('#main-content')).toBeVisible()
  // Lazy screens and their queries: wait until nothing is loading.
  await page.waitForLoadState('networkidle')
  await expect(page.locator('[data-slot="skeleton"]')).toHaveCount(0)
}

/** Commands the app sent that the fixture core does not answer. */
export async function unhandled(page: Page): Promise<string[]> {
  return page.evaluate(() => (window as unknown as { __unhandled?: string[] }).__unhandled ?? [])
}

/** How far the page itself scrolls sideways (its own scroll areas, like a grid, are fine). */
export async function horizontalOverflow(page: Page): Promise<number> {
  return page.evaluate(
    () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
  )
}

/**
 * Visible controls smaller than `min` pixels in either dimension, described for the failure
 * message.
 */
export async function smallTargets(
  page: Page,
  min: { width: number; height: number },
): Promise<string[]> {
  return page.evaluate((min) => {
    const selector = [
      'button',
      'a[href]',
      'input:not([type="hidden"])',
      'select',
      'textarea',
      '[role="button"]',
      '[role="tab"]',
      '[role="checkbox"]',
      '[role="combobox"]',
      '[role="switch"]',
      '[role="radio"]',
      '[role="menuitem"]',
      '[role="option"]',
    ].join(',')
    const found: string[] = []
    // What a pointer can hit: the control, its <label>s, or the input group around an input.
    const hitBox = (element: HTMLElement): DOMRect => {
      const parts = [element.getBoundingClientRect()]
      const group = element.closest<HTMLElement>('[data-slot="input-group"]')
      if (group) parts.push(group.getBoundingClientRect())
      const labels = (element as HTMLInputElement).labels
      if (labels) for (const label of labels) parts.push(label.getBoundingClientRect())
      const visible = parts.filter((part) => part.width > 0 && part.height > 0)
      const left = Math.min(...visible.map((part) => part.left))
      const top = Math.min(...visible.map((part) => part.top))
      const right = Math.max(...visible.map((part) => part.right))
      const bottom = Math.max(...visible.map((part) => part.bottom))
      return new DOMRect(left, top, right - left, bottom - top)
    }
    for (const element of document.querySelectorAll<HTMLElement>(selector)) {
      const own = element.getBoundingClientRect()
      const style = getComputedStyle(element)
      const inert =
        // Zero or one pixel: visually hidden helpers such as Radix's native <select>.
        own.width <= 1 ||
        own.height <= 1 ||
        style.visibility === 'hidden' ||
        style.pointerEvents === 'none' ||
        Number(style.opacity) === 0 ||
        element.closest('.sr-only') !== null
      if (inert) continue
      // Inline links in a sentence are exempt, as WCAG 2.5.8 allows.
      if (element.tagName === 'A' && element.closest('p, li') && style.display === 'inline')
        continue
      // Controls inside the grid and the editor are sized to their rows; both have keyboard
      // equivalents for everything.
      if (element.closest('[role="grid"], .cm-editor')) continue
      // The sidebar's edge rail duplicates the Toggle Sidebar button (WCAG 2.5.8 "equivalent").
      if (element.dataset.sidebar === 'rail') continue
      const box = hitBox(element)
      if (box.width < min.width || box.height < min.height) {
        const name =
          element.getAttribute('aria-label') ??
          element.getAttribute('title') ??
          element.textContent?.trim().slice(0, 40) ??
          element.tagName
        const role = element.getAttribute('role') ?? element.tagName.toLowerCase()
        found.push(`${role} "${name}" ${Math.round(box.width)}×${Math.round(box.height)}`)
      }
    }
    return found
  }, min)
}

/** Accessibility problems axe finds against WCAG 2.2 A and AA. */
export async function axeViolations(page: Page): Promise<string[]> {
  const results = await new AxeBuilder({ page })
    .withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa'])
    .analyze()
  return results.violations.map(
    (violation) =>
      `${violation.id} (${violation.impact}): ${violation.help} — ${violation.nodes
        .slice(0, 3)
        .map((node) => node.target.join(' '))
        .join(' | ')}`,
  )
}
