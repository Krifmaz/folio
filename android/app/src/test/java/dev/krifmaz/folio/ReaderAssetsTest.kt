package dev.krifmaz.folio

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** [ReaderAssets]: the reader's script goes into every page, and the page may run nothing else. */
class ReaderAssetsTest {
    private val tags = "<link rel=\"stylesheet\" href=\"https://folio.book/__folio/reader.css\"/><script nonce=\"n1\" src=\"https://folio.book/__folio/reader.js\"></script>"

    @Test
    fun injectsIntoTheHead() {
        assertEquals("<html><head><title>T</title>$tags</head><body/></html>", ReaderAssets.inject("<html><head><title>T</title></head><body/></html>", "n1"))
        assertEquals("<html><body class=\"x\">${tags}Hi</body></html>", ReaderAssets.inject("<html><body class=\"x\">Hi</body></html>", "n1"))
        assertEquals(tags + "plain", ReaderAssets.inject("plain", "n1"))
    }

    @Test
    fun policyAllowsOnlyTheReadersScript() {
        val p = ReaderAssets.policy("n1")
        assertTrue(p.startsWith("default-src 'none'"))
        assertTrue(p.endsWith("script-src 'nonce-n1'"))
    }

    @Test
    fun types() {
        assertEquals("application/xhtml+xml", ReaderAssets.mimeOf("OEBPS/a.XHTML"))
        assertEquals("image/jpeg", ReaderAssets.mimeOf("i/c.jpeg"))
        assertEquals("application/octet-stream", ReaderAssets.mimeOf("noext"))
    }
}
