import { axeViolations, expect, open, ROUTES, smallTargets, test } from './support'

const SIZES = [
  { width: 390, height: 844 },
  { width: 1440, height: 900 },
] as const

for (const theme of ['light', 'dark'] as const) {
  for (const size of SIZES) {
    test.describe(`${theme}, ${size.width}px`, () => {
      test.use({ viewport: size })

      for (const route of ROUTES) {
        test(`${route} meets WCAG 2.2 AA`, async ({ page }) => {
          await open(page, route, { theme })
          await expect(page.getByRole('main')).toHaveCount(1)
          expect(await axeViolations(page)).toEqual([])
        })
      }
    })
  }
}

test.describe('dialogs and sheets', () => {
  for (const size of SIZES) {
    test.describe(`${size.width}px`, () => {
      test.use({ viewport: size })

      test('connection form', async ({ page }) => {
        await open(page, '/settings/connections')
        await page.getByRole('button', { name: 'Add connection' }).click()
        await expect(page.getByRole('dialog', { name: 'Add a connection' })).toBeVisible()
        expect(await axeViolations(page)).toEqual([])
        if (size.width < 640) expect(await smallTargets(page, { width: 0, height: 44 })).toEqual([])
      })

      test('query plan', async ({ page }) => {
        await open(page, '/sql')
        await page.locator('.cm-content').click()
        await page.keyboard.type('select count(*) from orders')
        await page.getByRole('button', { name: 'Explain' }).click()
        await expect(page.getByRole('dialog')).toBeVisible()
        await expect(
          page.getByRole('dialog').getByText('orders', { exact: false }).first(),
        ).toBeVisible()
        expect(await axeViolations(page)).toEqual([])
        if (size.width < 640) expect(await smallTargets(page, { width: 0, height: 44 })).toEqual([])
      })
    })
  }
})

test('no connection: screens meet WCAG 2.2 AA', async ({ page }) => {
  for (const route of ['/ask', '/data', '/sql', '/changes', '/migrations']) {
    await open(page, route, { scenario: 'disconnected' })
    expect(await axeViolations(page), route).toEqual([])
  }
})
