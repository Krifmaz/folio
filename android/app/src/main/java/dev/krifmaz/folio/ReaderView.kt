package dev.krifmaz.folio

import android.annotation.SuppressLint
import android.content.Context
import android.os.Handler
import android.os.Looper
import android.webkit.JavascriptInterface
import android.webkit.WebResourceRequest
import android.webkit.WebResourceResponse
import android.webkit.WebView
import android.webkit.WebViewClient
import java.io.ByteArrayInputStream
import java.io.File
import java.util.UUID
import java.util.zip.ZipFile

/**
 * What the reader's page tells the app: it is laid out (with how many pages the part has), the page turned, a tap
 * in the middle (show or hide the controls), or a turn past either end of the part (the next or previous part).
 */
interface ReaderEvents {
    /** The part is laid out in [pages] pages and shows [page] (from 0). */
    fun onPage(page: Int, pages: Int)

    /** A tap in the middle third of the page. */
    fun onMiddleTap()

    /** A turn past the end ([direction] 1) or the start (-1) of this part. */
    fun onEdge(direction: Int)
}

/**
 * The reader's look, put into every page: the theme's background and text colours (from the kit's palette, as
 * `#rrggbb`), and the text size in percent.
 */
data class ReaderLook(val background: String, val text: String, val link: String, val sizePercent: Int)

/**
 * A WebView that shows one part of an EPUB at a time, laid out in pages side by side (CSS columns), straight from
 * the EPUB file on the phone. The book's files are served at `https://folio.book/<path>`; nothing else is fetched,
 * the book's own scripts are blocked (a content security policy lets only the reader's script run), and the page
 * talks back through [ReaderEvents].
 */
@SuppressLint("SetJavaScriptEnabled", "ViewConstructor")
class ReaderView(context: Context, private val epub: File, private val events: ReaderEvents) : WebView(context) {
    private val zip = ZipFile(epub)
    private val nonce = UUID.randomUUID().toString().replace("-", "")
    private val main = Handler(Looper.getMainLooper())
    private var look: ReaderLook? = null

    // where to go once the part is laid out: a progression (0–1), or -1 for its last page
    private var pending = 0.0

    init {
        settings.javaScriptEnabled = true
        settings.allowFileAccess = false
        settings.allowContentAccess = false
        settings.domStorageEnabled = false
        settings.blockNetworkLoads = false
        isVerticalScrollBarEnabled = false
        isHorizontalScrollBarEnabled = false
        addJavascriptInterface(Bridge(), "FolioReader")
        webViewClient = Client()
    }

    /** Shows the part at [href] (a path from the EPUB's root), at [progression] (0–1), or its last page with -1. */
    fun open(href: String, progression: Double) {
        pending = progression
        loadUrl(ORIGIN + href.split('/').joinToString("/") { android.net.Uri.encode(it) })
    }

    /** Changes the colours and text size (the pages are laid out again, keeping the place). */
    fun setLook(l: ReaderLook) {
        look = l
        evaluateJavascript(lookCall(l, "null"), null)
    }

    // the look, then the place: [target] is a progression, -1 for the last page, or null to keep the place
    private fun lookCall(l: ReaderLook, target: String): String = "window.folioLook && folioLook(${q(l.background)}, ${q(l.text)}, ${q(l.link)}, ${l.sizePercent}, $target)"

    /** The next page (or the next part, through [ReaderEvents.onEdge]). */
    fun next() = evaluateJavascript("window.folioNext && folioNext()", null)

    /** The page before (or the part before). */
    fun previous() = evaluateJavascript("window.folioPrev && folioPrev()", null)

    /** Lets the EPUB file go. */
    fun close() {
        try {
            zip.close()
        } catch (_: Exception) {
            // already closed
        }
        destroy()
    }

    private fun q(s: String): String = "'" + s.replace("\\", "\\\\").replace("'", "\\'") + "'"

    /** Calls from the page's script (on a background thread: each goes to the main thread). */
    private inner class Bridge {
        /** The part is laid out: put the look on and go to the place asked for. */
        @JavascriptInterface
        fun ready(pages: Int) {
            main.post {
                val l = look
                val p = pending
                evaluateJavascript(
                    if (l != null) {
                        lookCall(l, "$p")
                    } else if (p < 0) {
                        "folioGo($pages)"
                    } else {
                        "folioAt($p)"
                    },
                    null,
                )
            }
        }

        /** The page shown changed. */
        @JavascriptInterface
        fun page(page: Int, pages: Int) {
            main.post { events.onPage(page, pages) }
        }

        /** A tap in the middle. */
        @JavascriptInterface
        fun tap() {
            main.post { events.onMiddleTap() }
        }

        /** A turn past an end of the part. */
        @JavascriptInterface
        fun edge(direction: Int) {
            main.post { events.onEdge(direction) }
        }
    }

    /** Serves the book's files from the EPUB and the reader's own script and style; refuses everything else. */
    private inner class Client : WebViewClient() {
        override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean {
            val url = request.url.toString()
            // a link inside the book opens that part; links out of the book do nothing
            if (url.startsWith(ORIGIN)) {
                val path = android.net.Uri.decode(url.removePrefix(ORIGIN).substringBefore('#').substringBefore('?'))
                open(path, 0.0)
            }
            return true
        }

        override fun shouldInterceptRequest(view: WebView, request: WebResourceRequest): WebResourceResponse {
            val url = request.url.toString()
            if (!url.startsWith(ORIGIN)) return refuse()
            val path = android.net.Uri.decode(url.removePrefix(ORIGIN).substringBefore('#').substringBefore('?'))
            return when (path) {
                "__folio/reader.js" -> respond("text/javascript", ReaderAssets.SCRIPT.toByteArray())
                "__folio/reader.css" -> respond("text/css", ReaderAssets.STYLE.toByteArray())
                else -> {
                    val entry = zip.getEntry(path) ?: return refuse(404)
                    val bytes = zip.getInputStream(entry).use { it.readBytes() }
                    val type = ReaderAssets.mimeOf(path)
                    if (type == "application/xhtml+xml" ||
                        type == "text/html"
                    ) {
                        respond(type, ReaderAssets.inject(String(bytes, Charsets.UTF_8), nonce).toByteArray())
                    } else {
                        respond(type, bytes)
                    }
                }
            }
        }

        private fun respond(type: String, bytes: ByteArray): WebResourceResponse = WebResourceResponse(
            type,
            "utf-8",
            200,
            "OK",
            mapOf("Content-Security-Policy" to ReaderAssets.policy(nonce), "Cache-Control" to "no-store"),
            ByteArrayInputStream(bytes),
        )

        private fun refuse(code: Int = 403): WebResourceResponse = WebResourceResponse(
            "text/plain",
            "utf-8",
            code,
            if (code ==
                404
            ) {
                "Not Found"
            } else {
                "Forbidden"
            },
            emptyMap(),
            ByteArrayInputStream(ByteArray(0)),
        )
    }

    private companion object {
        const val ORIGIN = "https://folio.book/"
    }
}
