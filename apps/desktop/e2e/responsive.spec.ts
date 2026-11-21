import { expect, horizontalOverflow, open, ROUTES, smallTargets, test, WIDTHS } from './support'

/** Below this width the app is in its phone layout (Tailwind's `sm`). */
const PHONE = 640

for (const size of WIDTHS) {
  test.describe(`${size.width}×${size.height}`, () => {
    test.use({ viewport: size })

    for (const route of ROUTES) {
      test(`${route} fits the window`, async ({ page }) => {
        await open(page, route)
        expect(await horizontalOverflow(page), 'the page scrolls sideways').toBe(0)
        if (size.width < PHONE) {
          // Phones: every control is at least 44px tall (WCAG 2.5.5, platform guidance).
          expect(await smallTargets(page, { width: 0, height: 44 })).toEqual([])
        }
      })
    }

    test('one primary navigation, placed for the width', async ({ page }) => {
      await open(page, '/data')
      const nav = page.locator('nav[aria-label="Primary"]:visible')
      await expect(nav).toHaveCount(1)
      const link = nav.getByRole('link', { name: 'SQL' })
      const box = await link.boundingBox()
      expect(box).not.toBeNull()
      if (size.width < PHONE) {
        // A bottom bar on phones.
        expect(box!.y).toBeGreaterThan(size.height - 80)
      } else {
        // A sidebar on wider windows.
        expect(box!.x).toBeLessThan(300)
      }
      await link.click()
      await expect(page).toHaveURL(/#\/sql$/)
      await expect(link).toHaveAttribute('aria-current', 'page')
    })
  })
}

test.describe('no connection', () => {
  for (const size of [WIDTHS[0], WIDTHS[4]]) {
    test(`${size.width}px: screens explain what to do`, async ({ page }) => {
      await page.setViewportSize(size)
      for (const route of ['/ask', '/data', '/sql', '/migrations']) {
        await open(page, route, { scenario: 'disconnected' })
        expect(await horizontalOverflow(page), `${route} scrolls sideways`).toBe(0)
        await expect(page.getByRole('link', { name: 'No connection' })).toBeVisible()
      }
    })
  }
})
