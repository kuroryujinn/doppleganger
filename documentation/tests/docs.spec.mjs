import { test, expect } from '@playwright/test'
import { groups } from '../.vitepress/navigation.mjs'

test('every documentation route renders, has working navigation and heading anchors', async ({ page }) => {
  const errors = []
  page.on('pageerror', error => errors.push(error.message))
  for (const group of groups) {
    for (const item of group.items) {
      const response = await page.goto(item.link)
      expect(response.status()).toBe(200)
      await expect(page.locator('.vp-doc h1')).toBeVisible()
      await expect(page.getByRole('navigation', { name: 'Breadcrumb' })).toContainText(group.text)
      const headings = await page.locator('.vp-doc h2[id]').evaluateAll(elements => elements.map(e => e.id))
      expect(headings.length).toBeGreaterThan(0)
      for (const id of headings) await expect(page.locator(`.VPDocAside a[href="#${id}"]`)).toHaveCount(1)
      await expect(page.locator('.VPSidebarItem.is-active')).not.toHaveCount(0)
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true)
    }
  }
  expect(errors).toEqual([])
})

test('copy controls, theme persistence, TOC, previous and next links', async ({ page, context }) => {
  await context.grantPermissions(['clipboard-read', 'clipboard-write'])
  await page.goto('/docs/quickstart')
  const core = page.locator('.VPSidebarItem.level-0').filter({ has: page.locator('h2', { hasText: 'Core concepts' }) })
  await expect(core).toHaveClass(/collapsed/)
  await core.locator('.item[role="button"]').first().focus()
  await page.keyboard.press('Enter')
  await expect(core).not.toHaveClass(/collapsed/)
  await page.keyboard.press('Enter')
  await expect(core).toHaveClass(/collapsed/)
  await page.locator('.vp-doc h1').click()
  await page.screenshot({ path: 'verification/desktop-light.png' })
  const block = page.locator('.vp-doc .language-bash').first()
  await block.hover()
  await block.locator('button.copy').click()
  await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toContain('cargo +1.98.0 install')
  await page.getByRole('switch').click()
  await expect(page.locator('html')).toHaveClass(/dark/)
  await page.reload()
  await expect(page.locator('html')).toHaveClass(/dark/)
  await page.screenshot({ path: 'verification/desktop-dark.png' })
  await page.locator('.VPDocAside a[href="#_4-run-the-complete-test"]').click()
  await expect(page).toHaveURL(/#_4-run-the-complete-test$/)
  await expect(page.locator('.VPDocAside a.active')).toHaveAttribute('href', '#_4-run-the-complete-test')
  await page.locator('.VPDocFooter a.next').click()
  await expect(page).toHaveURL(/\/docs\/first-test$/)
  await page.locator('.VPDocFooter a.prev').click()
  await expect(page).toHaveURL(/\/docs\/quickstart$/)
})

test('keyboard search navigates to a real result', async ({ page }) => {
  await page.goto('/docs/quickstart')
  await page.keyboard.press('Control+k')
  const search = page.locator('#localsearch-input')
  await expect(search).toBeVisible()
  await search.fill('balance-field')
  await expect(page.locator('.VPLocalSearchBox .result').first()).toBeVisible()
  await page.keyboard.press('ArrowDown')
  await page.keyboard.press('Enter')
  await expect(search).not.toBeVisible()
  await expect(page.locator('.vp-doc')).toContainText('balance-field')
})

test('mobile menu and code blocks fit at 375px', async ({ page }) => {
  await page.setViewportSize({ width: 375, height: 812 })
  await page.goto('/docs/first-test')
  await page.getByRole('button', { name: 'Menu', exact: true }).click()
  await expect(page.locator('.VPSidebar')).toBeVisible()
  await page.locator('.VPSidebar a[href="/docs/quickstart"]').click()
  await expect(page).toHaveURL(/\/docs\/quickstart$/)
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true)
  for (const group of groups) {
    for (const item of group.items) {
      await page.goto(item.link)
      await expect(page.locator('.vp-doc h1')).toBeVisible()
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true)
    }
  }
  await page.goto('/docs/quickstart')
  await expect(page.locator('.vp-doc h1')).toHaveText('Quickstart')
  await page.screenshot({ path: 'verification/mobile.png' })
})
