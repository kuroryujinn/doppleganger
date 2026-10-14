import { readdir, readFile, access } from 'node:fs/promises'
import { resolve, join } from 'node:path'
import assert from 'node:assert/strict'
import { groups } from '../.vitepress/navigation.mjs'

const dist = resolve('.vitepress/dist')
async function walk(directory) {
  const files = []
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name)
    if (entry.isDirectory()) files.push(...await walk(path))
    else if (path.endsWith('.html')) files.push(path)
  }
  return files
}
function target(pathname) {
  return join(dist, pathname.endsWith('/') ? pathname + 'index.html' : pathname + (pathname.endsWith('.html') ? '' : '.html'))
}
const routes = groups.flatMap(group => group.items.map(item => item.link))
assert.equal(new Set(routes).size, routes.length, 'Duplicate navigation routes')
for (const route of routes) {
  const html = await readFile(target(route), 'utf8')
  assert.match(html, /<h1[^>]*>.+?<\/h1>/, `${route}: missing page heading`)
  assert.match(html, /aria-label="Breadcrumb"/, `${route}: missing breadcrumbs`)
  assert.match(html, /<meta name="description"/, `${route}: missing metadata`)
}
let links = 0
for (const file of await walk(dist)) {
  const html = await readFile(file, 'utf8')
  for (const [, href] of html.matchAll(/href="([^"\s]+)"/g)) {
    if (!href.startsWith('/') || href.startsWith('//') || href.startsWith('/assets/')) continue
    const url = new URL(href, 'http://docs.local')
    const pathname = decodeURIComponent(url.pathname)
    const path = /\.[a-z0-9]+$/i.test(pathname) && !pathname.endsWith('.html')
      ? join(dist, pathname) : target(pathname)
    await access(path)
    if (url.hash) {
      const destination = await readFile(path, 'utf8')
      const id = decodeURIComponent(url.hash.slice(1))
      assert.ok(destination.includes(`id="${id}"`), `${file}: missing anchor ${href}`)
    }
    links++
  }
}
console.log(`Checked ${routes.length} documentation routes and ${links} internal links/anchors.`)
