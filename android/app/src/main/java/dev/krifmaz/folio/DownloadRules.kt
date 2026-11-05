package dev.krifmaz.folio

/** A download on its way: how much of the book has come, and why it stopped when it did. */
data class DownloadState(
    /** the edition being downloaded */
    val editionId: Long,
    /** bytes so far, all files together */
    val done: Long,
    /** all the files' bytes together (0 while not known) */
    val total: Long,
    /** why it stopped, in plain words ("" while it goes on) */
    val error: String = "",
) {
    /** How far, in whole percent (0 while the size isn't known). */
    val percent: Int get() = if (total <= 0) 0 else (done * 100 / total).toInt().coerceIn(0, 100)
}

/** Which files a download keeps, and its words. Plain Kotlin, so it is unit-tested. */
object DownloadRules {
    /** The ebook formats, best first: the reader opens EPUB; the others are kept only when there is no EPUB. */
    private val EBOOK_ORDER = listOf("epub", "pdf", "azw3")

    /** The files worth keeping: every audio file, else the best ebook file. Covers stay on the server. */
    fun toKeep(files: List<BookFile>): List<BookFile> {
        val audio = files.filter { it.kind == "audio" }
        if (audio.isNotEmpty()) return audio
        for (k in EBOOK_ORDER) files.firstOrNull { it.kind == k }?.let { return listOf(it) }
        return emptyList()
    }

    /** The button's words for a book: "Download", "Downloading 45%", "Try the download again", "On this phone". */
    fun label(kept: Boolean, state: DownloadState?): String = when {
        kept -> "On this phone"
        state == null -> "Download"
        state.error.isNotEmpty() -> "Try the download again"
        state.total <= 0 -> "Downloading…"
        else -> "Downloading ${state.percent}%"
    }
}
