package dev.krifmaz.folio

import dev.krifmaz.kit.compose.core.parseJsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/** [LibraryRules]: reading the server's library answers and the words the pages show. */
class LibraryRulesTest {
    private val work =
        """
        {"id": 7, "title": "Project Hail Mary", "authors": [{"id": 3, "name": "Andy Weir"}],
         "series": [], "cover_url": "https://covers.example/phm.jpg", "year": 2021, "formats": ["audiobook", "ebook"],
         "description": "<p>A lone astronaut.</p><p>Earth&#39;s last hope &amp; more.</p>",
         "editions": [
           {"id": 11, "work_id": 7, "format": "ebook", "title": "Project Hail Mary", "isbn13": "9780593135204",
            "authors": [{"name": "Andy Weir"}], "language": "en", "pages": 496},
           {"id": 10, "work_id": 7, "format": "audiobook", "asin": "B08G9PRS1K", "title": "Project Hail Mary",
            "authors": [{"name": "Andy Weir"}], "narrators": ["Ray Porter"], "language": "en", "runtime_min": 970,
            "abridged": false, "chapters": [{"title": "Chapter 1", "start_ms": 0, "length_ms": 1000},
            {"title": "Chapter 2", "start_ms": 1000, "length_ms": 500}]}
         ]}
        """.trimIndent()

    @Test
    fun readsAWorkWithAudiobooksFirst() {
        val w = LibraryRules.workOf(parseJsonObject(work)!!)
        assertEquals("Project Hail Mary", w.summary.title)
        assertEquals(2021, w.summary.year)
        assertEquals(listOf(10L, 11L), w.editions.map { it.id })
        val a = w.editions[0]
        assertEquals(listOf("Ray Porter"), a.narrators)
        assertEquals(970, a.runtimeMin)
        assertEquals(false, a.abridged)
        assertEquals(listOf(ChapterMark("Chapter 1", 0, 1000), ChapterMark("Chapter 2", 1000, 1500)), a.chapters)
        assertNull(w.editions[1].abridged)
        assertEquals("A lone astronaut.\n\nEarth's last hope & more.", w.description)
    }

    @Test
    fun factsInPlainWords() {
        val w = LibraryRules.workOf(parseJsonObject(work)!!)
        assertEquals("Audiobook · Read by Ray Porter · 16 h 10 min · Unabridged · English", LibraryRules.factsLine(w.editions[0]))
        assertEquals("Ebook · 496 pages · English", LibraryRules.factsLine(w.editions[1]))
        assertEquals("Audiobook and ebook", LibraryRules.formatsLine(w.summary.formats))
    }

    @Test
    fun namesAndSeries() {
        assertEquals("Unknown author", LibraryRules.names(emptyList()))
        assertEquals("A and B", LibraryRules.names(listOf("A", "B")))
        assertEquals("A, B and C", LibraryRules.names(listOf("A", "B", "C")))
        assertEquals("Discworld, book 3", LibraryRules.seriesLine(listOf(SeriesPlace(1, "Discworld", "3"))))
        assertEquals("Discworld", LibraryRules.seriesLine(listOf(SeriesPlace(1, "Discworld", ""))))
        assertEquals("", LibraryRules.seriesLine(emptyList()))
    }

    @Test
    fun runtimes() {
        assertEquals("", LibraryRules.runtime(null))
        assertEquals("45 min", LibraryRules.runtime(45))
        assertEquals("2 h", LibraryRules.runtime(120))
        assertEquals("10 h 53 min", LibraryRules.runtime(653))
    }

    @Test
    fun worksQueryEscapes() {
        assertEquals("", LibraryRules.worksQuery(" ", ""))
        assertEquals("?q=hail+mary&format=audiobook", LibraryRules.worksQuery(" hail mary ", "audiobook"))
        assertEquals("?format=ebook", LibraryRules.worksQuery("", "ebook"))
    }

    @Test
    fun listOfWorks() {
        val list = LibraryRules.worksOf(parseJsonObject("""{"works": [{"id": 1, "title": "T", "authors": [], "formats": ["ebook"], "year": null}]}""")!!)
        assertEquals(1, list.size)
        assertNull(list[0].year)
        assertEquals("Ebook", LibraryRules.formatsLine(list[0].formats))
    }
}
