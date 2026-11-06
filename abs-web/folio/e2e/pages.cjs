/**
 * Proof for the web fork's pages (PARITY W0001–W0042): each page is opened in Chromium against a running Folio that
 * holds the original's recorded library (see seed.py), signed in as root. A page passes when it shows without a
 * script error and none of its calls to Folio fails (a 5xx answer, or a 404 from the API). Each page is
 * screenshotted to SHOTS_DIR/<row>.png when SHOTS_DIR is set.
 *
 * Run from anywhere: BASE_URL=http://localhost:13380 node abs-web/folio/e2e/pages.cjs [row ...]
 * (needs Playwright with Chromium). With rows given, only those pages are opened.
 */
const { chromium } = require('playwright')

const BASE = (process.env.BASE_URL || 'http://localhost:13380').replace(/\/$/, '')
const SHOTS = process.env.SHOTS_DIR || ''
const ONLY = process.argv.slice(2)

/** Answers a signed-in API call's JSON. */
async function api(token, path) {
  const r = await fetch(`${BASE}${path}`, { headers: { Authorization: `Bearer ${token}` } })
  if (!r.ok) throw new Error(`${path} answered ${r.status}`)
  return r.json()
}

;(async () => {
  const login = await fetch(`${BASE}/login`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', 'x-return-tokens': 'true' },
    body: JSON.stringify({ username: 'root', password: 'root' })
  })
  const token = (await login.json()).user.accessToken
  const libs = (await api(token, '/api/libraries')).libraries
  const lib = libs[0].id
  const items = (await api(token, `/api/libraries/${lib}/items?limit=10`)).results
  const item = items.find((i) => i.media.numAudioFiles || i.media.audioFiles?.length || i.media.numTracks) || items[0]
  const series = (await api(token, `/api/libraries/${lib}/series?limit=10`)).results[0]
  const author = (await api(token, `/api/libraries/${lib}/authors`)).authors[0]
  const user = (await api(token, '/api/users')).users[0]
  const collections = (await api(token, '/api/collections').catch(() => ({ collections: [] }))).collections || []
  const playlists = (await api(token, '/api/playlists').catch(() => ({ playlists: [] }))).playlists || []

  /** Each row's page, as the app's own links name it. */
  const pages = [
    ['W0001', '/account'],
    ['W0002', '/config'],
    ['W0003', '/'],
    ['W0005', '/oops'],
    ['W0006', playlists[0] && `/playlist/${playlists[0].id}`],
    ['W0007', collections[0] && `/collection/${collections[0].id}`],
    ['W0008', '/config/authentication'],
    ['W0009', '/config/backups'],
    ['W0010', '/config/email'],
    ['W0011', '/config'],
    ['W0012', '/config/libraries'],
    ['W0013', '/config/log'],
    ['W0014', '/config/notifications'],
    ['W0015', '/config/rss-feeds'],
    ['W0016', '/config/sessions'],
    ['W0017', '/config/stats'],
    ['W0018', '/config/users'],
    ['W0019', `/config/users/${user.id}`],
    ['W0020', `/config/users/${user.id}/sessions`],
    ['W0021', '/config/api-keys'],
    ['W0022', '/config/item-metadata-utils/custom-metadata-providers'],
    ['W0023', '/config/item-metadata-utils/genres'],
    ['W0024', '/config/item-metadata-utils'],
    ['W0025', '/config/item-metadata-utils/tags'],
    ['W0026', `/library/${lib}`],
    ['W0027', `/library/${lib}/narrators`],
    ['W0028', `/library/${lib}/search?q=a`],
    ['W0029', `/library/${lib}/stats`],
    ['W0030', series && `/library/${lib}/series/${series.id}`],
    ['W0034', `/library/${lib}/bookshelf/series`],
    ['W0036', '/upload'],
    ['W0037', `/audiobook/${item.id}/chapters`],
    ['W0038', `/audiobook/${item.id}/edit`],
    ['W0039', `/audiobook/${item.id}/manage`],
    ['W0040', `/item/${item.id}`],
    ['W0041', author && `/author/${author.id}`],
    ['W0042', '/batch']
  ].filter(([row]) => !ONLY.length || ONLY.includes(row))

  const browser = await chromium.launch()
  const ctx = await browser.newContext({ viewport: { width: 1280, height: 800 }, deviceScaleFactor: SHOTS ? 2 : 1 })
  const p = await ctx.newPage()
  let problems = []
  p.on('pageerror', (e) => problems.push(`script error: ${e.message}`))
  p.on('response', (r) => {
    const path = new URL(r.url()).pathname
    const failed = r.status() >= 500 || (r.status() === 404 && path.startsWith('/api/'))
    if (failed && /^\/(api|hls|public|auth|login|status|init|feed)/.test(path)) problems.push(`${r.request().method()} ${path} answered ${r.status()}`)
  })

  // W0004: the sign-in page, used as a person would.
  await p.goto(`${BASE}/login`, { waitUntil: 'networkidle' })
  if (!ONLY.length || ONLY.includes('W0004')) {
    if (SHOTS) await p.screenshot({ path: `${SHOTS}/W0004.png` })
  }
  await p.locator('input[autocomplete="username"], input[name="username"]').first().fill('root')
  await p.locator('input[type="password"]').first().fill('root')
  await p.locator('button[type="submit"]').first().click()
  await p.waitForURL((u) => !u.pathname.startsWith('/login'), { timeout: 15000 })
  const login4 = problems
  problems = []
  const results = [['W0004', login4]]

  for (const [row, path] of pages) {
    if (!path) {
      results.push([row, ['no data for this page in the recorded library']])
      continue
    }
    await p.goto(`${BASE}${path}`, { waitUntil: 'networkidle' }).catch((e) => problems.push(`didn't load: ${e.message}`))
    await p.waitForTimeout(500)
    if (SHOTS) await p.screenshot({ path: `${SHOTS}/${row}.png` })
    results.push([row, problems])
    problems = []
  }
  // The ebook reader: the book with an EPUB opens in the reader and shows its text.
  const ebook = items.find((i) => i.media.ebookFormat || i.media.ebookFile)
  if (ebook && (!ONLY.length || ONLY.includes('reader'))) {
    await p.goto(`${BASE}/item/${ebook.id}`, { waitUntil: 'networkidle' })
    await p.getByRole('button', { name: 'Read' }).first().click()
    const page = p.frameLocator('#viewer iframe').locator('body')
    await page.waitFor({ timeout: 15000 }).catch(() => problems.push("the reader didn't show the book's text"))
    await p.waitForTimeout(1000)
    if (SHOTS) await p.screenshot({ path: `${SHOTS}/reader.png` })
    results.push(['reader', problems])
    problems = []
  }

  // The player: a book in MP3 parts starts playing and its time moves on.
  const mp3 = items.find((i) => (i.media.audioFiles || []).some?.((a) => a.mimeType === 'audio/mpeg')) || items.find((i) => i.media.numAudioFiles > 1)
  if (mp3 && (!ONLY.length || ONLY.includes('player'))) {
    await p.goto(`${BASE}/item/${mp3.id}`, { waitUntil: 'networkidle' })
    await p.getByRole('button', { name: 'Play' }).first().click()
    const moved = await p
      .waitForFunction(() => [...document.querySelectorAll('audio')].some((a) => a.currentTime > 1 && !a.paused), null, { timeout: 20000 })
      .then(() => true)
      .catch(() => false)
    if (!moved) problems.push("the player didn't start playing")
    if (SHOTS) await p.screenshot({ path: `${SHOTS}/player.png` })
    results.push(['player', problems])
    problems = []
  }
  await browser.close()
  let failed = 0
  for (const [row, list] of results) {
    console.log(`${list.length ? 'FAIL' : 'ok  '} ${row}${list.length ? '\n       ' + [...new Set(list)].join('\n       ') : ''}`)
    if (list.length) failed++
  }
  process.exit(failed ? 1 : 0)
})().catch((e) => {
  console.error(e)
  process.exit(1)
})
