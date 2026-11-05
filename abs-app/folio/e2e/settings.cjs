/**
 * Proof for the mobile fork's settings page (PARITY M0009) and its device settings (M0028–M0051): each setting is
 * changed through the page the way a person would, and must be saved to the device's settings and still be there
 * after the app is opened again (a reload). It runs against the app's web build (`npx nuxt generate`, served at
 * BASE_URL), where the device's storage is the browser's; on the phone the same page calls the native plugin.
 *
 * Run from abs-app/: BASE_URL=http://localhost:8765 node folio/e2e/settings.cjs (needs Playwright with Chromium).
 */
const assert = require('node:assert/strict')
const { chromium } = require('playwright')

const BASE = (process.env.BASE_URL || 'http://localhost:8765').replace(/\/$/, '')
const SHOTS = process.env.SHOTS_DIR || ''

/** The device settings as the app saved them. */
async function stored(p) {
  return p.evaluate(() => JSON.parse(localStorage.getItem('device') || '{}').deviceSettings || {})
}

/** Opens the settings page afresh, as when the app starts again. */
async function open(p) {
  await p.goto(`${BASE}/settings/`, { waitUntil: 'networkidle' })
  await p.getByText('Use bookshelf view', { exact: true }).waitFor()
}

/** The row of the setting labelled [label]. */
function row(p, label) {
  return p.getByText(label, { exact: true }).locator('xpath=..')
}

/** Flips the switch of [label]. */
async function toggle(p, label) {
  await row(p, label).locator('xpath=./div[1]').click()
  await p.waitForTimeout(300)
}

/** Opens the choices of [label] and picks [option]. */
async function pick(p, label, option) {
  await row(p, label).locator('input').click()
  await p.getByText(option, { exact: true }).last().click()
  await p.waitForTimeout(300)
}

/** Types [value] into the field of [label]. */
async function type(p, label, value) {
  const input = row(p, label).locator('input')
  await input.fill(value)
  await input.dispatchEvent('input')
  await p.waitForTimeout(300)
}

/** Checks that [key] reads [expected] (or passes [expected] when it is a function) after the app opens again. */
async function expectSaved(p, key, expected) {
  await open(p)
  const v = (await stored(p))[key]
  if (typeof expected === 'function') assert.ok(expected(v), `${key} saved as ${JSON.stringify(v)}`)
  else assert.deepEqual(v, expected, `${key}`)
  console.log(`ok  ${key} = ${JSON.stringify(v)}`)
  return v
}

/** A switch is saved both ways: flipped once, then back. */
async function expectToggle(p, label, key) {
  await toggle(p, label)
  const first = await expectSaved(p, key, (v) => v !== undefined)
  await toggle(p, label)
  await expectSaved(p, key, (v) => v !== undefined && v !== first)
}

;(async () => {
  const browser = await chromium.launch()
  const p = await browser.newPage({ viewport: { width: 390, height: 844 }, deviceScaleFactor: SHOTS ? 2 : 1 })
  const errors = []
  p.on('pageerror', (e) => errors.push(e.message))
  await open(p)
  if (SHOTS) await p.screenshot({ path: `${SHOTS}/M0009.png` })

  // user interface
  await expectToggle(p, 'Use bookshelf view', 'enableAltView')
  await expectToggle(p, 'Lock orientation', 'lockOrientation')
  await pick(p, 'Haptic feedback', 'Heavy')
  await expectSaved(p, 'hapticFeedback', 'HEAVY')

  // playback
  await pick(p, 'Jump backwards time', '30s')
  await expectSaved(p, 'jumpBackwardsTime', 30)
  await pick(p, 'Jump forwards time', '60s')
  await expectSaved(p, 'jumpForwardTime', 60)
  await expectToggle(p, 'Disable auto rewind', 'disableAutoRewind')
  await expectToggle(p, 'Enable mp3 index seeking', 'enableMp3IndexSeeking')
  await expectToggle(p, 'Allow position seeking on media notification controls', 'allowSeekingOnMediaControls')

  // sleep timer
  await pick(p, 'Shake sensitivity', 'Very High')
  await expectSaved(p, 'shakeSensitivity', 'VERY_HIGH')
  await expectToggle(p, 'Disable shake to reset', 'disableShakeToResetSleepTimer')
  await expectToggle(p, 'Disable audio fade out', 'disableSleepTimerFadeOut')
  await expectToggle(p, 'Disable vibrate on reset', 'disableSleepTimerResetFeedback')
  await expectToggle(p, 'Play a chime when almost finished', 'enableSleepTimerAlmostDoneChime')
  await toggle(p, 'Auto sleep timer')
  await expectSaved(p, 'autoSleepTimer', true)
  await type(p, 'Start time', '21:30')
  await expectSaved(p, 'autoSleepTimerStartTime', '21:30')
  await type(p, 'End time', '07:15')
  await expectSaved(p, 'autoSleepTimerEndTime', '07:15')
  await pick(p, 'Sleep timer', '45 min')
  await expectSaved(p, 'sleepTimerLength', 45 * 60 * 1000)
  await toggle(p, 'Auto sleep timer auto rewind')
  await expectSaved(p, 'autoSleepTimerAutoRewind', true)
  await pick(p, 'Auto rewind time', '10 min')
  await expectSaved(p, 'autoSleepTimerAutoRewindTime', 10 * 60 * 1000)
  await toggle(p, 'Auto sleep timer')
  await expectSaved(p, 'autoSleepTimer', false)

  // data and Android Auto
  await pick(p, 'Download using Cellular', 'Never')
  await expectSaved(p, 'downloadUsingCellular', 'NEVER')
  await pick(p, 'Streaming using Cellular', 'Never')
  await expectSaved(p, 'streamingUsingCellular', 'NEVER')
  await type(p, 'Alphabetical drawdown limit', '50')
  await expectSaved(p, 'androidAutoBrowseLimitForGrouping', (v) => Number(v) === 50)
  await pick(p, 'Series books order', 'Sequence Descending')
  await expectSaved(p, 'androidAutoBrowseSeriesSequenceOrder', 'DESC')

  // language last: the page's words change with it
  await pick(p, 'Language', 'Deutsch')
  await p.goto(`${BASE}/settings/`, { waitUntil: 'networkidle' })
  await p.waitForTimeout(800)
  const lang = (await stored(p)).languageCode
  assert.equal(lang, 'de', 'languageCode')
  assert.equal(await p.getByText('Use bookshelf view', { exact: true }).count(), 0, 'the page is in German after a restart')
  console.log(`ok  languageCode = "${lang}", and the page is in German after a restart`)

  assert.deepEqual(errors, [], 'no script errors on the page')
  await browser.close()
  console.log('all device settings are saved and kept')
})().catch((e) => {
  console.error(e)
  process.exit(1)
})
