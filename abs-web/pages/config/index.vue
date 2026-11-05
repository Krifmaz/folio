<template>
  <div>
    <app-settings-content :header-text="$strings.HeaderSettings">
      <div class="lg:flex">
        <div class="flex-1">
          <div class="pt-4">
            <h2 class="font-semibold">{{ $strings.HeaderSettingsGeneral }}</h2>
          </div>
          <div role="article" :aria-label="$strings.LabelSettingsStoreCoversWithItemHelp" class="flex items-end py-2">
            <ui-toggle-switch :label="$strings.LabelSettingsStoreCoversWithItem" v-model="newServerSettings.storeCoverWithItem" :disabled="updatingServerSettings" @input="(val) => updateSettingsKey('storeCoverWithItem', val)" />
            <ui-tooltip aria-hidden="true" :text="$strings.LabelSettingsStoreCoversWithItemHelp">
              <p class="pl-4">
                <span id="settings-store-cover-with-items">{{ $strings.LabelSettingsStoreCoversWithItem }}</span>
                <span class="material-symbols icon-text">info</span>
              </p>
            </ui-tooltip>
          </div>

          <div role="article" :aria-label="$strings.LabelSettingsStoreMetadataWithItemHelp" class="flex items-center py-2">
            <ui-toggle-switch :label="$strings.LabelSettingsStoreMetadataWithItem" v-model="newServerSettings.storeMetadataWithItem" :disabled="updatingServerSettings" @input="(val) => updateSettingsKey('storeMetadataWithItem', val)" />
            <ui-tooltip aria-hidden="true" :text="$strings.LabelSettingsStoreMetadataWithItemHelp">
              <p class="pl-4">
                <span id="settings-store-metadata-with-items">{{ $strings.LabelSettingsStoreMetadataWithItem }}</span>
                <span class="material-symbols icon-text">info</span>
              </p>
            </ui-tooltip>
          </div>

          <div role="article" :aria-label="$strings.LabelSettingsSortingIgnorePrefixesHelp" class="flex items-center py-2">
            <ui-toggle-switch :label="$strings.LabelSettingsSortingIgnorePrefixes" v-model="newServerSettings.sortingIgnorePrefix" :disabled="updatingServerSettings" @input="(val) => updateSettingsKey('sortingIgnorePrefix', val)" />
            <ui-tooltip aria-hidden="true" :text="$strings.LabelSettingsSortingIgnorePrefixesHelp">
              <p class="pl-4">
                <span id="settings-sorting-ignore-prefixes">{{ $strings.LabelSettingsSortingIgnorePrefixes }}</span>
                <span class="material-symbols icon-text">info</span>
              </p>
            </ui-tooltip>
          </div>
          <div v-if="newServerSettings.sortingIgnorePrefix" class="w-72 ml-14 mb-2">
            <ui-multi-select v-model="newServerSettings.sortingPrefixes" small :items="newServerSettings.sortingPrefixes" :label="$strings.LabelPrefixesToIgnore" @input="sortingPrefixesUpdated" :disabled="savingPrefixes" />
            <div class="flex justify-end py-1">
              <ui-btn v-if="hasPrefixesChanged" color="bg-success" :loading="savingPrefixes" small @click="updateSortingPrefixes">Save</ui-btn>
            </div>
          </div>

          <div class="pt-4">
            <h2 class="font-semibold">{{ $strings.HeaderSettingsScanner }}</h2>
          </div>

          <div role="article" :aria-label="$strings.LabelSettingsParseSubtitlesHelp" class="flex items-center py-2">
            <ui-toggle-switch :label="$strings.LabelSettingsParseSubtitles" v-model="newServerSettings.scannerParseSubtitle" :disabled="updatingServerSettings" @input="(val) => updateSettingsKey('scannerParseSubtitle', val)" />
            <ui-tooltip aria-hidden="true" :text="$strings.LabelSettingsParseSubtitlesHelp">
              <p class="pl-4">
                <span id="settings-parse-subtitles">{{ $strings.LabelSettingsParseSubtitles }}</span>
                <span class="material-symbols icon-text">info</span>
              </p>
            </ui-tooltip>
          </div>

          <div role="article" :aria-label="$strings.LabelSettingsFindCoversHelp" class="flex items-center py-2">
            <ui-toggle-switch :label="$strings.LabelSettingsFindCovers" v-model="newServerSettings.scannerFindCovers" :disabled="updatingServerSettings" @input="(val) => updateSettingsKey('scannerFindCovers', val)" />
            <ui-tooltip aria-hidden="true" :text="$strings.LabelSettingsFindCoversHelp">
              <p class="pl-4">
                <span id="settings-find-covers">{{ $strings.LabelSettingsFindCovers }}</span>
                <span class="material-symbols icon-text">info</span>
              </p>
            </ui-tooltip>
            <div class="grow" />
          </div>
          <div v-if="newServerSettings.scannerFindCovers" class="w-44 ml-14 mb-2">
            <ui-dropdown v-model="newServerSettings.scannerCoverProvider" small :items="providers" :label="$strings.LabelCoverProvider" @input="updateScannerCoverProvider" :disabled="updatingServerSettings" />
          </div>

          <div role="article" :aria-label="$strings.LabelSettingsPreferMatchedMetadataHelp" class="flex items-center py-2">
            <ui-toggle-switch :label="$strings.LabelSettingsPreferMatchedMetadata" v-model="newServerSettings.scannerPreferMatchedMetadata" :disabled="updatingServerSettings" @input="(val) => updateSettingsKey('scannerPreferMatchedMetadata', val)" />
            <ui-tooltip aria-hidden="true" :text="$strings.LabelSettingsPreferMatchedMetadataHelp">
              <p class="pl-4">
                <span id="settings-prefer-matched-metadata">{{ $strings.LabelSettingsPreferMatchedMetadata }}</span>
                <span class="material-symbols icon-text">info</span>
              </p>
            </ui-tooltip>
          </div>

          <div role="article" :aria-label="$strings.LabelSettingsEnableWatcherHelp" class="flex items-center py-2">
            <ui-toggle-switch :label="$strings.LabelSettingsEnableWatcher" v-model="scannerEnableWatcher" :disabled="updatingServerSettings" @input="(val) => updateSettingsKey('scannerDisableWatcher', !val)" />
            <ui-tooltip aria-hidden="true" :text="$strings.LabelSettingsEnableWatcherHelp">
              <p class="pl-4">
                <span id="settings-disable-watcher">{{ $strings.LabelSettingsEnableWatcher }}</span>
                <span class="material-symbols icon-text">info</span>
              </p>
            </ui-tooltip>
          </div>

          <div class="pt-4">
            <h2 class="font-semibold">{{ $strings.HeaderSettingsWebClient }}</h2>
          </div>

          <div class="flex items-center py-2">
            <ui-toggle-switch v-model="newServerSettings.chromecastEnabled" :label="$strings.LabelSettingsChromecastSupport" :disabled="updatingServerSettings" @input="(val) => updateSettingsKey('chromecastEnabled', val)" />
            <p aria-hidden="true" class="pl-4">{{ $strings.LabelSettingsChromecastSupport }}</p>
          </div>

          <div class="flex items-center py-2 mb-2">
            <ui-toggle-switch v-model="newServerSettings.allowIframe" :label="$strings.LabelSettingsAllowIframe" :disabled="updatingServerSettings" @input="(val) => updateSettingsKey('allowIframe', val)" />
            <p aria-hidden="true" class="pl-4">{{ $strings.LabelSettingsAllowIframe }}</p>
          </div>
        </div>

        <div class="flex-1">
          <div class="pt-4">
            <h2 class="font-semibold">{{ $strings.HeaderSettingsDisplay }}</h2>
          </div>

          <div class="flex items-center py-2">
            <ui-toggle-switch labeledBy="settings-home-page-uses-bookshelf" v-model="homepageUseBookshelfView" :disabled="updatingServerSettings" @input="updateHomeUseBookshelfView" />
            <ui-tooltip :text="$strings.LabelSettingsBookshelfViewHelp">
              <p class="pl-4">
                <span id="settings-home-page-uses-bookshelf">{{ $strings.LabelSettingsHomePageBookshelfView }}</span>
                <span class="material-symbols icon-text">info</span>
              </p>
            </ui-tooltip>
          </div>

          <div class="flex items-center py-2">
            <ui-toggle-switch labeledBy="settings-library-uses-bookshelf" v-model="useBookshelfView" :disabled="updatingServerSettings" @input="updateUseBookshelfView" />
            <ui-tooltip :text="$strings.LabelSettingsBookshelfViewHelp">
              <p class="pl-4">
                <span id="settings-library-uses-bookshelf">{{ $strings.LabelSettingsLibraryBookshelfView }}</span>
                <span class="material-symbols icon-text">info</span>
              </p>
            </ui-tooltip>
          </div>

          <div class="grow py-2">
            <ui-dropdown :label="$strings.LabelSettingsDateFormat" v-model="newServerSettings.dateFormat" :items="dateFormats" small class="max-w-72" @input="(val) => updateSettingsKey('dateFormat', val)" />
            <p class="text-xs ml-1 text-white/60">{{ $strings.LabelExample }}: {{ dateExample }}</p>
          </div>

          <div class="grow py-2">
            <ui-dropdown :label="$strings.LabelSettingsTimeFormat" v-model="newServerSettings.timeFormat" :items="timeFormats" small class="max-w-72" @input="(val) => updateSettingsKey('timeFormat', val)" />
            <p class="text-xs ml-1 text-white/60">{{ $strings.LabelExample }}: {{ timeExample }}</p>
          </div>

          <div class="py-2">
            <ui-dropdown :label="$strings.LabelLanguageDefaultServer" ref="langDropdown" v-model="newServerSettings.language" :items="$languageCodeOptions" small class="max-w-72" @input="updateServerLanguage" />
          </div>

          <div class="pt-4">
            <h2 class="font-semibold">{{ $strings.HeaderSettingsSecurity }}</h2>
          </div>

          <div class="py-2">
            <ui-multi-select v-model="newServerSettings.allowedOrigins" :items="newServerSettings.allowedOrigins" :label="$strings.LabelCorsAllowed" class="max-w-72" @input="updateCorsOrigins" />
          </div>
        </div>
      </div>
    </app-settings-content>

    <div class="h-0.5 bg-primary/30 w-full" />

    <div class="flex items-center py-4">
      <div class="grow" />
      <ui-btn color="bg-bg" small :padding-x="4" class="mr-2 text-xs md:text-sm" :loading="isPurgingCache" @click.stop="purgeCache">{{ $strings.ButtonPurgeAllCache }}</ui-btn>
      <ui-btn color="bg-bg" small :padding-x="4" class="mr-2 text-xs md:text-sm" :loading="isPurgingCache" @click.stop="purgeItemsCache">{{ $strings.ButtonPurgeItemsCache }}</ui-btn>
    </div>

    <div class="flex items-center py-4">
      <div class="grow" />
      <p class="pr-2 text-sm text-yellow-400">
        {{ $strings.MessageReportBugsAndContribute }}
        <a class="underline" href="https://github.com/Krifmaz/folio" target="_blank">github</a>
      </p>
      <a href="https://github.com/Krifmaz/folio" target="_blank" class="text-white hover:text-gray-200 hover:scale-150 hover:rotate-6 transform duration-500">
        <svg xmlns="http://www.w3.org/2000/svg" fill="currentColor" width="24" height="24" viewBox="0 0 24 24">
          <path
            d="M12 0c-6.626 0-12 5.373-12 12 0 5.302 3.438 9.8 8.207 11.387.599.111.793-.261.793-.577v-2.234c-3.338.726-4.033-1.416-4.033-1.416-.546-1.387-1.333-1.756-1.333-1.756-1.089-.745.083-.729.083-.729 1.205.084 1.839 1.237 1.839 1.237 1.07 1.834 2.807 1.304 3.492.997.107-.775.418-1.305.762-1.604-2.665-.305-5.467-1.334-5.467-5.931 0-1.311.469-2.381 1.236-3.221-.124-.303-.535-1.524.117-3.176 0 0 1.008-.322 3.301 1.23.957-.266 1.983-.399 3.003-.404 1.02.005 2.047.138 3.006.404 2.291-1.552 3.297-1.23 3.297-1.23.653 1.653.242 2.874.118 3.176.77.84 1.235 1.911 1.235 3.221 0 4.609-2.807 5.624-5.479 5.921.43.372.823 1.102.823 2.222v3.293c0 .319.192.694.801.576 4.765-1.589 8.199-6.086 8.199-11.386 0-6.627-5.373-12-12-12z"
          />
        </svg>
      </a>
    </div>

    <div class="h-0.5 bg-primary/30 w-full" />

    <!-- confirm cache purge dialog -->
    <prompt-dialog v-model="showConfirmPurgeCache" :width="675">
      <div class="px-4 w-full text-sm py-6 rounded-lg bg-bg shadow-lg border border-black-300">
        <p class="text-error font-semibold">{{ $strings.MessageImportantNotice }}</p>
        <p class="my-8 text-center" v-html="$strings.MessageConfirmPurgeCache" />
        <div class="flex px-1 items-center">
          <ui-btn color="bg-primary" @click="showConfirmPurgeCache = false">{{ $strings.ButtonNevermind }}</ui-btn>
          <div class="grow" />
          <ui-btn color="bg-success" @click="confirmPurge">{{ $strings.ButtonYes }}</ui-btn>
        </div>
      </div>
    </prompt-dialog>
  </div>
</template>

<script>
export default {
  asyncData({ store, redirect }) {
    if (!store.getters['user/getIsAdminOrUp']) {
      redirect('/')
    }
  },
  data() {
    return {
      isResettingLibraryItems: false,
      updatingServerSettings: false,
      homepageUseBookshelfView: false,
      useBookshelfView: false,
      scannerEnableWatcher: false,
      isPurgingCache: false,
      hasPrefixesChanged: false,
      newServerSettings: {},
      showConfirmPurgeCache: false,
      savingPrefixes: false
    }
  },
  watch: {
    serverSettings(newVal, oldVal) {
      if (newVal && !oldVal) {
        this.initServerSettings()
      }
    }
  },
  computed: {
    serverSettings() {
      return this.$store.state.serverSettings
    },
    providers() {
      // Use book cover providers for the cover provider dropdown
      return this.$store.state.scanners.bookCoverProviders || []
    },
    dateFormats() {
      return this.$store.state.globals.dateFormats
    },
    timeFormats() {
      return this.$store.state.globals.timeFormats
    },
    dateExample() {
      const date = new Date(2014, 2, 25)
      return this.$formatJsDate(date, this.newServerSettings.dateFormat)
    },
    timeExample() {
      const date = new Date(2014, 2, 25, 17, 30, 0)
      return this.$formatJsTime(date, this.newServerSettings.timeFormat)
    }
  },
  methods: {
    sortingPrefixesUpdated(val) {
      const prefixes = [...new Set(val?.map((prefix) => prefix.trim().toLowerCase()) || [])]
      this.newServerSettings.sortingPrefixes = prefixes
      const serverPrefixes = this.serverSettings.sortingPrefixes || []
      this.hasPrefixesChanged = prefixes.some((p) => !serverPrefixes.includes(p)) || serverPrefixes.some((p) => !prefixes.includes(p))
    },
    updateSortingPrefixes() {
      const prefixes = [...new Set(this.newServerSettings.sortingPrefixes.map((prefix) => prefix.trim().toLowerCase()) || [])]
      if (!prefixes.length) {
        this.$toast.error(this.$strings.ToastSortingPrefixesEmptyError)
        return
      }

      this.savingPrefixes = true
      this.$axios
        .$patch(`/api/sorting-prefixes`, { sortingPrefixes: prefixes })
        .then((data) => {
          this.$toast.success(this.$getString('ToastSortingPrefixesUpdateSuccess', [data.rowsUpdated]))
          if (data.serverSettings) {
            this.$store.commit('setServerSettings', data.serverSettings)
          }
          this.hasPrefixesChanged = false
        })
        .catch((error) => {
          console.error('Failed to update prefixes', error)
          this.$toast.error(this.$strings.ToastFailedToUpdate)
        })
        .finally(() => {
          this.savingPrefixes = false
        })
    },
    updateScannerCoverProvider(val) {
      this.updateServerSettings({
        scannerCoverProvider: val
      })
    },
    updateHomeUseBookshelfView(val) {
      this.updateServerSettings({
        homeBookshelfView: !val ? this.$constants.BookshelfView.DETAIL : this.$constants.BookshelfView.STANDARD
      })
    },
    updateUseBookshelfView(val) {
      this.updateServerSettings({
        bookshelfView: !val ? this.$constants.BookshelfView.DETAIL : this.$constants.BookshelfView.STANDARD
      })
    },
    updateServerLanguage(val) {
      this.updateSettingsKey('language', val)
    },
    updateCorsOrigins(val) {
      const validOrigins = []
      const invalidOrigins = []

      val.forEach((origin) => {
        const trimmedOrigin = origin.trim().toLowerCase()
        try {
          new URL(trimmedOrigin)
          validOrigins.push(trimmedOrigin)
        } catch {
          invalidOrigins.push(trimmedOrigin)
        }
      })

      if (invalidOrigins.length > 0) {
        this.$toast.error(this.$strings.ToastInvalidUrls)
      }

      this.newServerSettings.allowedOrigins = validOrigins
      this.updateSettingsKey('allowedOrigins', validOrigins)
    },
    updateSettingsKey(key, val) {
      if (key === 'scannerDisableWatcher') {
        this.newServerSettings.scannerDisableWatcher = val
      }
      this.updateServerSettings({
        [key]: val
      })
    },
    updateServerSettings(payload) {
      this.updatingServerSettings = true
      this.$store.dispatch('updateServerSettings', payload).then((response) => {
        this.updatingServerSettings = false

        if (response.error) {
          console.error('Failed to update server settins', response.error)
          this.$toast.error(response.error)
          this.initServerSettings()
          return
        }

        if (payload.language) {
          // Updating language after save allows for re-rendering
          this.$setLanguageCode(payload.language)
        }
      })
    },
    initServerSettings() {
      this.newServerSettings = this.serverSettings ? { ...this.serverSettings } : {}
      this.newServerSettings.sortingPrefixes = [...(this.newServerSettings.sortingPrefixes || [])]
      this.newServerSettings.allowedOrigins = [...(this.newServerSettings.allowedOrigins || [])]
      this.scannerEnableWatcher = !this.newServerSettings.scannerDisableWatcher

      this.homepageUseBookshelfView = this.newServerSettings.homeBookshelfView != this.$constants.BookshelfView.DETAIL
      this.useBookshelfView = this.newServerSettings.bookshelfView != this.$constants.BookshelfView.DETAIL
    },
    purgeCache() {
      this.showConfirmPurgeCache = true
    },
    async confirmPurge() {
      this.showConfirmPurgeCache = false
      this.isPurgingCache = true
      await this.$axios
        .$post('/api/cache/purge')
        .then(() => {
          this.$toast.success(this.$strings.ToastCachePurgeSuccess)
        })
        .catch((error) => {
          console.error('Failed to purge cache', error)
          this.$toast.error(this.$strings.ToastCachePurgeFailed)
        })
      this.isPurgingCache = false
    },
    purgeItemsCache() {
      const payload = {
        message: this.$strings.MessageConfirmPurgeItemsCache,
        allowHtml: true,
        callback: (confirmed) => {
          if (confirmed) {
            this.sendPurgeItemsCache()
          }
        },
        type: 'yesNo'
      }
      this.$store.commit('globals/setConfirmPrompt', payload)
    },
    async sendPurgeItemsCache() {
      this.isPurgingCache = true
      await this.$axios
        .$post('/api/cache/items/purge')
        .then(() => {
          this.$toast.success(this.$strings.ToastCachePurgeSuccess)
        })
        .catch((error) => {
          console.error('Failed to purge items cache', error)
          this.$toast.error(this.$strings.ToastCachePurgeFailed)
        })
      this.isPurgingCache = false
    }
  },
  mounted() {
    this.initServerSettings()
    // Fetch providers if not already loaded (for cover provider dropdown)
    this.$store.dispatch('scanners/fetchProviders')
  }
}
</script>
