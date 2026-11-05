package dev.krifmaz.folio

import android.content.Context
import android.net.Uri
import dev.krifmaz.kit.compose.core.parseJsonObject
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import java.io.File
import java.io.IOException

/** A book kept on the phone: the edition, its files there, and how much room they take. */
data class KeptBook(
    /** the edition */
    val edition: Edition,
    /** its files as the server listed them */
    val files: List<BookFile>,
    /** the room the files take on the phone, in bytes */
    val bytes: Long,
)

/**
 * Books kept on the phone to listen to or read without the server: each edition in `files/downloads/<id>/`, with the
 * server's answers about it (`edition.json`, `files.json`) so its pages open offline, and its files. A book counts as
 * kept once every file has come whole (`complete`). Downloads run in the app's process, one book at a time; one
 * that was stopped (the app closed, the network went) starts again where it can with [resume].
 */
class Downloads(context: Context, private val server: ServerApi) {
    private val root = File(context.applicationContext.filesDir, "downloads")
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO.limitedParallelism(1))
    private val activeFlow = MutableStateFlow<Map<Long, DownloadState>>(emptyMap())
    private val keptFlow = MutableStateFlow<Set<Long>>(emptySet())
    private val jobs = java.util.concurrent.ConcurrentHashMap<Long, Job>()

    // removing doesn't wait behind a download on its way
    private val side = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    /** The downloads on their way (and those that stopped with an error), by edition. */
    val active: StateFlow<Map<Long, DownloadState>> = activeFlow.asStateFlow()

    /** The editions kept on the phone. */
    val kept: StateFlow<Set<Long>> = keptFlow.asStateFlow()

    init {
        scope.launch { keptFlow.value = readKept() }
    }

    /** Whether [editionId] is kept on the phone, whole. */
    fun isKept(editionId: Long): Boolean = File(dir(editionId), COMPLETE).isFile

    /** The phone's copy of [f] of [editionId], when the book is kept. */
    fun local(editionId: Long, f: BookFile): File? = file(editionId, f).takeIf { isKept(editionId) && it.isFile }

    /** Where the player gets [f] from: the phone's copy when kept, else the server's stream. */
    fun uriOf(editionId: Long, f: BookFile): Uri = local(editionId, f)?.let(Uri::fromFile) ?: Uri.parse(server.url("/api/files/${f.id}"))

    /** The edition and its files as they were when the book was kept (for its pages while offline); null when not kept. */
    fun offline(editionId: Long): Pair<Edition, List<BookFile>>? {
        if (!isKept(editionId)) return null
        val e = parseJsonObject(File(dir(editionId), EDITION).readTextOrNull()) ?: return null
        val f = parseJsonObject(File(dir(editionId), FILES).readTextOrNull()) ?: return null
        return LibraryRules.editionOf(e) to PlaybackRules.filesOf(f)
    }

    /** Every kept book, by title. Blocks (reads the phone's storage); run it on Dispatchers.IO. */
    fun list(): List<KeptBook> = readKept().mapNotNull { id ->
        val (e, files) = offline(id) ?: return@mapNotNull null
        KeptBook(e, files, dir(id).walkTopDown().filter { it.isFile }.sumOf { it.length() })
    }.sortedBy { it.edition.title.lowercase() }

    /** Starts keeping [editionId] on the phone (nothing when it is already kept or on its way). */
    fun start(editionId: Long) {
        if (isKept(editionId) || jobs[editionId]?.isActive == true) return
        activeFlow.update { it + (editionId to DownloadState(editionId, 0, 0)) }
        jobs[editionId] = scope.launch { run(editionId) }
    }

    /** Starts again every download that didn't finish (a folder without `complete`). */
    fun resume() {
        scope.launch {
            root.listFiles()?.mapNotNull { it.name.toLongOrNull() }?.filter { !isKept(it) }?.forEach { start(it) }
        }
    }

    /** Stops a download on its way and forgets it, or removes a kept book from the phone. */
    fun remove(editionId: Long) {
        jobs.remove(editionId)?.cancel()
        side.launch {
            dir(editionId).deleteRecursively()
            activeFlow.update { it - editionId }
            keptFlow.update { it - editionId }
        }
    }

    private suspend fun run(editionId: Long) {
        val d = dir(editionId)
        val job = kotlin.coroutines.coroutineContext[Job]
        try {
            d.mkdirs()
            val editionJson = server.getObject("/api/library/editions/$editionId")
            val filesJson = server.getObject("/api/library/editions/$editionId/files")
            val wanted = DownloadRules.toKeep(PlaybackRules.filesOf(filesJson))
            if (wanted.isEmpty()) throw IOException("This book has no files on the server yet, so there is nothing to download.")
            File(d, EDITION).writeText(editionJson.toString())
            File(d, FILES).writeText(filesJson.toString())
            val total = wanted.sumOf { it.size }
            var before = 0L
            for (f in wanted) {
                val dest = file(editionId, f)
                if (!(dest.isFile && f.size > 0 && dest.length() == f.size)) {
                    BookFiles.fetch(server.http, server.url("/api/files/${f.id}"), dest) { n, _ ->
                        // Remove stops the download between two pieces
                        if (job?.isActive == false) throw CancellationException("Removed")
                        activeFlow.update { it + (editionId to DownloadState(editionId, before + n, total)) }
                    }
                }
                before += f.size
                activeFlow.update { it + (editionId to DownloadState(editionId, before, total)) }
            }
            File(d, COMPLETE).writeText("")
            activeFlow.update { it - editionId }
            keptFlow.update { it + editionId }
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            val why = errorWords("The download stopped", e)
            activeFlow.update { m -> m + (editionId to (m[editionId] ?: DownloadState(editionId, 0, 0)).copy(error = why)) }
        } finally {
            job?.let { jobs.remove(editionId, it) }
        }
    }

    private fun readKept(): Set<Long> = root.listFiles()?.mapNotNull { it.name.toLongOrNull() }?.filter(::isKept)?.toSet().orEmpty()

    private fun dir(editionId: Long): File = File(root, editionId.toString())

    private fun file(editionId: Long, f: BookFile): File = File(dir(editionId), "${f.id}.${BookFiles.extension(f)}")

    private fun File.readTextOrNull(): String? = try {
        readText()
    } catch (_: IOException) {
        null
    }

    private companion object {
        const val EDITION = "edition.json"
        const val FILES = "files.json"
        const val COMPLETE = "complete"
    }
}
