package dev.krifmaz.folio

import dev.krifmaz.kit.compose.core.parseJsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** [PlaybackRules]: places across files, chapters, the sleep timer, speeds and progress. */
class PlaybackRulesTest {
    private fun file(id: Long, ms: Long, chapters: List<ChapterMark> = emptyList()) = BookFile(id, "Part $id.mp3", "audio", "audio/mpeg", 1, ms, chapters)

    private val twoFiles = listOf(file(1, 60_000), file(2, 40_000))

    @Test
    fun readsFiles() {
        val o = parseJsonObject(
            """{"files": [{"id": 4, "name": "Book.m4b", "kind": "audio", "mime": "audio/mp4", "size": 99, "duration_ms": 5000,
            "chapters": [{"title": "One", "start_ms": 0, "end_ms": 2000}, {"title": "Two", "start_ms": 2000, "end_ms": 5000}]},
            {"id": 5, "name": "cover.jpg", "kind": "cover", "mime": "image/jpeg", "size": 9, "duration_ms": 0, "chapters": []}]}""",
        )!!
        val files = PlaybackRules.filesOf(o)
        assertEquals(2, files.size)
        assertEquals(listOf(4L), PlaybackRules.audio(files).map { it.id })
        assertEquals(ChapterMark("Two", 2000, 5000), files[0].chapters[1])
    }

    @Test
    fun placesAcrossFiles() {
        assertEquals(FilePlace(0, 0), PlaybackRules.place(twoFiles, -5))
        assertEquals(FilePlace(0, 59_999), PlaybackRules.place(twoFiles, 59_999))
        assertEquals(FilePlace(1, 0), PlaybackRules.place(twoFiles, 60_000))
        assertEquals(FilePlace(1, 40_000), PlaybackRules.place(twoFiles, 500_000))
        assertEquals(70_000L, PlaybackRules.position(twoFiles, 1, 10_000))
        assertEquals(100_000L, PlaybackRules.total(twoFiles))
        assertEquals(FilePlace(0, 0), PlaybackRules.place(emptyList(), 10))
    }

    @Test
    fun chaptersAcrossFiles() {
        val withMarks = listOf(file(1, 10_000, listOf(ChapterMark("A", 0, 4000), ChapterMark("B", 4000, 0))), file(2, 5000))
        val c = PlaybackRules.bookChapters(withMarks)
        assertEquals(listOf(ChapterMark("A", 0, 4000), ChapterMark("B", 4000, 10_000), ChapterMark("Part 2", 10_000, 15_000)), c)
        // one file without chapters: the metadata's
        val meta = listOf(ChapterMark("M", 0, 5000))
        assertEquals(meta, PlaybackRules.bookChapters(listOf(file(1, 5000)), meta))
        assertEquals(1, PlaybackRules.chapterAt(c, 4000))
        assertEquals(2, PlaybackRules.chapterAt(c, 14_999))
        assertEquals(-1, PlaybackRules.chapterAt(emptyList(), 1))
    }

    @Test
    fun previousAndNextChapter() {
        val c = listOf(ChapterMark("A", 0, 10_000), ChapterMark("B", 10_000, 20_000), ChapterMark("C", 20_000, 30_000))
        // well into B: back to its start
        assertEquals(10_000L, PlaybackRules.previousChapterStart(c, 15_000))
        // just started B: to A
        assertEquals(0L, PlaybackRules.previousChapterStart(c, 11_000))
        assertEquals(20_000L, PlaybackRules.nextChapterStart(c, 15_000))
        assertNull(PlaybackRules.nextChapterStart(c, 25_000))
    }

    @Test
    fun sleepTimer() {
        val c = listOf(ChapterMark("A", 0, 60_000), ChapterMark("B", 60_000, 120_000))
        assertEquals(240_000L, PlaybackRules.sleepLeftMs(SleepTimer.After(300_000), 1000, 61_000, c, 0, 120_000, 1f))
        assertEquals(0L, PlaybackRules.sleepLeftMs(SleepTimer.After(1000), 0, 5000, c, 0, 120_000, 1f))
        // end of chapter at double speed: half the time of what is left in the chapter
        assertEquals(10_000L, PlaybackRules.sleepLeftMs(SleepTimer.EndOfChapter, 0, 0, c, 40_000, 120_000, 2f))
        assertEquals(20_000L, PlaybackRules.sleepLeftMs(SleepTimer.EndOfChapter, 0, 0, emptyList(), 100_000, 120_000, 1f))
    }

    @Test
    fun speedsAndClocks() {
        assertEquals(1.1f, PlaybackRules.nextSpeed(1f))
        assertEquals(0.75f, PlaybackRules.nextSpeed(3f))
        assertEquals("1×", PlaybackRules.speedLabel(1f))
        assertEquals("1.25×", PlaybackRules.speedLabel(1.25f))
        assertEquals("1.5×", PlaybackRules.speedLabel(1.5f))
        assertEquals("2:03", PlaybackRules.clock(123_000))
        assertEquals("1:02:03", PlaybackRules.clock(3_723_000))
        assertEquals("2 h 5 min left", PlaybackRules.left(7_500_000, 0, 1f))
        assertEquals("1 h left", PlaybackRules.left(7_200_000, 0, 2f))
        assertEquals("Less than a minute left", PlaybackRules.left(10_000, 0, 1f))
    }

    @Test
    fun progressAndLastWriterWins() {
        val p = PlaybackRules.audioProgress(3, 900_000, 1_000_000, "android:x", 50.0)
        assertEquals(0.9, p.fraction, 1e-9)
        assertFalse(p.finished)
        // the last 30 seconds count as the end
        assertTrue(PlaybackRules.audioProgress(3, 975_000, 1_000_000, "android:x", 50.0).finished)
        val back = PlaybackRules.progressOf(parseJsonObject(p.toJson().toString()))
        assertEquals(p, back)
        val server = p.copy(positionMs = 10, updatedAt = 60.0, device = "web")
        assertEquals(server, PlaybackRules.newer(p, server))
        assertEquals(p, PlaybackRules.newer(p, server.copy(updatedAt = 50.0)))
        assertEquals("Went on from where you were in the web app.", PlaybackRules.movedNote(server, "android:x"))
        assertNull(PlaybackRules.movedNote(p, "android:x"))
    }

    @Test
    fun locatorsAndBookmarks() {
        val o = parseJsonObject(
            """{"edition_id": 2, "locator": {"href": "ch1.xhtml", "progression": 0.5, "total_progression": 0.1}, "fraction": 0.1, "updated_at": 9}""",
        )!!
        assertEquals(Locator("ch1.xhtml", 0.5, 0.1), PlaybackRules.progressOf(o)!!.locator)
        val asText = parseJsonObject("""{"edition_id": 2, "locator": "{\"href\": \"a.xhtml\", \"progression\": 1, \"total_progression\": 1}"}""")!!
        assertEquals("a.xhtml", PlaybackRules.progressOf(asText)!!.locator!!.href)
        val marks = PlaybackRules.bookmarksOf(
            parseJsonObject("""{"bookmarks": [{"id": 2, "position_ms": 5, "note": "b", "created_at": 20}, {"id": 1, "position_ms": 1, "note": "a", "created_at": 10}]}""")!!,
        )
        assertEquals(listOf(1L, 2L), marks.map { it.id })
        assertEquals("""{"position_ms":5,"note":"hi"}""", PlaybackRules.bookmarkBody(5, null, " hi ").toString())
    }
}
