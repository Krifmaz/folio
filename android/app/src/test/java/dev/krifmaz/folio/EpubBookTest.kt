package dev.krifmaz.folio

import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Test

/** [EpubBook]: reading order, table of contents and places, from EPUB 3 and EPUB 2 books kept in a map. */
class EpubBookTest {
    private val container =
        """<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
        <rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"""

    private val opf3 =
        """<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0">
        <metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>The Book</dc:title><dc:language>en</dc:language></metadata>
        <manifest>
          <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
          <item id="c1" href="Text/ch1.xhtml" media-type="application/xhtml+xml"/>
          <item id="c2" href="Text/ch%202.xhtml" media-type="application/xhtml+xml"/>
          <item id="cover" href="cover.xhtml" media-type="application/xhtml+xml"/>
        </manifest>
        <spine><itemref idref="cover" linear="no"/><itemref idref="c1"/><itemref idref="c2"/></spine></package>"""

    private val nav =
        """<?xml version="1.0"?><html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><body>
        <nav epub:type="toc"><ol><li><a href="Text/ch1.xhtml">One</a><ol><li><a href="Text/ch1.xhtml#s2">One, part 2</a></li></ol></li>
        <li><a href="Text/ch%202.xhtml">  Two
        </a></li></ol></nav></body></html>"""

    private fun book(files: Map<String, String>, sizes: Map<String, Long> = emptyMap()) =
        EpubBook.read(name = { files[it]?.toByteArray() }, size = { sizes[it] ?: (files[it]?.length?.toLong() ?: 0L) })

    @Test
    fun readsEpub3() {
        val b = book(
            mapOf("META-INF/container.xml" to container, "OEBPS/content.opf" to opf3, "OEBPS/nav.xhtml" to nav),
            mapOf("OEBPS/Text/ch1.xhtml" to 300, "OEBPS/Text/ch 2.xhtml" to 100),
        )
        assertEquals("The Book", b.title)
        assertEquals("en", b.language)
        assertEquals(listOf(SpineItem("OEBPS/Text/ch1.xhtml", 300), SpineItem("OEBPS/Text/ch 2.xhtml", 100)), b.spine)
        assertEquals(
            listOf(TocEntry("One", "OEBPS/Text/ch1.xhtml", 0), TocEntry("One, part 2", "OEBPS/Text/ch1.xhtml", 1), TocEntry("Two", "OEBPS/Text/ch 2.xhtml", 0)),
            b.toc,
        )
        assertEquals("Two", b.chapterTitle(1))
    }

    @Test
    fun readsEpub2Ncx() {
        val opf2 =
            """<package xmlns="http://www.idpf.org/2007/opf" version="2.0"><metadata/><manifest>
            <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/><item id="a" href="a.html" media-type="application/xhtml+xml"/>
            </manifest><spine toc="ncx"><itemref idref="a"/></spine></package>"""
        val ncx =
            """<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/"><navMap><navPoint id="p"><navLabel><text>Start</text></navLabel>
            <content src="a.html"/></navPoint></navMap></ncx>"""
        val b = book(mapOf("META-INF/container.xml" to container, "OEBPS/content.opf" to opf2, "OEBPS/toc.ncx" to ncx))
        assertEquals(listOf(TocEntry("Start", "OEBPS/a.html", 0)), b.toc)
    }

    @Test
    fun brokenBooksSaySo() {
        val e = assertThrows(IllegalArgumentException::class.java) { book(mapOf("META-INF/container.xml" to container)) }
        assertEquals(EpubBook.BROKEN, e.message)
        assertThrows(IllegalArgumentException::class.java) { book(emptyMap()) }
    }

    @Test
    fun placesWeighedBySize() {
        val b = EpubBook("T", "", listOf(SpineItem("a", 300), SpineItem("b", 100)), emptyList())
        assertEquals(0.375, b.total(0, 0.5), 1e-9)
        assertEquals(0.875, b.total(1, 0.5), 1e-9)
        assertEquals(1 to 0.5, b.at(0.875))
        assertEquals(0 to 0.5, b.at(0.375))
        assertEquals(Locator("b", 0.5, 0.875), b.locator(1, 0.5))
        // another device's locator: the same path, a path ending in it, or only how far through the book
        assertEquals(1 to 0.25, b.place(Locator("b", 0.25, 0.0), 0.0))
        assertEquals(1 to 0.5, b.place(Locator("missing", 0.0, 0.875), 0.0))
        assertEquals(0 to 0.5, b.place(null, 0.375))
        assertEquals(0 to 0.0, b.place(null, 0.0))
    }

    @Test
    fun resolvesPaths() {
        assertEquals("Images/a.jpg", EpubBook.resolve("OEBPS", "../Images/a.jpg"))
        assertEquals("OEBPS/Text/b.xhtml", EpubBook.resolve("OEBPS", "Text/b.xhtml#x"))
        assertEquals("c.xhtml", EpubBook.resolve("", "./c.xhtml"))
        assertEquals("OEBPS/a b.xhtml", EpubBook.resolve("OEBPS", "a%20b.xhtml"))
    }
}
