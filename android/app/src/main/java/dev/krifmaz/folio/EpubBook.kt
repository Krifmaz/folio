package dev.krifmaz.folio

import org.w3c.dom.Element
import org.w3c.dom.Node
import java.io.ByteArrayInputStream
import java.io.File
import java.util.zip.ZipFile
import javax.xml.parsers.DocumentBuilderFactory

/** One part of the book in reading order (an OPF spine item): its path in the EPUB and its size, for progress. */
data class SpineItem(
    /** the path inside the EPUB, from its root (`OEBPS/ch03.xhtml`) */
    val href: String,
    /** its size in bytes: how much of the book it is */
    val size: Long,
)

/** A line of the table of contents: its title, the spine item it opens and how deep it sits (0 at the top). */
data class TocEntry(
    /** the title */
    val title: String,
    /** the spine item's path from the EPUB's root (without any `#fragment`) */
    val href: String,
    /** 0 for a top line, 1 under it… */
    val depth: Int,
)

/**
 * An EPUB's reading order, table of contents and metadata, read from its container, OPF and nav (EPUB 3) or NCX
 * (EPUB 2). Paths are from the EPUB's root. Plain JVM code (javax.xml), so it is unit-tested.
 */
data class EpubBook(
    /** the title the OPF gives */
    val title: String,
    /** the language the OPF gives ("" when none) */
    val language: String,
    /** the parts in reading order (only those meant to be read in order) */
    val spine: List<SpineItem>,
    /** the table of contents ("" titles left out) */
    val toc: List<TocEntry>,
) {
    /** All the parts' sizes together (at least 1). */
    val totalSize: Long get() = spine.sumOf { it.size }.coerceAtLeast(1)

    /** The index of the spine item [href] names (an exact path, else one that ends with it), or -1. */
    fun indexOf(href: String): Int {
        val h = href.substringBefore('#').trimStart('/')
        if (h.isEmpty()) return -1
        val exact = spine.indexOfFirst { it.href == h }
        return if (exact >= 0) exact else spine.indexOfFirst { it.href.endsWith("/$h") || h.endsWith("/${it.href}") }
    }

    /** How far through the whole book [progression] (0–1) through item [index] is, weighed by the items' sizes. */
    fun total(index: Int, progression: Double): Double {
        if (spine.isEmpty()) return 0.0
        val i = index.coerceIn(0, spine.lastIndex)
        val before = spine.take(i).sumOf { it.size }
        return ((before + progression.coerceIn(0.0, 1.0) * spine[i].size) / totalSize).coerceIn(0.0, 1.0)
    }

    /** The spine item and the progression in it for a place [total] (0–1) through the whole book. */
    fun at(total: Double): Pair<Int, Double> {
        if (spine.isEmpty()) return 0 to 0.0
        var left = total.coerceIn(0.0, 1.0) * totalSize
        for ((i, s) in spine.withIndex()) {
            if (left < s.size || i == spine.lastIndex) return i to (if (s.size <= 0) 0.0 else (left / s.size).coerceIn(0.0, 1.0))
            left -= s.size
        }
        return spine.lastIndex to 1.0
    }

    /** The locator for [progression] through item [index]. */
    fun locator(index: Int, progression: Double): Locator {
        val i = index.coerceIn(0, (spine.size - 1).coerceAtLeast(0))
        return Locator(spine.getOrNull(i)?.href.orEmpty(), progression.coerceIn(0.0, 1.0), total(i, progression))
    }

    /** Where a [locator] (from this phone or another device) opens: its spine item and progression. */
    fun place(locator: Locator?, fraction: Double): Pair<Int, Double> {
        if (locator != null) {
            val i = indexOf(locator.href)
            if (i >= 0) return i to locator.progression.coerceIn(0.0, 1.0)
            if (locator.totalProgression > 0) return at(locator.totalProgression)
        }
        return if (fraction > 0) at(fraction) else 0 to 0.0
    }

    /** The table of contents' title for spine item [index]: the last entry at or before it ("" with none). */
    fun chapterTitle(index: Int): String {
        var best = ""
        for (t in toc) {
            val i = indexOf(t.href)
            if (i in 0..index) best = t.title
        }
        return best
    }

    /** Reading an EPUB. */
    companion object {
        /** Why an EPUB can't be opened, in plain words. */
        const val BROKEN = "This file isn't an EPUB that can be opened: its table of contents (the OPF) is missing or broken."

        /** Reads the EPUB [file] (a ZIP). Throws an [IllegalArgumentException] with [BROKEN] when it can't. */
        fun open(file: File): EpubBook = ZipFile(file).use { zip ->
            read(name = { n -> zip.getEntry(n)?.let { e -> zip.getInputStream(e).use { it.readBytes() } } }, size = { n -> zip.getEntry(n)?.size ?: 0L })
        }

        /**
         * Reads an EPUB through [name] (an entry's bytes, null when there is none) and [size] (an entry's size), so
         * it works on a ZIP or, in tests, a map.
         */
        fun read(name: (String) -> ByteArray?, size: (String) -> Long): EpubBook {
            val container = xml(name("META-INF/container.xml") ?: throw IllegalArgumentException(BROKEN))
            val opfPath = container.all("rootfile").firstOrNull()?.getAttribute("full-path")?.takeIf { it.isNotBlank() } ?: throw IllegalArgumentException(BROKEN)
            val opf = xml(name(opfPath) ?: throw IllegalArgumentException(BROKEN))
            val base = opfPath.substringBeforeLast('/', "")
            val manifest = opf.all("item").associate { it.getAttribute("id") to (resolve(base, it.getAttribute("href")) to it) }
            val spine = opf.all("itemref")
                .filter { it.getAttribute("linear") != "no" }
                .mapNotNull { manifest[it.getAttribute("idref")]?.first }
                .map { SpineItem(it, size(it).coerceAtLeast(1)) }
            if (spine.isEmpty()) throw IllegalArgumentException(BROKEN)
            val title = opf.all("title").firstOrNull()?.textContent?.trim().orEmpty()
            val language = opf.all("language").firstOrNull()?.textContent?.trim().orEmpty()
            // EPUB 3: the item with the nav property; EPUB 2: the spine's toc (the NCX)
            val nav = manifest.values.firstOrNull { (_, e) -> e.getAttribute("properties").split(' ').contains("nav") }?.first
            val ncx = opf.all("spine").firstOrNull()?.getAttribute("toc")?.let { manifest[it]?.first }
                ?: manifest.values.firstOrNull { (_, e) -> e.getAttribute("media-type") == "application/x-dtbncx+xml" }?.first
            // a table of contents that can't be read (an HTML entity XML doesn't know, say) is left out, not fatal
            val toc = runCatching { nav?.let { p -> name(p)?.let { tocFromNav(xml(it), p.substringBeforeLast('/', "")) } } }.getOrNull()?.takeIf { it.isNotEmpty() }
                ?: runCatching { ncx?.let { p -> name(p)?.let { tocFromNcx(xml(it), p.substringBeforeLast('/', "")) } } }.getOrNull()
                ?: emptyList()
            return EpubBook(title, language, spine, toc)
        }

        /** A path relative to [base] (a folder inside the EPUB), resolved: `OEBPS` + `../Images/a.jpg` → `Images/a.jpg`. */
        fun resolve(base: String, href: String): String {
            val clean = href.substringBefore('#').substringBefore('?')
            val decoded = try {
                java.net.URLDecoder.decode(clean.replace("+", "%2B"), "UTF-8")
            } catch (_: IllegalArgumentException) {
                clean
            }
            val joined = if (base.isEmpty() || decoded.startsWith("/")) decoded.trimStart('/') else "$base/$decoded"
            val out = ArrayDeque<String>()
            for (part in joined.split('/')) {
                when (part) {
                    "", "." -> {}
                    ".." -> out.removeLastOrNull()
                    else -> out.addLast(part)
                }
            }
            return out.joinToString("/")
        }

        private fun tocFromNav(doc: Element, base: String): List<TocEntry> {
            val navs = doc.all("nav")
            val toc =
                navs.firstOrNull { it.getAttribute("epub:type") == "toc" || it.getAttributeNS("http://www.idpf.org/2007/ops", "type") == "toc" } ?: navs.firstOrNull()
                    ?: return emptyList()
            val out = mutableListOf<TocEntry>()
            fun walk(list: Element, depth: Int) {
                for (li in list.children("li")) {
                    val a = li.children("a").firstOrNull() ?: li.children("span").firstOrNull()
                    val href = a?.getAttribute("href").orEmpty()
                    val title = a?.textContent?.trim()?.replace(Regex("\\s+"), " ").orEmpty()
                    if (title.isNotEmpty() && href.isNotEmpty()) out += TocEntry(title, resolve(base, href), depth)
                    li.children("ol").firstOrNull()?.let { walk(it, depth + 1) }
                }
            }
            toc.children("ol").firstOrNull()?.let { walk(it, 0) }
            return out
        }

        private fun tocFromNcx(doc: Element, base: String): List<TocEntry> {
            val out = mutableListOf<TocEntry>()
            fun walk(parent: Element, depth: Int) {
                for (p in parent.children("navPoint")) {
                    val title = p.children("navLabel").firstOrNull()?.textContent?.trim()?.replace(Regex("\\s+"), " ").orEmpty()
                    val src = p.children("content").firstOrNull()?.getAttribute("src").orEmpty()
                    if (title.isNotEmpty() && src.isNotEmpty()) out += TocEntry(title, resolve(base, src), depth)
                    walk(p, depth + 1)
                }
            }
            doc.all("navMap").firstOrNull()?.let { walk(it, 0) }
            return out
        }

        private fun xml(bytes: ByteArray): Element = try {
            parse(bytes)
        } catch (e: Exception) {
            throw IllegalArgumentException(BROKEN, e)
        }

        private fun parse(bytes: ByteArray): Element {
            val f = DocumentBuilderFactory.newInstance()
            f.isNamespaceAware = true
            f.isExpandEntityReferences = false
            val b = f.newDocumentBuilder()
            // a book never reaches outside itself: a DTD it names (XHTML's) is read as empty
            b.setEntityResolver { _, _ -> org.xml.sax.InputSource(ByteArrayInputStream(ByteArray(0))) }
            return b.parse(ByteArrayInputStream(bytes)).documentElement
        }

        /** Every element under this one with the local name [local], in document order. */
        private fun Element.all(local: String): List<Element> {
            val nodes = getElementsByTagNameNS("*", local)
            return (0 until nodes.length).map { nodes.item(it) as Element }
        }

        /** The direct children with the local name [local]. */
        private fun Element.children(local: String): List<Element> {
            val out = mutableListOf<Element>()
            var n: Node? = firstChild
            while (n != null) {
                if (n is Element && (n.localName ?: n.nodeName.substringAfter(':')) == local) out += n
                n = n.nextSibling
            }
            return out
        }
    }
}
