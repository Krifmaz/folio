# Folio's fork of the Audiobookshelf app

`abs-app/` is [audiobookshelf-app](https://github.com/advplyr/audiobookshelf-app) at the tag in
[UPSTREAM.lock](../UPSTREAM.lock), with Folio's changes on top. Upstream updates come in with
`scripts/upstream-merge.sh abs-app` as merge commits, so every change here is kept small and, where it can be, in
files of its own.

| Change | Where |
| --- | --- |
| Version from the release workflow (`-PappVersionName`, `-PappVersionCode`), release signing from the environment (the debug key without one) | `android/app/folio.gradle`, applied by one line at the end of `android/app/build.gradle` |

## Build

CI builds it on every pull request that touches `abs-app/` (`.github/workflows/abs-app.yml`, through the kit's
`kit-android.yml`). By hand, with Node 22, Java 17 and the Android SDK:

```sh
cd abs-app
npm ci
npx nuxt generate && npx cap sync android
cd android && ./gradlew :app:assembleDebug
```
