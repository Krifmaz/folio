package dev.krifmaz.folio

import android.content.ComponentName
import android.content.Context
import android.net.Uri
import androidx.core.content.ContextCompat
import androidx.media3.common.MediaItem
import androidx.media3.common.MediaMetadata
import androidx.media3.session.MediaController
import androidx.media3.session.SessionToken
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * The app's line to [PlayerService]: one Media3 controller for the whole app, made on first use (which starts the
 * service). Use it on the main thread.
 */
class PlayerConnection(context: Context) {
    private val context = context.applicationContext
    private val flow = MutableStateFlow<MediaController?>(null)
    private var asking = false

    /** The controller once connected (null before). */
    val controller: StateFlow<MediaController?> = flow.asStateFlow()

    /** Connects to the player service when not yet connected. */
    fun connect() {
        if (flow.value != null || asking) return
        asking = true
        val token = SessionToken(context, ComponentName(context, PlayerService::class.java))
        val future = MediaController.Builder(context, token).buildAsync()
        future.addListener(
            {
                asking = false
                flow.value = try {
                    future.get()
                } catch (_: Exception) {
                    null
                }
            },
            ContextCompat.getMainExecutor(context),
        )
    }

    /**
     * Loads [book] into the player at [positionMs] (in the whole book), each file from [uriOf] (the server's stream or
     * the phone's download), paused. A book already loaded only moves when [positionMs] is given again.
     */
    fun load(book: NowPlaying.Book, positionMs: Long, speed: Float, uriOf: (BookFile) -> Uri) {
        val c = flow.value ?: return
        val authors = LibraryRules.names(book.edition.authors.map { it.name })
        val items = book.audio.mapIndexed { i, f ->
            val meta = MediaMetadata
                .Builder()
                .setTitle(book.edition.title)
                .setArtist(authors)
                .setAlbumTitle(book.edition.title)
                .setTrackNumber(i + 1)
                .setTotalTrackCount(book.audio.size)
                .setIsPlayable(true)
                .setMediaType(MediaMetadata.MEDIA_TYPE_AUDIO_BOOK_CHAPTER)
                .apply { if (book.edition.coverUrl.isNotBlank()) setArtworkUri(Uri.parse(book.edition.coverUrl)) }
                .build()
            MediaItem
                .Builder()
                .setMediaId("${book.edition.id}:$i")
                .setUri(uriOf(f))
                .setMimeType(f.mime.ifBlank { null })
                .setMediaMetadata(meta)
                .build()
        }
        NowPlaying.load(book)
        val place = PlaybackRules.place(book.audio, positionMs)
        c.setMediaItems(items, place.index, place.offsetMs)
        c.setPlaybackSpeed(speed)
        c.prepare()
    }

    /** Jumps to [positionMs] in the whole loaded book. */
    fun seekTo(positionMs: Long) {
        val c = flow.value ?: return
        val b = NowPlaying.book.value ?: return
        val p = PlaybackRules.place(b.audio, positionMs.coerceIn(0, b.totalMs))
        c.seekTo(p.index, p.offsetMs)
    }

    /** Moves by [deltaMs] (back when negative) in the whole book, across files. */
    fun skip(deltaMs: Long) {
        val c = flow.value ?: return
        seekTo(NowPlaying.positionOf(c) + deltaMs)
    }
}

/** The player's choices kept on the phone: the speed (one for every book). */
class PlayerPrefs(context: Context) {
    private val prefs = context.applicationContext.getSharedPreferences("player", Context.MODE_PRIVATE)

    /** The speed books play at (1× until changed). */
    var speed: Float
        get() = prefs.getFloat(K_SPEED, 1f).takeIf { it in 0.5f..3.5f } ?: 1f
        set(value) = prefs.edit().putFloat(K_SPEED, value).apply()

    private companion object {
        const val K_SPEED = "speed"
    }
}
