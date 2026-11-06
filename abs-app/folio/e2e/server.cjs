/**
 * Proof that the mobile fork works against a real Folio server: the app signs in (PARITY M0003), opens each page
 * that reads the library and shows the original's recorded books on it (home M0002/M0006/M0017, library M0019,
 * series M0022, one series M0021, authors M0015, a book M0023, account M0001, stats M0010, logs M0007), and
 * streams a book (the play button, Folio's session and its track stream). Every page must load without a failed
 * request to Folio and without an error in the page.
 *
 * It runs against the app's web build (served at BASE_URL) and a Folio server at FOLIO_URL started by
 * folio/e2e/seed_folio.sh. The web build's requests leave the browser, so Chromium runs without web security: on
 * the phone they are native requests, to which a browser's cross-site rules do not apply.
 *
 * Run from abs-app/: BASE_URL=http://localhost:8765 FOLIO_URL=http://localhost:8091 node folio/e2e/server.cjs
 */
const assert = require('node:assert/strict')
const { chromium } = require('playwright')

const BASE = (process.env.BASE_URL || 'http://localhost:8765').replace(/\/$/, '')
const FOLIO = (process.env.FOLIO_URL || 'http://localhost:8091').replace(/\/$/, '')
const SHOTS = process.env.SHOTS_DIR || ''

/** The recorded library's book with three MP3 parts, its library item, and the series and book it is not in. */
const PSALM_ITEM = '2ae3ffc2-cdbb-4dbf-9879-92b78bd9172d'
/** The recorded series "The Murderbot Diaries". */
const MURDERBOT_SERIES = 'ea9f35d3-5cea-4418-81f8-26978c289f71'

/** Each page the test opens, the PARITY rows it proves and the words it must show from Folio's data. */
const PAGES = [
  { path: '/bookshelf', rows: 'M0002, M0006, M0017', shows: ['Continue Listening', 'Recently Added', 'A Psalm for the Wild-Built'] },
  { path: '/bookshelf/library', rows: 'M0019', shows: ['A Psalm for the Wild-Built', 'Ancillary Justice', 'All Systems Red'] },
  { path: '/bookshelf/series', rows: 'M0022', shows: ['2 Series', 'Imperial Radch', 'The Murderbot Diaries'] },
  { path: `/bookshelf/series/${MURDERBOT_SERIES}`, rows: 'M0021', shows: ['The Murderbot Diaries', 'All Systems Red'] },
  { path: '/bookshelf/authors', rows: 'M0015', shows: ['3 Authors', 'Ann Leckie', 'Becky Chambers', 'Martha Wells'] },
  { path: `/item/${PSALM_ITEM}`, rows: 'M0023', shows: ['A Psalm for the Wild-Built', 'Becky Chambers', 'Audio Tracks'] },
  { path: '/account', rows: 'M0001', shows: ['root'] },
  { path: '/stats', rows: 'M0010', shows: ['Recent Sessions'] },
  { path: '/logs', rows: 'M0007', shows: ['Logs'] }
]

/** Watches a page for failed requests to Folio and for errors in the page; [take] hands over what was seen. */
function watch(p) {
  const seen = []
  p.on('response', (r) => {
    if (r.url().startsWith(FOLIO) && r.status() >= 400) seen.push(`${r.status()} ${r.request().method()} ${new URL(r.url()).pathname}`)
  })
  p.on('pageerror', (e) => seen.push(`page error: ${e.message}`))
  return { take: () => seen.splice(0) }
}

/** Signs in to Folio through the app's connect page, as a person would. */
async function signIn(p) {
  await p.goto(`${BASE}/connect/`, { waitUntil: 'networkidle' })
  const add = p.getByText('Add New Server', { exact: false })
  if (await add.count()) await add.first().click()
  await p.locator('input[type=url]').fill(FOLIO)
  await p.getByRole('button', { name: /submit/i }).click()
  await p.locator('input[type=password]').waitFor()
  await p.locator('input').nth(0).fill('root')
  await p.locator('input[type=password]').fill('root')
  await p.getByRole('button', { name: /submit/i }).click()
  await p.waitForURL('**/bookshelf**')
}

/** Opens [path] inside the running app, as its own links do. */
async function go(p, path) {
  await p.evaluate((to) => window.$nuxt.$router.push(to), path)
  await p.waitForLoadState('networkidle')
  await p.waitForTimeout(1500)
}

;(async () => {
  const browser = await chromium.launch({ args: ['--disable-web-security', '--autoplay-policy=no-user-gesture-required'] })
  const p = await browser.newPage({ viewport: { width: 390, height: 844 }, deviceScaleFactor: SHOTS ? 2 : 1 })
  const seen = watch(p)
  try {
    await signIn(p)
    assert.deepEqual(seen.take(), [], 'signing in (M0003)')

    for (const page of PAGES) {
      await go(p, page.path)
      const text = await p.locator('body').innerText()
      for (const words of page.shows) assert.ok(text.includes(words), `${page.path} (${page.rows}) shows "${words}"`)
      assert.deepEqual(seen.take(), [], `${page.path} (${page.rows}) loads without errors`)
      console.log(`ok ${page.path} (${page.rows})`)
    }

    // Streaming: the book's play button opens a session on Folio and the audio plays from its track stream.
    await go(p, `/item/${PSALM_ITEM}`)
    await p.getByRole('button', { name: /stream|play/i }).first().click()
    await p.waitForFunction(() => {
      const a = document.querySelector('audio')
      return a && !a.paused && a.currentTime > 2
    }, null, { timeout: 20000 })
    const src = await p.evaluate(() => document.querySelector('audio').src)
    assert.match(src, new RegExp(`^${FOLIO}/public/session/[^/]+/track/\\d+$`), 'the audio comes from the session track stream')
    assert.deepEqual(seen.take(), [], 'streaming loads without errors')
    if (SHOTS) {
      await p.locator('.cover-wrapper').click({ force: true })
      await p.waitForTimeout(1000)
      await p.screenshot({ path: `${SHOTS}/player.png` })
    }
    console.log('ok streaming')
  } finally {
    await browser.close()
  }
})().catch((e) => {
  console.error(e)
  process.exit(1)
})
