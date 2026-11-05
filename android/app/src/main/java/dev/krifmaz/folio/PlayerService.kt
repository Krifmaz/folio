package dev.krifmaz.folio

import android.app.PendingIntent
import android.content.Intent
import android.os.Handler
import android.os.Looper
import androidx.media3.common.AudioAttributes
import androidx.media3.common.C
import androidx.media3.common.Player
import androidx.media3.datasource.DefaultDataSource
import androidx.media3.datasource.okhttp.OkHttpDataSource
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.exoplayer.source.DefaultMediaSourceFactory
import androidx.media3.session.MediaSession
import androidx.media3.session.MediaSessionService
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * The audiobook that is loaded in the player, shared by the player page and [PlayerService] (one process): the
 * edition, its audio files in play order, its chapters across the files, and the sleep timer.
 */
object NowPlaying {
    /** A loaded audiobook. */
    data class Book(
        /** the edition playing */
        val edition: Edition,
        /** its audio files, in play order */
        val audio: List<BookFile>,
        /** its chapters, counted from the start of the book */
        val chapters: List<ChapterMark>,
    ) {
        /** How long the whole book runs. */
        val totalMs: Long get() = PlaybackRules.total(audio)
    }

    /** A sleep timer that is set, and when it was set (the clock's milliseconds). */
    data class Sleep(val timer: SleepTimer, val setAtMs: Long)

    private val bookFlow = MutableStateFlow<Book?>(null)
    private val sleepFlow = MutableStateFlow<Sleep?>(null)

    /** The book loaded, or null. */
    val book: StateFlow<Book?> = bookFlow.asStateFlow()

    /** The sleep timer, or null when off. */
    val sleep: StateFlow<Sleep?> = sleepFlow.asStateFlow()

    /** Loads [b] (the player is given its files by the page). The sleep timer goes off. */
    fun load(b: Book) {
        bookFlow.value = b
        sleepFlow.value = null
    }

    /** Sets the sleep timer, or turns it off with null. */
    fun setSleep(timer: SleepTimer?) {
        sleepFlow.value = timer?.let { Sleep(it, System.currentTimeMillis()) }
    }

    /** Where the player is in the whole book (across its files). */
    fun positionOf(player: Player): Long {
        val b = bookFlow.value ?: return 0
        return PlaybackRules.position(b.audio, player.currentMediaItemIndex, player.currentPosition)
    }

    /** How long until the sleep timer stops the book (null when off). */
    fun sleepLeftMs(player: Player): Long? {
        val s = sleepFlow.value ?: return null
        val b = bookFlow.value ?: return null
        return PlaybackRules.sleepLeftMs(s.timer, s.setAtMs, System.currentTimeMillis(), b.chapters, positionOf(player), b.totalMs, player.playbackParameters.speed)
    }
}

/**
 * Plays audiobooks in the background: Media3's player in a media session, so the notification, the lock screen,
 * headphones, Bluetooth and Android Auto can control it. Files stream from the server with the app's sign-in (or
 * play from the phone once downloaded). While playing it saves the place every [PlaybackRules.SAVE_EVERY_MS], on
 * pause and on every jump, and stops the book when the sleep timer runs out.
 */
class PlayerService : MediaSessionService() {
    private var session: MediaSession? = null
    private val ticker = Handler(Looper.getMainLooper())
    private var lastSave = 0L

    /** Builds the player (streaming through the app's own HTTP client) and its session. */
    override fun onCreate() {
        super.onCreate()
        val app = application as App
        val http = OkHttpDataSource.Factory(app.server.http)
        val sources = DefaultMediaSourceFactory(DefaultDataSource.Factory(this, http))
        val player = ExoPlayer
            .Builder(this)
            .setMediaSourceFactory(sources)
            .setAudioAttributes(AudioAttributes.Builder().setUsage(C.USAGE_MEDIA).setContentType(C.AUDIO_CONTENT_TYPE_SPEECH).build(), true)
            .setHandleAudioBecomingNoisy(true)
            .setWakeMode(C.WAKE_MODE_NETWORK)
            .setSeekBackIncrementMs(PlaybackRules.SKIP_BACK_MS)
            .setSeekForwardIncrementMs(PlaybackRules.SKIP_FORWARD_MS)
            .build()
        player.addListener(
            object : Player.Listener {
                override fun onIsPlayingChanged(isPlaying: Boolean) {
                    if (!isPlaying) save(player)
                }

                override fun onPositionDiscontinuity(oldPosition: Player.PositionInfo, newPosition: Player.PositionInfo, reason: Int) {
                    if (reason == Player.DISCONTINUITY_REASON_SEEK) save(player)
                }
            },
        )
        val open = PendingIntent.getActivity(this, 0, Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP), PendingIntent.FLAG_IMMUTABLE)
        session = MediaSession.Builder(this, player).setSessionActivity(open).build()
        ticker.post(tick(player))
    }

    /** The one session: every controller (the app, the notification, Bluetooth, Auto) gets it. */
    override fun onGetSession(controllerInfo: MediaSession.ControllerInfo): MediaSession? = session

    /** Swiped away from the recent apps while paused: stop the service (the place is saved). */
    override fun onTaskRemoved(rootIntent: Intent?) {
        val p = session?.player
        if (p == null || !p.playWhenReady || p.mediaItemCount == 0) {
            p?.let(::save)
            stopSelf()
        }
    }

    /** Saves the place and lets the player go. */
    override fun onDestroy() {
        ticker.removeCallbacksAndMessages(null)
        session?.run {
            save(player)
            player.release()
            release()
        }
        session = null
        super.onDestroy()
    }

    /** Once a second: the sleep timer, and the place saved every 15 seconds while playing. */
    private fun tick(player: Player): Runnable = object : Runnable {
        override fun run() {
            if (player.isPlaying) {
                NowPlaying.sleepLeftMs(player)?.let { left ->
                    if (left <= 0) {
                        player.pause()
                        NowPlaying.setSleep(null)
                    } else if (left < FADE_MS) {
                        // the last seconds fade out, so sleep isn't broken by a sudden stop
                        player.volume = (left.toFloat() / FADE_MS).coerceIn(0.1f, 1f)
                    }
                }
                if (System.currentTimeMillis() - lastSave >= PlaybackRules.SAVE_EVERY_MS) save(player)
            }
            if (NowPlaying.sleep.value == null && player.volume < 1f) player.volume = 1f
            ticker.postDelayed(this, 1000)
        }
    }

    /** Saves where the player is in the loaded book (nothing when no book is loaded). */
    private fun save(player: Player) {
        val b = NowPlaying.book.value ?: return
        if (player.mediaItemCount == 0 || player.currentMediaItem?.mediaId?.startsWith("${b.edition.id}:") != true) return
        lastSave = System.currentTimeMillis()
        val app = application as App
        val p = PlaybackRules.audioProgress(b.edition.id, NowPlaying.positionOf(player), b.totalMs, app.sync.device, lastSave / 1000.0)
        app.sync.saveLater(p)
    }

    private companion object {
        const val FADE_MS = 10_000L
    }
}
