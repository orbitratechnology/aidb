import type { Page } from '@playwright/test'
import { expect, open, test } from './support'

test.use({ viewport: { width: 1440, height: 900 } })

const MOD = process.platform === 'darwin' ? 'Meta' : 'Control'

/** Whether the focused element shows a focus indicator (an outline or a ring). */
function focusShows(page: Page) {
  return page.evaluate(() => {
    const focused = document.activeElement as HTMLElement | null
    if (!focused || focused === document.body) return null
    // The editor draws its indicator around the whole editor, not on its content element.
    const element = focused.closest<HTMLElement>('.cm-editor') ?? focused
    const style = getComputedStyle(element)
    const outline = style.outlineStyle !== 'none' && parseFloat(style.outlineWidth) > 0
    const ring = style.boxShadow !== 'none'
    const name =
      focused.getAttribute('aria-label') ??
      focused.textContent?.trim().slice(0, 40) ??
      focused.tagName
    return { name, visible: outline || ring }
  })
}

test('the skip link is first and moves focus to the content', async ({ page }) => {
  await open(page, '/data')
  await page.keyboard.press('Tab')
  const skip = page.getByRole('button', { name: 'Skip to content' })
  await expect(skip).toBeFocused()
  await expect(skip).toBeVisible()
  await page.keyboard.press('Enter')
  await expect(page.locator('#main-content')).toBeFocused()
})

for (const route of ['/data', '/settings/connections', '/settings/safety', '/sql']) {
  test(`${route}: every control reached with Tab shows focus`, async ({ page }) => {
    await open(page, route)
    const seen: string[] = []
    for (let step = 0; step < 25; step += 1) {
      await page.keyboard.press('Tab')
      const focus = await focusShows(page)
      // Past the last control, focus leaves the page for the browser's own interface.
      if (!focus) break
      // Some widgets (the editor) draw their indicator a frame after taking focus.
      await expect
        .poll(async () => (await focusShows(page))?.visible, {
          message: `no visible focus on "${focus.name}"`,
        })
        .toBe(true)
      seen.push(focus.name)
    }
    expect(seen.length).toBeGreaterThan(3)
  })
}

test('a sheet traps focus, closes with Escape and returns focus', async ({ page }) => {
  await open(page, '/settings/connections')
  const trigger = page.getByRole('button', { name: 'Add connection' })
  await trigger.focus()
  await page.keyboard.press('Enter')
  const dialog = page.getByRole('dialog', { name: 'Add a connection' })
  await expect(dialog).toBeVisible()
  for (let step = 0; step < 30; step += 1) {
    await page.keyboard.press('Tab')
    expect(await dialog.evaluate((node) => node.contains(document.activeElement))).toBe(true)
  }
  await page.keyboard.press('Escape')
  await expect(dialog).toBeHidden()
  await expect(trigger).toBeFocused()
})

test('SQL: a query runs from the keyboard', async ({ page }) => {
  await open(page, '/sql')
  await page.locator('.cm-content').focus()
  await page.keyboard.type('select count(*) from orders')
  await page.keyboard.press(`${MOD}+Enter`)
  const result = page.getByRole('region', { name: 'Result' })
  await expect(result.getByRole('cell', { name: '200000' })).toBeVisible()
})

test('SQL: draft tabs follow the tabs pattern', async ({ page }) => {
  await open(page, '/sql')
  await page.getByRole('button', { name: 'New tab' }).click()
  await page.getByRole('button', { name: 'New tab' }).click()
  const tabs = page.getByRole('tablist', { name: 'SQL drafts' }).getByRole('tab')
  await expect(tabs).toHaveCount(3)
  await expect(tabs.nth(2)).toHaveAttribute('aria-selected', 'true')

  await tabs.nth(2).focus()
  await page.keyboard.press('ArrowLeft')
  await expect(tabs.nth(1)).toBeFocused()
  await expect(tabs.nth(1)).toHaveAttribute('aria-selected', 'true')
  await page.keyboard.press('Home')
  await expect(tabs.nth(0)).toBeFocused()
  await page.keyboard.press('End')
  await expect(tabs.nth(2)).toBeFocused()
  await page.keyboard.press('ArrowRight')
  await expect(tabs.nth(0)).toBeFocused()

  await page.keyboard.press('Delete')
  await expect(tabs).toHaveCount(2)
  await expect(page.getByRole('tab', { selected: true })).toBeFocused()
  await expect(page.getByRole('tabpanel')).toBeVisible()
})

test('a dialog opened by a command returns focus to the control that opened it', async ({
  page,
}) => {
  await open(page, '/sql')
  await page.locator('.cm-content').focus()
  await page.keyboard.type('select 1')
  const save = page.getByRole('button', { name: 'Save' })
  await save.focus()
  await page.keyboard.press('Enter')
  const dialog = page.getByRole('dialog', { name: 'Save query' })
  await expect(dialog).toBeVisible()
  await expect(dialog.getByRole('textbox', { name: 'Name' })).toBeFocused()
  await page.keyboard.press('Escape')
  await expect(dialog).toBeHidden()
  await expect(save).toBeFocused()
})
