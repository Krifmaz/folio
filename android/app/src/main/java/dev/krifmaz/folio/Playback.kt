package dev.krifmaz.folio

import dev.krifmaz.kit.compose.core.arr
import dev.krifmaz.kit.compose.core.bool
import dev.krifmaz.kit.compose.core.dbl
import dev.krifmaz.kit.compose.core.long
import dev.krifmaz.kit.compose.core.objectsOf
import dev.krifmaz.kit.compose.core.parseJsonObject
import dev.krifmaz.kit.compose.core.str
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

/** One file of an edition (`GET /api/library/editions/{id}/files`). */
data class BookFile(
    /** the file's id (`/api/files/{id}`) */
    val id: Long,
    /** the file name shown to people */
    val name: String,
    /** `audio`, `epub`, `pdf`, `azw3` or `cover` */
    val kind: String,
    /** the file's type, e.g. `audio/mp4` */
    val mime: String,
    /** its size in bytes */
    val size: Long,
    /** how long an audio file runs, in milliseconds (0 otherwise) */
    val durationMs: Long,
    /** an audio file's chapters, counted from the start of this file */
    val chapters: List<ChapterMark>,
)

/**
 * Where someone is in a book (`/api/progress/{edition_id}`). Audio keeps [positionMs] across all its files; an
 * ebook keeps a [Locator]. The last writer wins by [updatedAt].
 */
data class Progress(
    /** the edition */
    val editionId: Long,
    /** audio: where in the whole book, in milliseconds */
    val positionMs: Long = 0,
    /** ebook: where in the book (null for audio) */
    val locator: Locator? = null,
    /** how far through the book, 0 to 1 */
    val fraction: Double = 0.0,
    /** read or listened to the end */
    val finished: Boolean = false,
    /** which app wrote it: `android:<device id>`, `web`, `koreader:<name>`… */
    val device: String = "",
    /** when it was read or listened to, Unix seconds */
    val updatedAt: Double = 0.0,
) {
    /** The body of `PUT /api/progress/{edition_id}` (also how the phone keeps it). */
    fun toJson(): JsonObject = buildJsonObject {
        put("edition_id", editionId)
        put("position_ms", positionMs)
        if (locator != null) put("locator", locator.toJson()) else put("locator", JsonNull)
        put("fraction", fraction)
        put("finished", finished)
        put("device", device)
        put("updated_at", updatedAt)
    }
}

/** Where in an EPUB: the spine item, how far through it and through the book, and a CFI when known. */
data class Locator(
    /** the spine item's path inside the EPUB (`OEBPS/ch03.xhtml`) */
    val href: String,
    /** how far through that item, 0 to 1 */
    val progression: Double,
    /** how far through the whole book, 0 to 1 */
    val totalProgression: Double,
    /** an EPUB CFI ("" when not known) */
    val cfi: String = "",
) {
    /** The locator as the server keeps it. */
    fun toJson(): JsonObject = buildJsonObject {
        put("href", href)
        put("progression", progression)
        put("total_progression", totalProgression)
        if (cfi.isNotEmpty()) put("cfi", cfi)
    }
}

/** A bookmark (`/api/editions/{id}/bookmarks`): a place in a book with a note. */
data class Bookmark(
    /** the bookmark's id (0 while it waits to be sent) */
    val id: Long,
    /** audio: where in the whole book */
    val positionMs: Long,
    /** ebook: where in the book */
    val locator: Locator?,
    /** what the person wrote ("" for none) */
    val note: String,
    /** when it was made, Unix seconds */
    val createdAt: Double,
)

/** A place in a multi-file audiobook: which file, and how far into it. */
data class FilePlace(
    /** the file's index in play order */
    val index: Int,
    /** milliseconds from the start of that file */
    val offsetMs: Long,
)

/** What the sleep timer waits for: a number of minutes, or the end of the chapter playing. */
sealed interface SleepTimer {
    /** Stop when this many milliseconds of the clock have passed. */
    data class After(val ms: Long) : SleepTimer

    /** Stop where the chapter playing ends. */
    data object EndOfChapter : SleepTimer
}

/** Reading the playback answers and the player's arithmetic. Plain Kotlin, so it is unit-tested. */
object PlaybackRules {
    /** The speeds offered, slowest first. */
    val SPEEDS: List<Float> = listOf(0.75f, 1.0f, 1.1f, 1.25f, 1.5f, 1.75f, 2.0f, 2.5f, 3.0f)

    /** The sleep timer's choices, in minutes. */
    val SLEEP_MINUTES: List<Int> = listOf(5, 10, 15, 30, 45, 60, 90)

    /** How far Back and Forward jump. */
    const val SKIP_BACK_MS: Long = 15_000
    const val SKIP_FORWARD_MS: Long = 30_000

    /** How often the place is saved while playing or reading. */
    const val SAVE_EVERY_MS: Long = 15_000

    /** The last this much of a book counts as the end (credits, the publisher's outro). */
    const val FINISHED_WITHIN_MS: Long = 30_000

    /** The files of `GET /api/library/editions/{id}/files`. */
    fun filesOf(o: JsonObject): List<BookFile> = o.arr("files").objectsOf().map { f ->
        BookFile(
            id = f.long("id"),
            name = f.str("name"),
            kind = f.str("kind"),
            mime = f.str("mime"),
            size = f.long("size"),
            durationMs = f.long("duration_ms"),
            chapters = f.arr("chapters").objectsOf().map { ChapterMark(it.str("title"), it.long("start_ms"), it.long("end_ms")) },
        )
    }

    /** The audio files in play order (the server lists them by position). */
    fun audio(files: List<BookFile>): List<BookFile> = files.filter { it.kind == "audio" }

    /** How long the whole audiobook runs. */
    fun total(audio: List<BookFile>): Long = audio.sumOf { it.durationMs }

    /** The place [positionMs] (in the whole book) is at: the file and the offset in it; past the end, the end. */
    fun place(audio: List<BookFile>, positionMs: Long): FilePlace {
        if (audio.isEmpty()) return FilePlace(0, 0)
        var left = positionMs.coerceAtLeast(0)
        for ((i, f) in audio.withIndex()) {
            if (left < f.durationMs || i == audio.lastIndex) return FilePlace(i, left.coerceAtMost(f.durationMs.coerceAtLeast(0)))
            left -= f.durationMs
        }
        return FilePlace(audio.lastIndex, audio.last().durationMs)
    }

    /** Where in the whole book the place [index] / [offsetMs] is. */
    fun position(audio: List<BookFile>, index: Int, offsetMs: Long): Long = audio.take(index.coerceIn(0, audio.size)).sumOf { it.durationMs } + offsetMs.coerceAtLeast(0)

    /**
     * The book's chapters across all its files, counted from the start of the book. Files without chapters of
     * their own count as one chapter each (named after the file); with none at all, [fallback] (the metadata's).
     */
    fun bookChapters(audio: List<BookFile>, fallback: List<ChapterMark> = emptyList()): List<ChapterMark> {
        if (audio.none { it.chapters.isNotEmpty() } && fallback.isNotEmpty() && audio.size <= 1) return fallback
        val out = mutableListOf<ChapterMark>()
        var start = 0L
        for (f in audio) {
            if (f.chapters.isEmpty()) {
                out += ChapterMark(f.name.substringBeforeLast('.'), start, start + f.durationMs)
            } else {
                f.chapters.forEachIndexed { i, c ->
                    val end = if (c.endMs > 0) c.endMs else f.chapters.getOrNull(i + 1)?.startMs ?: f.durationMs
                    out += ChapterMark(c.title, start + c.startMs, start + end)
                }
            }
            start += f.durationMs
        }
        return out
    }

    /** The index of the chapter [positionMs] is in (-1 with no chapters). */
    fun chapterAt(chapters: List<ChapterMark>, positionMs: Long): Int {
        if (chapters.isEmpty()) return -1
        val i = chapters.indexOfLast { it.startMs <= positionMs }
        return i.coerceAtLeast(0)
    }

    /**
     * Where Previous chapter goes: the start of this chapter, or the one before when this one started less than 3
     * seconds ago (as a CD player does).
     */
    fun previousChapterStart(chapters: List<ChapterMark>, positionMs: Long): Long {
        val i = chapterAt(chapters, positionMs)
        if (i < 0) return 0
        val c = chapters[i]
        return if (positionMs - c.startMs < 3_000 && i > 0) chapters[i - 1].startMs else c.startMs
    }

    /** Where Next chapter goes: the next chapter's start, or null on the last one. */
    fun nextChapterStart(chapters: List<ChapterMark>, positionMs: Long): Long? {
        val i = chapterAt(chapters, positionMs)
        return chapters.getOrNull(i + 1)?.startMs
    }

    /** The next speed up from [speed] (back to the slowest after the fastest). */
    fun nextSpeed(speed: Float): Float = SPEEDS.firstOrNull { it > speed + 0.001f } ?: SPEEDS.first()

    /** "1×", "1.25×". */
    fun speedLabel(speed: Float): String {
        val s = if (speed == speed.toInt().toFloat()) speed.toInt().toString() else "%.2f".format(java.util.Locale.ROOT, speed).trimEnd('0').trimEnd('.')
        return "$s×"
    }

    /** "1:02:03" or "2:03". */
    fun clock(ms: Long): String {
        val t = (ms.coerceAtLeast(0) / 1000)
        val h = t / 3600
        val m = (t % 3600) / 60
        val s = t % 60
        return if (h > 0) "%d:%02d:%02d".format(java.util.Locale.ROOT, h, m, s) else "%d:%02d".format(java.util.Locale.ROOT, m, s)
    }

    /** "2 h 5 min left", "12 min left", "Less than a minute left", for what is left at [speed]. */
    fun left(totalMs: Long, positionMs: Long, speed: Float): String {
        val ms = ((totalMs - positionMs).coerceAtLeast(0) / speed.coerceAtLeast(0.1f)).toLong()
        val min = ms / 60_000
        return when {
            min < 1 -> "Less than a minute left"
            min < 60 -> "$min min left"
            min % 60 == 0L -> "${min / 60} h left"
            else -> "${min / 60} h ${min % 60} min left"
        }
    }

    /**
     * How long until the sleep timer stops the book, in milliseconds of the clock: the time left for [After][SleepTimer.After]
     * (counted from [setAtMs]), or what is left of the chapter at [speed] for the end of the chapter.
     */
    fun sleepLeftMs(timer: SleepTimer, setAtMs: Long, nowMs: Long, chapters: List<ChapterMark>, positionMs: Long, totalMs: Long, speed: Float): Long = when (timer) {
        is SleepTimer.After -> (setAtMs + timer.ms - nowMs).coerceAtLeast(0)

        SleepTimer.EndOfChapter -> {
            val i = chapterAt(chapters, positionMs)
            val end = if (i < 0) totalMs else chapters[i].endMs.takeIf { it > 0 } ?: totalMs
            ((end - positionMs).coerceAtLeast(0) / speed.coerceAtLeast(0.1f)).toLong()
        }
    }

    /** The audiobook's place as progress: the fraction, and finished in the last [FINISHED_WITHIN_MS]. */
    fun audioProgress(editionId: Long, positionMs: Long, totalMs: Long, device: String, nowSeconds: Double): Progress {
        val fraction = if (totalMs <= 0) 0.0 else (positionMs.toDouble() / totalMs).coerceIn(0.0, 1.0)
        val finished = totalMs > 0 && totalMs - positionMs <= FINISHED_WITHIN_MS
        return Progress(editionId, positionMs = positionMs, fraction = fraction, finished = finished, device = device, updatedAt = nowSeconds)
    }

    /** Progress from the server's JSON (or the phone's copy); null for anything without an edition. */
    fun progressOf(o: JsonObject?): Progress? {
        if (o == null || o.long("edition_id") <= 0) return null
        return Progress(
            editionId = o.long("edition_id"),
            positionMs = o.long("position_ms"),
            locator = locatorOf(o["locator"]),
            fraction = o.dbl("fraction"),
            finished = o.bool("finished"),
            device = o.str("device"),
            updatedAt = o.dbl("updated_at"),
        )
    }

    /** A locator from JSON: an object, or (as some servers keep it) a JSON text of one. */
    fun locatorOf(e: kotlinx.serialization.json.JsonElement?): Locator? {
        val o = when (e) {
            is JsonObject -> e
            null, JsonNull -> null
            is JsonPrimitive -> parseJsonObject(e.content)
            else -> null
        } ?: return null
        val href = o.str("href")
        if (href.isEmpty()) return null
        return Locator(href, o.dbl("progression"), o.dbl("total_progression"), o.str("cfi"))
    }

    /** The list of `GET /api/progress` (`{progress: [...]}`). */
    fun progressListOf(o: JsonObject): List<Progress> = o.arr("progress").objectsOf().mapNotNull(::progressOf)

    /** The bookmarks of `GET /api/editions/{id}/bookmarks`, oldest first. */
    fun bookmarksOf(o: JsonObject): List<Bookmark> = o.arr("bookmarks").objectsOf().map {
        Bookmark(it.long("id"), it.long("position_ms"), locatorOf(it["locator"]), it.str("note"), it.dbl("created_at"))
    }.sortedBy { it.createdAt }

    /** The body of `POST /api/editions/{id}/bookmarks`. */
    fun bookmarkBody(positionMs: Long?, locator: Locator?, note: String): JsonObject = buildJsonObject {
        if (positionMs != null) put("position_ms", positionMs)
        if (locator != null) put("locator", locator.toJson())
        put("note", note.trim())
    }

    /**
     * Which of two places to go on from: the newer by when it was read (the last writer wins, as on the server).
     * A tie keeps [local], so a place that is waiting to be sent isn't lost.
     */
    fun newer(local: Progress?, server: Progress?): Progress? = when {
        local == null -> server
        server == null -> local
        server.updatedAt > local.updatedAt -> server
        else -> local
    }

    /** "Continued from your place on another device" when the place came from somewhere else, else null. */
    fun movedNote(chosen: Progress?, device: String): String? = chosen?.takeIf { it.device.isNotEmpty() && it.device != device && it.fraction > 0 }?.let {
        val where = when {
            it.device == "web" -> "the web app"
            it.device.startsWith("android:") -> "another phone"
            it.device.startsWith("koreader:") -> "KOReader"
            it.device.startsWith("kobo:") -> "a Kobo"
            it.device.startsWith("abs:") -> "an Audiobookshelf app"
            else -> "another device"
        }
        "Went on from where you were in $where."
    }
}
