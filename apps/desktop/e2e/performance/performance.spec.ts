import fs from 'node:fs'
import { expect, type Page } from '@playwright/test'
import { open, test } from '../support'

// The interface with a large database, production-built. Times are taken inside the page
// (`performance.now()`), from the action to the frame where the result is on screen, and
// written to test-results/performance.json. Budgets are several times the targets, so CI
// machines' variance does not fail the run; docs/performance.md records the measurements.

test.use({ viewport: { width: 1440, height: 900 } })

/** Each measurement with its unit: milliseconds, or a count of DOM elements. */
const results: Record<string, { value: number; unit: 'ms' | 'elements' }> = {}

function record(name: string, value: number, unit: 'ms' | 'elements' = 'ms') {
  results[name] = { value: Math.round(value), unit }
}

test.afterAll(() => {
  fs.mkdirSync('test-results', { recursive: true })
  fs.writeFileSync('test-results/performance.json', `${JSON.stringify(results, null, 2)}\n`)
  const rows = Object.entries(results).map(
    ([name, { value, unit }]) => `| ${name} | ${value} ${unit} |`,
  )
  const table = ['| Interface, production build | Measured |', '| --- | ---: |', ...rows].join('\n')
  console.log(table)
  if (process.env.GITHUB_STEP_SUMMARY) {
    fs.appendFileSync(process.env.GITHUB_STEP_SUMMARY, `\n${table}\n`)
  }
})

const DESTINATIONS = [
  { to: '/data', label: 'Data' },
  { to: '/sql', label: 'SQL' },
  { to: '/changes', label: 'Changes' },
  { to: '/migrations', label: 'Migrations' },
  { to: '/settings', label: 'Settings' },
  { to: '/ask', label: 'Ask' },
] as const

/** Clicks the sidebar link and waits for the screen: its title, its route, nothing loading. */
function navigate(page: Page, to: string, label: string): Promise<number> {
  return page.evaluate(
    async ({ to, label }) => {
      const link = document.querySelector<HTMLElement>(`[data-sidebar="sidebar"] a[href="#${to}"]`)
      if (!link) throw new Error(`no link to ${to}`)
      const started = performance.now()
      link.click()
      await new Promise<void>((resolve, reject) => {
        const check = () => {
          const title = document.querySelector('header h1')?.textContent
          const ready =
            location.hash.startsWith(`#${to}`) &&
            title === label &&
            !document.querySelector('[data-slot="skeleton"]')
          if (ready) resolve()
          else if (performance.now() - started > 10_000) reject(new Error(`${to} did not open`))
          else requestAnimationFrame(check)
        }
        requestAnimationFrame(check)
      })
      return performance.now() - started
    },
    { to, label },
  )
}

test('the interface is ready soon after the page loads', async ({ page }) => {
  await open(page, '/ask', { scenario: 'large' })
  // From the start of navigation to the first screen, with the core's answers in place.
  const ready = await page.evaluate(() => performance.now())
  record('interface ready after page load', ready)
  expect(ready).toBeLessThan(3_000)
})

test('screens open at once after their first visit', async ({ page }) => {
  await open(page, '/ask', { scenario: 'large' })
  for (const { to, label } of DESTINATIONS) {
    record(`first visit: ${label}`, await navigate(page, to, label))
  }
  for (const { to, label } of DESTINATIONS) {
    const ms = await navigate(page, to, label)
    record(`next visit: ${label}`, ms)
    // Target: under 100 ms (PRD); the budget allows for slow CI machines.
    expect(ms, label).toBeLessThan(500)
  }
})

test('a schema of 10,000 objects keeps the tree small and search fast', async ({ page }) => {
  await open(page, '/data', { scenario: 'large' })
  const tree = page.getByRole('navigation', { name: 'Tables and views' })
  await expect(tree.getByRole('link').first()).toBeVisible()
  const links = await tree.getByRole('link').count()
  record('tree links rendered for 10,000 objects', links, 'elements')
  // 200 per schema, five schemas.
  expect(links).toBeLessThanOrEqual(1_000)

  const search = page.getByRole('searchbox', { name: 'Search tables and views' })
  const ms = await search.evaluate(async (input: HTMLInputElement) => {
    const started = performance.now()
    const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set
    setValue?.call(input, 't09999')
    input.dispatchEvent(new Event('input', { bubbles: true }))
    await new Promise<void>((resolve) => {
      const check = () => {
        const nav = document.querySelector('nav[aria-label="Tables and views"]')
        if (nav && nav.querySelectorAll('a').length === 1) resolve()
        else requestAnimationFrame(check)
      }
      requestAnimationFrame(check)
    })
    return performance.now() - started
  })
  record('search 10,000 objects', ms)
  expect(ms).toBeLessThan(500)
})

test('a full page of rows renders only the rows in view', async ({ page }) => {
  await open(page, '/data/public/t00001', { scenario: 'large' })
  const grid = page.locator('table[aria-rowcount="501"]')
  await expect(grid).toBeVisible()
  const rendered = await grid.locator('tbody tr').count()
  record('grid rows in the DOM for 500 loaded', rendered, 'elements')
  expect(rendered).toBeLessThan(80)

  // Scrolling to the end shows the last row and keeps the DOM small.
  const ms = await page.evaluate(async () => {
    const table = document.querySelector('table[aria-rowcount="501"]')
    const scroller = table?.parentElement
    if (!scroller) throw new Error('no grid')
    const started = performance.now()
    scroller.scrollTop = scroller.scrollHeight
    await new Promise<void>((resolve) => {
      const check = () => {
        if (document.querySelector('tr[aria-rowindex="501"]')) resolve()
        else requestAnimationFrame(check)
      }
      requestAnimationFrame(check)
    })
    return performance.now() - started
  })
  record('scroll 500 rows to the end', ms)
  expect(ms).toBeLessThan(500)
  expect(await grid.locator('tbody tr').count()).toBeLessThan(80)
})
