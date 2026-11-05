package dev.krifmaz.folio

import org.junit.Assert.assertEquals
import org.junit.Test

/** [DownloadRules]: which files a download keeps, and the button's words. */
class DownloadRulesTest {
    private fun f(id: Long, kind: String) = BookFile(id, "f$id", kind, "", 10, 0, emptyList())

    @Test
    fun keepsAudioElseTheBestEbook() {
        assertEquals(listOf(1L, 2L), DownloadRules.toKeep(listOf(f(1, "audio"), f(9, "cover"), f(2, "audio"))).map { it.id })
        assertEquals(listOf(4L), DownloadRules.toKeep(listOf(f(3, "pdf"), f(4, "epub"), f(9, "cover"))).map { it.id })
        assertEquals(listOf(3L), DownloadRules.toKeep(listOf(f(3, "pdf"), f(5, "azw3"))).map { it.id })
        assertEquals(emptyList<BookFile>(), DownloadRules.toKeep(listOf(f(9, "cover"))))
    }

    @Test
    fun buttonWords() {
        assertEquals("Download", DownloadRules.label(false, null))
        assertEquals("On this phone", DownloadRules.label(true, null))
        assertEquals("Downloading…", DownloadRules.label(false, DownloadState(1, 0, 0)))
        assertEquals("Downloading 45%", DownloadRules.label(false, DownloadState(1, 45, 100)))
        assertEquals("Try the download again", DownloadRules.label(false, DownloadState(1, 45, 100, "The download stopped: no network")))
    }
}
