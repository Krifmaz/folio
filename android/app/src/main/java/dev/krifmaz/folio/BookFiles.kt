package dev.krifmaz.folio

import dev.krifmaz.kit.compose.core.ApiException
import dev.krifmaz.kit.compose.core.parseJsonObject
import dev.krifmaz.kit.compose.core.str
import okhttp3.OkHttpClient
import okhttp3.Request
import java.io.File
import java.io.IOException

/** Getting a book's files onto the phone. */
object BookFiles {
    /**
     * The file [f] of [editionId] on the phone: the kept copy when the book was downloaded, else the app's cache when
     * it is there whole, else fetched from the server into the cache first. Blocks; run it on Dispatchers.IO.
     */
    fun onPhone(app: App, editionId: Long, f: BookFile): File {
        app.downloads.local(editionId, f)?.let { return it }
        val dest = File(File(app.cacheDir, "books"), "${f.id}.${extension(f)}")
        if (dest.isFile && (f.size <= 0 || dest.length() == f.size)) return dest
        fetch(app.server.http, app.server.url("/api/files/${f.id}"), dest) { _, _ -> }
        return dest
    }

    /**
     * Fetches [url] into [dest]: into `<dest>.part` first, then renamed, so a half-fetched file is never taken for
     * the book. [progress] hears the bytes so far and the whole size (-1 when the server doesn't say). Throws
     * [ApiException] with the server's words, or [IOException] when it can't be reached or the phone is full.
     */
    fun fetch(http: OkHttpClient, url: String, dest: File, progress: (Long, Long) -> Unit) {
        if (url.isEmpty()) throw IOException("Not connected to a server yet")
        dest.parentFile?.mkdirs()
        val part = File(dest.parentFile, dest.name + ".part")
        http.newCall(Request.Builder().url(url).build()).execute().use { res ->
            if (!res.isSuccessful) {
                val o = parseJsonObject(res.body.string())
                throw ApiException(res.code, o?.str("detail")?.ifEmpty { null } ?: "HTTP ${res.code}", o)
            }
            val total = res.body.contentLength()
            var done = 0L
            res.body.byteStream().use { input ->
                part.outputStream().use { out ->
                    val buf = ByteArray(64 * 1024)
                    var lastTold = 0L
                    while (true) {
                        val n = input.read(buf)
                        if (n < 0) break
                        out.write(buf, 0, n)
                        done += n
                        if (done - lastTold >= 256 * 1024) {
                            progress(done, total)
                            lastTold = done
                        }
                    }
                }
            }
            if (total >= 0 && done != total) {
                part.delete()
                throw IOException("The download stopped before the end. Try again.")
            }
            progress(done, total)
        }
        if (dest.exists()) dest.delete()
        if (!part.renameTo(dest)) throw IOException("The file couldn't be kept on the phone.")
    }

    /**
     * An edition and its files: from the server, or, when it can't be reached, as they were when the book was
     * downloaded. Blocks; run it on Dispatchers.IO.
     */
    fun editionWithFiles(app: App, editionId: Long): Pair<Edition, List<BookFile>> = try {
        LibraryRules.editionOf(app.server.getObject("/api/library/editions/$editionId")) to
            PlaybackRules.filesOf(app.server.getObject("/api/library/editions/$editionId/files"))
    } catch (e: IOException) {
        app.downloads.offline(editionId) ?: throw e
    }

    /** The file name's extension, or one from its kind. */
    fun extension(f: BookFile): String = f.name.substringAfterLast('.', "").lowercase().takeIf { it.isNotEmpty() && it.length <= 5 }
        ?: when (f.kind) {
            "epub" -> "epub"
            "pdf" -> "pdf"
            "azw3" -> "azw3"
            else -> "bin"
        }
}
