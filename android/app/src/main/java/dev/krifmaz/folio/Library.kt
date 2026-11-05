package dev.krifmaz.folio

import dev.krifmaz.kit.compose.core.arr
import dev.krifmaz.kit.compose.core.bool
import dev.krifmaz.kit.compose.core.dbl
import dev.krifmaz.kit.compose.core.long
import dev.krifmaz.kit.compose.core.objectsOf
import dev.krifmaz.kit.compose.core.str
import dev.krifmaz.kit.compose.core.strings
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject

/** A person named on a book: an author (`{id?, name}`). */
data class Person(
    /** the server's id of the author (0 when the source gave only a name) */
    val id: Long,
    /** the name as the source writes it */
    val name: String,
)

/** A series a book is part of, and its place in it ("3", "1.5", "" when the source doesn't say). */
data class SeriesPlace(
    /** the server's id of the series (0 when not kept) */
    val id: Long,
    /** the series' name */
    val name: String,
    /** the book's place in the series, as written */
    val position: String,
)

/** A work in a list (`GET /api/library/works`): what the library page shows for one book. */
data class WorkSummary(
    /** the work's id */
    val id: Long,
    /** the title */
    val title: String,
    /** the authors, in order */
    val authors: List<Person>,
    /** the series it is in */
    val series: List<SeriesPlace>,
    /** a cover picture's address ("" when there is none) */
    val coverUrl: String,
    /** the year it came out (null when unknown) */
    val year: Int?,
    /** the formats it has editions in: "audiobook", "ebook" */
    val formats: List<String>,
)

/** One audiobook chapter: its title and where it starts and ends, in milliseconds from the start. */
data class ChapterMark(
    /** the chapter's title */
    val title: String,
    /** where it starts */
    val startMs: Long,
    /** where it ends (0 when the source gives only starts) */
    val endMs: Long,
)

/** One exact edition (`GET /api/library/editions/{id}`, or one of a work's editions). */
data class Edition(
    /** the edition's id */
    val id: Long,
    /** the work it belongs to */
    val workId: Long,
    /** "audiobook" or "ebook" */
    val format: String,
    /** the title without its subtitle */
    val title: String,
    /** the subtitle ("" when none) */
    val subtitle: String,
    /** the authors */
    val authors: List<Person>,
    /** the narrators (audiobooks) */
    val narrators: List<String>,
    /** the series and places */
    val series: List<SeriesPlace>,
    /** the language as a two-letter code ("" when unknown) */
    val language: String,
    /** how long the audiobook runs, in minutes (null when unknown or an ebook) */
    val runtimeMin: Int?,
    /** whether the audiobook is abridged (null when the source doesn't say) */
    val abridged: Boolean?,
    /** how many pages (null when unknown) */
    val pages: Int?,
    /** the Audible ASIN ("" for ebooks) */
    val asin: String,
    /** the ISBN-13 ("" when none) */
    val isbn13: String,
    /** the publisher */
    val publisher: String,
    /** when it came out, "YYYY-MM-DD" or the year */
    val releaseDate: String,
    /** the blurb as plain text */
    val description: String,
    /** a cover picture's address */
    val coverUrl: String,
    /** the chapters the metadata source lists (audiobooks) */
    val chapters: List<ChapterMark>,
) {
    /** Whether this is an audiobook (else an ebook). */
    val isAudiobook: Boolean get() = format == AUDIOBOOK

    private companion object {
        const val AUDIOBOOK = "audiobook"
    }
}

/** A work with all its editions (`GET /api/library/works/{id}`). */
data class Work(
    /** the list part: title, authors, series, cover, year, formats */
    val summary: WorkSummary,
    /** the blurb as plain text */
    val description: String,
    /** every edition kept for it, audiobooks first */
    val editions: List<Edition>,
)

/** Reading the library's answers and putting them in plain words. Plain Kotlin, so it is unit-tested. */
object LibraryRules {
    /** The works of `GET /api/library/works` (`{works: [...]}`). */
    fun worksOf(o: JsonObject): List<WorkSummary> = o.arr("works").objectsOf().map(::summaryOf)

    /** One work of a list. */
    fun summaryOf(o: JsonObject): WorkSummary = WorkSummary(
        id = o.long("id"),
        title = o.str("title"),
        authors = peopleOf(o, "authors"),
        series = seriesOf(o),
        coverUrl = o.str("cover_url"),
        year = o["year"]?.takeIf { it != JsonNull }?.let { o.dbl("year").toInt() }?.takeIf { it > 0 },
        formats = o.arr("formats").strings(),
    )

    /** The work of `GET /api/library/works/{id}`, its editions with audiobooks first. */
    fun workOf(o: JsonObject): Work = Work(
        summary = summaryOf(o),
        description = plainText(o.str("description")),
        editions = o.arr("editions").objectsOf().map(::editionOf).sortedBy { if (it.isAudiobook) 0 else 1 },
    )

    /** One edition, from the server's JSON (the sources' fields beside `id` and `work_id`). */
    fun editionOf(o: JsonObject): Edition = Edition(
        id = o.long("id"),
        workId = o.long("work_id"),
        format = o.str("format"),
        title = o.str("title"),
        subtitle = o.str("subtitle"),
        authors = peopleOf(o, "authors"),
        narrators = o.arr("narrators").let { a -> a.strings().ifEmpty { a.objectsOf().map { it.str("name") } } }.filter { it.isNotBlank() },
        series = seriesOf(o),
        language = o.str("language"),
        runtimeMin = whole(o, "runtime_min"),
        abridged = o["abridged"]?.takeIf { it != JsonNull }?.let { o.bool("abridged") },
        pages = whole(o, "pages"),
        asin = o.str("asin"),
        isbn13 = o.str("isbn13"),
        publisher = o.str("publisher"),
        releaseDate = o.str("release_date"),
        description = plainText(o.str("description")),
        coverUrl = o.str("cover_url"),
        chapters = chaptersOf(o),
    )

    /** "Andy Weir", "Terry Pratchett and Neil Gaiman", "A, B and C"; "Unknown author" when none. */
    fun names(people: List<String>): String {
        val n = people.filter { it.isNotBlank() }
        return when (n.size) {
            0 -> "Unknown author"
            1 -> n[0]
            else -> n.dropLast(1).joinToString(", ") + " and " + n.last()
        }
    }

    /** "Discworld, book 3" or just "Discworld" when the place isn't known; "" with no series. */
    fun seriesLine(series: List<SeriesPlace>): String {
        val s = series.firstOrNull() ?: return ""
        return if (s.position.isBlank()) s.name else "${s.name}, book ${s.position}"
    }

    /** "10 h 53 min", "45 min"; "" when unknown. */
    fun runtime(minutes: Int?): String {
        if (minutes == null || minutes <= 0) return ""
        val h = minutes / 60
        val m = minutes % 60
        return when {
            h == 0 -> "$m min"
            m == 0 -> "$h h"
            else -> "$h h $m min"
        }
    }

    /** The language's name in English for the codes books come in most ("en" → "English"); the code otherwise. */
    fun language(code: String): String = LANGUAGES[code.lowercase()] ?: code.uppercase()

    /**
     * The facts of an edition on one line, as the book page lists them: format, narrator, runtime, abridged or not,
     * language, pages. For an audiobook: "Audiobook · Read by R. C. Bray · 10 h 53 min · Unabridged · English".
     */
    fun factsLine(e: Edition): String {
        val parts = mutableListOf(if (e.isAudiobook) "Audiobook" else "Ebook")
        if (e.isAudiobook) {
            if (e.narrators.isNotEmpty()) parts += "Read by " + names(e.narrators)
            runtime(e.runtimeMin).takeIf { it.isNotEmpty() }?.let { parts += it }
            when (e.abridged) {
                true -> parts += "Abridged"
                false -> parts += "Unabridged"
                null -> {}
            }
        } else {
            e.pages?.let { parts += "$it pages" }
        }
        if (e.language.isNotBlank()) parts += language(e.language)
        return parts.joinToString(" · ")
    }

    /** The formats a work has, in words: "Audiobook and ebook", "Audiobook", "Ebook". */
    fun formatsLine(formats: List<String>): String {
        val a = "audiobook" in formats
        val e = "ebook" in formats
        return when {
            a && e -> "Audiobook and ebook"
            a -> "Audiobook"
            e -> "Ebook"
            else -> ""
        }
    }

    /** The works list's query: `?q=…&format=…`, each part only when given, escaped for a URL. */
    fun worksQuery(q: String, format: String): String {
        val parts = buildList {
            if (q.isNotBlank()) add("q=" + java.net.URLEncoder.encode(q.trim(), "UTF-8"))
            if (format.isNotBlank()) add("format=$format")
        }
        return if (parts.isEmpty()) "" else "?" + parts.joinToString("&")
    }

    /** A blurb that may be simple HTML, as plain text: tags out, paragraphs as blank lines, entities read. */
    fun plainText(s: String): String {
        if ('<' !in s && '&' !in s) return s.trim()
        val broken = s.replace(Regex("(?i)<br\\s*/?>"), "\n").replace(Regex("(?i)</p\\s*>"), "\n\n")
        val text = broken.replace(Regex("<[^>]*>"), "")
            .replace("&nbsp;", " ").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&amp;", "&")
        return text.lines().joinToString("\n") { it.trim() }.replace(Regex("\n{3,}"), "\n\n").trim()
    }

    private fun peopleOf(o: JsonObject, key: String): List<Person> {
        val a = o.arr(key)
        val plain = a.strings()
        if (plain.isNotEmpty()) return plain.map { Person(0, it) }
        return a.objectsOf().map { Person(it.long("id"), it.str("name")) }.filter { it.name.isNotBlank() }
    }

    private fun seriesOf(o: JsonObject): List<SeriesPlace> = o.arr("series").objectsOf().map {
        SeriesPlace(it.long("id"), it.str("name"), it.str("position"))
    }.filter { it.name.isNotBlank() }

    private fun chaptersOf(o: JsonObject): List<ChapterMark> {
        // the metadata gives a start and a length; the files' own chapters (the player) give a start and an end
        val c = o.arr("chapters").objectsOf().map {
            val start = it.long("start_ms")
            val end = it.long("end_ms").takeIf { e -> e > 0 } ?: it.long("length_ms").takeIf { l -> l > 0 }?.let { l -> start + l } ?: 0L
            ChapterMark(it.str("title"), start, end)
        }
        // a chapter without an end ends where the next one starts
        return c.mapIndexed { i, ch -> if (ch.endMs > 0 || i == c.lastIndex) ch else ch.copy(endMs = c[i + 1].startMs) }
    }

    private fun whole(o: JsonObject, key: String): Int? = o[key]?.takeIf { it != JsonNull }?.let { o.dbl(key).toInt() }?.takeIf { it > 0 }

    private val LANGUAGES = mapOf(
        "en" to "English",
        "de" to "German",
        "fr" to "French",
        "es" to "Spanish",
        "it" to "Italian",
        "nl" to "Dutch",
        "pt" to "Portuguese",
        "sv" to "Swedish",
        "da" to "Danish",
        "no" to "Norwegian",
        "nb" to "Norwegian",
        "fi" to "Finnish",
        "pl" to "Polish",
        "ru" to "Russian",
        "ja" to "Japanese",
        "zh" to "Chinese",
        "ko" to "Korean",
    )
}
