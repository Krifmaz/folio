<template>
  <div class="w-full h-full p-4">
    <ui-text-input-with-label :value="serverAddress" :label="$strings.LabelHost" disabled class="my-2" />

    <ui-text-input-with-label :value="username" :label="$strings.LabelUsername" disabled class="my-2" />

    <div v-if="serverVersion" class="text-sm text-fg">
      <p>Server version: v{{ serverVersion }}</p>
    </div>

    <ui-btn color="primary flex items-center justify-between gap-2 ml-auto text-base mt-8" @click="logout">{{ $strings.ButtonSwitchServerUser }}<span class="material-symbols" style="font-size: 1.1rem">logout</span></ui-btn>

  </div>
</template>

<script>
export default {
  asyncData({ redirect, store }) {
    if (!store.state.socketConnected) {
      return redirect('/connect')
    }
    return {}
  },
  data() {
    return {}
  },
  computed: {
    username() {
      if (!this.user) return ''
      return this.user.username
    },
    user() {
      return this.$store.state.user.user
    },
    serverConnectionConfig() {
      return this.$store.state.user.serverConnectionConfig || {}
    },
    serverAddress() {
      return this.serverConnectionConfig.address
    },
    serverVersion() {
      // Saved in server connection config after 0.9.81
      return this.serverConnectionConfig.version
    }
  },
  methods: {
    async logout() {
      await this.$hapticsImpact()
      await this.$store.dispatch('user/logout')
      this.$router.push('/connect')
    }
  },
  mounted() {}
}
</script>
