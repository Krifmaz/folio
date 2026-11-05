package dev.krifmaz.folio

/** The reader's script and style, and how they go into each page. Plain Kotlin, so it is unit-tested. */
object ReaderAssets {
    /** The content security policy of every page: the book's own files only, and only the reader's script. */
    fun policy(nonce: String): String = "default-src 'none'; img-src https://folio.book data:; media-src https://folio.book; font-src https://folio.book data:; " +
        "style-src https://folio.book 'unsafe-inline'; script-src 'nonce-$nonce'"

    /** Puts the reader's style and script into a page's head (or at the start when it has none). */
    fun inject(page: String, nonce: String): String {
        val tags = "<link rel=\"stylesheet\" href=\"https://folio.book/__folio/reader.css\"/>" +
            "<script nonce=\"$nonce\" src=\"https://folio.book/__folio/reader.js\"></script>"
        val head = Regex("</head\\s*>", RegexOption.IGNORE_CASE).find(page)
        if (head != null) return page.substring(0, head.range.first) + tags + page.substring(head.range.first)
        val body = Regex("<body[^>]*>", RegexOption.IGNORE_CASE).find(page)
        return if (body != null) page.substring(0, body.range.last + 1) + tags + page.substring(body.range.last + 1) else tags + page
    }

    /** A file's type by its extension, as an EPUB's files go. */
    fun mimeOf(path: String): String = when (path.substringAfterLast('.', "").lowercase()) {
        "xhtml", "xht", "xml" -> "application/xhtml+xml"
        "html", "htm" -> "text/html"
        "css" -> "text/css"
        "jpg", "jpeg" -> "image/jpeg"
        "png" -> "image/png"
        "gif" -> "image/gif"
        "webp" -> "image/webp"
        "svg" -> "image/svg+xml"
        "ttf" -> "font/ttf"
        "otf" -> "font/otf"
        "woff" -> "font/woff"
        "woff2" -> "font/woff2"
        "mp3" -> "audio/mpeg"
        "mp4", "m4a" -> "audio/mp4"
        else -> "application/octet-stream"
    }

    /** The pages: the part in columns one screen wide, with margins; pictures fit the page. */
    val STYLE: String =
        """
        :root { --folio-bg: transparent; --folio-text: inherit; --folio-link: inherit; }
        html { margin: 0 !important; padding: 0 !important; height: 100vh !important; overflow: hidden !important;
          background: var(--folio-bg) !important; }
        body { margin: 0 !important; padding: 28px 24px !important; box-sizing: border-box !important; height: 100vh !important;
          column-width: calc(100vw - 48px) !important; column-gap: 48px !important; column-fill: auto !important;
          color: var(--folio-text) !important; background: var(--folio-bg) !important; overflow-wrap: break-word;
          -webkit-text-size-adjust: none; text-align: start; line-height: 1.5; }
        a { color: var(--folio-link) !important; }
        img, svg, video { max-width: 100% !important; max-height: calc(100vh - 56px) !important; object-fit: contain; break-inside: avoid; }
        pre { white-space: pre-wrap; }
        """.trimIndent()

    /** Turning pages: taps on the sides and swipes, the place in pages, the look. */
    val SCRIPT: String =
        """
        (function () {
          var R = window.FolioReader;
          function w() { return window.innerWidth; }
          function pages() { return Math.max(1, Math.round(document.documentElement.scrollWidth / w())); }
          function page() { return Math.round(window.scrollX / w()); }
          function report() { R.page(page(), pages()); }
          window.folioGo = function (p) { p = Math.max(0, Math.min(pages() - 1, p)); window.scrollTo(p * w(), 0); report(); };
          window.folioAt = function (f) { folioGo(Math.round(f * (pages() - 1))); };
          window.folioNext = function () { if (page() + 1 >= pages()) R.edge(1); else folioGo(page() + 1); };
          window.folioPrev = function () { if (page() <= 0) R.edge(-1); else folioGo(page() - 1); };
          window.folioLook = function (bg, text, link, size, target) {
            var at = (target === null || target === undefined) ? (pages() > 1 ? page() / (pages() - 1) : 0) : target;
            var s = document.documentElement.style;
            s.setProperty('--folio-bg', bg); s.setProperty('--folio-text', text); s.setProperty('--folio-link', link);
            s.fontSize = size + '%';
            setTimeout(function () { if (at < 0) folioGo(pages()); else folioAt(at); }, 50);
          };
          var x0 = null, y0 = null;
          document.addEventListener('touchstart', function (e) { x0 = e.touches[0].clientX; y0 = e.touches[0].clientY; }, { passive: true });
          document.addEventListener('touchend', function (e) {
            if (x0 === null) return;
            var dx = e.changedTouches[0].clientX - x0, dy = e.changedTouches[0].clientY - y0;
            x0 = null;
            if (Math.abs(dx) > 40 && Math.abs(dx) > Math.abs(dy)) { if (dx < 0) folioNext(); else folioPrev(); }
          }, { passive: true });
          document.addEventListener('click', function (e) {
            if (e.target.closest && e.target.closest('a')) return;
            var x = e.clientX / w();
            if (x < 0.3) folioPrev(); else if (x > 0.7) folioNext(); else R.tap();
          });
          window.addEventListener('resize', function () { setTimeout(report, 50); });
          window.addEventListener('load', function () { R.ready(pages()); });
        })();
        """.trimIndent()
}
