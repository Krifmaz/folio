package dev.krifmaz.folio

import android.content.Context
import dev.krifmaz.kit.compose.core.ApiException
import dev.krifmaz.kit.compose.core.long
import dev.krifmaz.kit.compose.core.parseJsonObject
import dev.krifmaz.kit.compose.core.str
import dev.krifmaz.kit.compose.offline.OutAction
import dev.krifmaz.kit.compose.offline.OutMerge
import dev.krifmaz.kit.compose.offline.Outbox
import dev.krifmaz.kit.compose.offline.OutboxFile
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import java.io.File
import java.io.IOException

/**
 * Keeps where the person is in every book, and their bookmarks, in step with the server, whether or not it can be
 * reached. Each place is kept on the phone at once and sent through the kit's outbox (only the latest per book
 * waits); bookmarks made or removed offline wait there too. [flush] sends what waits; the server's last-writer-wins
 * answer comes back into the phone's copy, so a place read on another device is picked up.
 *
 * Every call that talks to the server blocks; run it on Dispatchers.IO (or use the `…Later` calls).
 */
class Sync(context: Context, private val server: ServerApi, private val settings: ServerSettings) {
    private val prefs = context.applicationContext.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
    private val outbox = OutboxFile(File(context.applicationContext.filesDir, "outbox.json"))

    // one at a time, in order, so an older place never overwrites a newer one
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO.limitedParallelism(1))
    private val sending = Mutex()

    /** How this phone names itself in progress (`android:<device id>`). */
    val device: String get() = "android:${settings.deviceId}"

    /** Moves whenever something is added to or sent from the outbox. */
    val changes = outbox.changes

    /** The account the phone's copies and the outbox belong to: `<server>|<username>`. */
    private val owner: String get() = Outbox.account(settings.base.value, settings.username)

    /** The phone's copy of the place in [editionId] (null when never opened on this phone). */
    fun local(editionId: Long): Progress? = PlaybackRules.progressOf(parseJsonObject(prefs.getString(key(editionId), null)))

    /**
     * Where to go on from in [editionId]: the newer of the phone's copy and the server's (when it can be reached).
     * The server's place is kept on the phone too.
     */
    fun open(editionId: Long): Progress? {
        val mine = local(editionId)
        val theirs = try {
            PlaybackRules.progressOf(server.get("/api/progress/$editionId") as? JsonObject)
        } catch (e: ApiException) {
            // 404: never opened anywhere
            if (e.code == 404) null else return mine
        } catch (_: IOException) {
            return mine
        }
        val chosen = PlaybackRules.newer(mine, theirs)
        if (chosen != null && chosen == theirs) keep(chosen)
        return chosen
    }

    /** Keeps [p] on the phone and queues it for the server (only the latest per book waits). */
    fun save(p: Progress) {
        keep(p)
        outbox.update { it.own(owner).add(OutAction(KIND_PROGRESS, p.editionId.toString(), p.toJson(), p.updatedAt.toLong()), OutMerge.Latest) }
    }

    /** [save], then sends what waits, off the caller's thread. */
    fun saveLater(p: Progress) {
        scope.launch {
            save(p)
            flush()
        }
    }

    /** The bookmarks of [editionId]: the server's, then those still waiting to be sent (id 0). Throws when offline. */
    fun bookmarks(editionId: Long): List<Bookmark> {
        val waiting = outbox.read().takeIf { it.belongsTo(owner) }?.of(KIND_BOOKMARK).orEmpty()
            .filter { it.data.long("edition_id") == editionId }
            .map { Bookmark(0, it.data.long("position_ms"), PlaybackRules.locatorOf(it.data["locator"]), it.data.str("note"), it.at.toDouble()) }
        val gone = outbox.read().takeIf { it.belongsTo(owner) }?.of(KIND_UNBOOKMARK).orEmpty().map { it.key.toLong() }.toSet()
        val kept = PlaybackRules.bookmarksOf(server.getObject("/api/editions/$editionId/bookmarks")).filter { it.id !in gone }
        return kept + waiting
    }

    /** Adds a bookmark: queued, then sent when the server can be reached. */
    fun addBookmark(editionId: Long, positionMs: Long?, locator: Locator?, note: String) {
        val now = System.currentTimeMillis() / 1000
        val body = buildJsonObject {
            put("edition_id", editionId)
            PlaybackRules.bookmarkBody(positionMs, locator, note).forEach { (k, v) -> put(k, v) }
        }
        outbox.update { it.own(owner).add(OutAction(KIND_BOOKMARK, "$editionId:$now:${positionMs ?: locator?.totalProgression}", body, now)) }
    }

    /** Removes the bookmark [id] (one still waiting to be sent is just dropped). */
    fun removeBookmark(b: Bookmark, editionId: Long) {
        outbox.update { box ->
            if (b.id == 0L) {
                box.forget(box.of(KIND_BOOKMARK).filter { it.data.long("edition_id") == editionId && it.at.toDouble() == b.createdAt })
            } else {
                box.own(owner).add(OutAction(KIND_UNBOOKMARK, b.id.toString(), JsonObject(emptyMap()), System.currentTimeMillis() / 1000))
            }
        }
    }

    /** Sends what waits, in order, off the caller's thread. */
    fun flushLater() {
        scope.launch { flush() }
    }

    /**
     * Sends what waits, in the order it was made. Stops at the first change the server can't be reached for (it
     * stays for next time); a change the server refuses for good (it answers 4xx: the book is gone, say) is dropped.
     * Returns how many changes went.
     */
    suspend fun flush(): Int = sending.withLock {
        val box = outbox.read()
        if (!box.belongsTo(owner) || settings.token.isEmpty()) return 0
        var sent = 0
        for (a in box.actions) {
            try {
                when (a.kind) {
                    KIND_PROGRESS -> {
                        val stands = PlaybackRules.progressOf(server.put("/api/progress/${a.key}", a.data) as? JsonObject)
                        // the server keeps a newer place (read elsewhere meanwhile): the phone takes it
                        if (stands != null && stands.updatedAt > (local(stands.editionId)?.updatedAt ?: 0.0)) keep(stands)
                    }

                    KIND_BOOKMARK -> server.post("/api/editions/${a.data.long("edition_id")}/bookmarks", JsonObject(a.data.filterKeys { it != "edition_id" }))

                    KIND_UNBOOKMARK -> server.delete("/api/bookmarks/${a.key}")
                }
            } catch (e: ApiException) {
                // signed out, or the server is busy: try again later; any other refusal won't change
                if (e.code == 401 || e.code == 408 || e.code == 429 || e.code >= 500) break
            } catch (_: IOException) {
                break
            }
            outbox.update { it.forget(listOf(a)) }
            sent += 1
        }
        sent
    }

    private fun keep(p: Progress) {
        prefs.edit().putString(key(p.editionId), p.toJson().toString()).apply()
    }

    private fun key(editionId: Long): String = "$owner|$editionId"

    private companion object {
        const val PREFS = "progress"
        const val KIND_PROGRESS = "progress"
        const val KIND_BOOKMARK = "bookmark"
        const val KIND_UNBOOKMARK = "unbookmark"
    }
}
