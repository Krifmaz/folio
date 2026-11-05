# Folio's fork of the Audiobookshelf app

`abs-app/` is [audiobookshelf-app](https://github.com/advplyr/audiobookshelf-app) at the tag in
[UPSTREAM.lock](../UPSTREAM.lock), with Folio's changes on top. Upstream updates come in with
`scripts/upstream-merge.sh abs-app` as merge commits, so every change here is kept small and, where it can be, in
files of its own.

| Change | Where |
| --- | --- |
| Version from the release workflow (`-PappVersionName`, `-PappVersionCode`), release signing from the environment (the debug key without one) | `android/app/folio.gradle`, applied by one line at the end of `android/app/build.gradle` |
| Encore look: the app's colours for the default, `black` (kit oled) and `light` themes, the Inter font and the accent | `assets/folio-theme.css` (loaded last in `nuxt.config.js`), `accent` in `tailwind.config.js` |
| Reader's dark theme in the Encore colours | `components/readers/Reader.vue`, `components/readers/EpubReader.vue` |
| App id `dev.krifmaz.folio.mobile` (installs beside the upstream app), name Folio, status bar, widget colours, launcher, notification and in-app icons | `android/app/folio.gradle`, `android/app/folio-res/` (laid over the upstream resources), `capacitor.config.json`, `static/Logo.png`; icons drawn by `folio/make_icons.py` |
| Android Auto and other media browsers accept Folio's app id | `PlayerNotificationService.kt` (`VALID_MEDIA_BROWSERS`) |
| No links out to the upstream GitHub, guides or discussions | `pages/connect.vue`, `pages/account.vue`, `pages/localMedia/folders/index.vue`, `components/connection/ServerConnectForm.vue` |

The `audiobookshelf://oauth` sign-in redirect is kept, so single sign-on works the same against Folio's
Audiobookshelf-compatible API.

## Build

CI builds it on every pull request that touches `abs-app/` (`.github/workflows/abs-app.yml`, through the kit's
`kit-android.yml`). By hand, with Node 22, Java 17 and the Android SDK:

```sh
cd abs-app
npm ci
npx nuxt generate && npx cap sync android
cd android && ./gradlew :app:assembleDebug
```
