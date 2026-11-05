package dev.krifmaz.folio

import android.content.Context
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import dev.krifmaz.kit.compose.theme.palette
import dev.krifmaz.kit.compose.toasts.Toasts
import dev.krifmaz.kit.compose.widgets.BackBar
import dev.krifmaz.kit.compose.widgets.EmptyState
import dev.krifmaz.kit.compose.widgets.ErrorBox
import dev.krifmaz.kit.compose.widgets.Gutter
import dev.krifmaz.kit.compose.widgets.RowDivider
import dev.krifmaz.kit.compose.widgets.SmallButton
import dev.krifmaz.kit.compose.widgets.Spinner
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File
import dev.krifmaz.kit.compose.R as KitR

/** An ebook ready to read: the edition, the EPUB on the phone, its reading order and where to start. */
private class OpenBook(val edition: Edition, val file: File, val book: EpubBook, val start: Progress?)

/** Which list covers the page: the table of contents or the bookmarks. */
private enum class Panel { Contents, Bookmarks }

/** The reader's choices kept on the phone: the text size. */
class ReaderPrefs(context: Context) {
    private val prefs = context.applicationContext.getSharedPreferences("reader", Context.MODE_PRIVATE)

    /** The text size in percent (100 until changed). */
    var size: Int
        get() = prefs.getInt(K_SIZE, 100).coerceIn(MIN, MAX)
        set(value) = prefs.edit().putInt(K_SIZE, value.coerceIn(MIN, MAX)).apply()

    /** The limits and the step of the text size. */
    companion object {
        /** The smallest text size. */
        const val MIN = 70

        /** The largest text size. */
        const val MAX = 200

        /** How much A− and A+ change it. */
        const val STEP = 10
        private const val K_SIZE = "size"
    }
}

/**
 * The EPUB reader: the book a page at a time (tap the sides or swipe to turn, tap the middle for the controls),
 * the text size, the table of contents, bookmarks, and the place kept in step with the server and other devices.
 * The EPUB comes from the phone when it was downloaded, else from the server once (kept in the app's cache).
 *
 * @param app the app (its server and sync).
 * @param editionId the ebook edition to read.
 * @param onBack back to the page below.
 */
@Composable
fun ReaderPage(app: App, editionId: Long, onBack: () -> Unit) {
    var opened by remember { mutableStateOf<OpenBook?>(null) }
    var error by remember { mutableStateOf("") }
    var missing by remember { mutableStateOf(false) }
    var round by remember { mutableIntStateOf(0) }
    LaunchedEffect(editionId, round) {
        error = ""
        try {
            opened = withContext(Dispatchers.IO) {
                val (edition, files) = BookFiles.editionWithFiles(app, editionId)
                val epub = files.firstOrNull { it.kind == "epub" }
                if (epub == null) {
                    null
                } else {
                    val file = BookFiles.onPhone(app, editionId, epub)
                    OpenBook(edition, file, EpubBook.open(file), app.sync.open(editionId))
                }
            }
            missing = opened == null
        } catch (e: CancellationException) {
            throw e
        } catch (e: IllegalArgumentException) {
            error = e.message ?: EpubBook.BROKEN
        } catch (e: Exception) {
            error = errorWords("The book couldn't be opened", e)
        }
    }
    val o = opened
    when {
        o != null -> Reading(app, o, onBack)

        else ->
            Column(Modifier.fillMaxSize()) {
                BackBar("Reader", onBack)
                when {
                    error.isNotEmpty() -> ErrorBox(error) { round += 1 }

                    missing -> EmptyState(KitR.drawable.kit_ic_file, "Not here yet", "This ebook has no EPUB on the server yet. It opens once it has been downloaded and checked.")

                    else -> Box(Modifier.fillMaxWidth().padding(24.dp), contentAlignment = Alignment.Center) { Spinner(24.dp) }
                }
            }
    }
}

/** Reading an opened book. */
@Composable
private fun Reading(app: App, o: OpenBook, onBack: () -> Unit) {
    val scope = rememberCoroutineScope()
    val prefs = remember { ReaderPrefs(app) }
    val startPlace = remember(o) { o.book.place(o.start?.locator, o.start?.fraction ?: 0.0) }
    var index by remember { mutableIntStateOf(startPlace.first) }
    var page by remember { mutableIntStateOf(0) }
    var pages by remember { mutableIntStateOf(1) }
    var laidOut by remember { mutableStateOf(false) }
    var controls by remember { mutableStateOf(false) }
    var panel by remember { mutableStateOf<Panel?>(null) }
    var size by remember { mutableIntStateOf(prefs.size) }
    var view by remember { mutableStateOf<ReaderView?>(null) }
    var bookmarks by remember { mutableStateOf<List<Bookmark>?>(null) }
    var marksRound by remember { mutableIntStateOf(0) }
    val p = MaterialTheme.palette
    val look = ReaderLook(hex(p.bg), hex(p.text), hex(p.accent), size)
    val progression = if (pages > 1) page.toDouble() / (pages - 1) else 0.0
    val here = o.book.locator(index, progression)
    val currentHere by rememberUpdatedState(here)

    LaunchedEffect(Unit) { PlaybackRules.movedNote(o.start, app.sync.device)?.let { Toasts.show(it) } }
    // the place is saved a moment after the page settles, and when the reader closes
    LaunchedEffect(index, page, laidOut) {
        if (!laidOut) return@LaunchedEffect
        delay(1500)
        app.sync.saveLater(progressOf(o, here, app.sync.device))
    }
    DisposableEffect(o) {
        onDispose {
            app.sync.saveLater(progressOf(o, currentHere, app.sync.device))
            view?.close()
        }
    }
    LaunchedEffect(o.edition.id, marksRound, panel) {
        if (panel != Panel.Bookmarks) return@LaunchedEffect
        bookmarks = try {
            withContext(Dispatchers.IO) { app.sync.flush().let { app.sync.bookmarks(o.edition.id) } }
        } catch (e: CancellationException) {
            throw e
        } catch (_: Exception) {
            null
        }
    }
    LaunchedEffect(look) { view?.setLook(look) }
    if (panel != null) BackHandler { panel = null }

    Box(Modifier.fillMaxSize().background(p.bg)) {
        AndroidView(
            factory = { ctx ->
                ReaderView(
                    ctx,
                    o.file,
                    object : ReaderEvents {
                        override fun onPage(page2: Int, pages2: Int) {
                            page = page2
                            pages = pages2
                            laidOut = true
                        }

                        override fun onMiddleTap() {
                            controls = !controls
                        }

                        override fun onEdge(direction: Int) {
                            val next = index + direction
                            if (next !in o.book.spine.indices) {
                                if (direction > 0) Toasts.show("That's the end of the book.")
                                return
                            }
                            index = next
                            view?.open(o.book.spine[next].href, if (direction > 0) 0.0 else -1.0)
                        }
                    },
                ).also {
                    view = it
                    it.setLook(look)
                    it.open(o.book.spine[startPlace.first].href, startPlace.second)
                }
            },
            modifier = Modifier.fillMaxSize(),
        )
        if (controls) {
            Column(Modifier.fillMaxWidth().background(p.panel).align(Alignment.TopCenter)) {
                BackBar(o.edition.title, onBack)
            }
            Column(Modifier.fillMaxWidth().background(p.panel).align(Alignment.BottomCenter).padding(Gutter)) {
                val chapter = o.book.chapterTitle(index)
                if (chapter.isNotEmpty()) Text(chapter, maxLines = 1, overflow = TextOverflow.Ellipsis)
                Text(
                    "Page ${page + 1} of $pages in this part · ${(here.totalProgression * 100).toInt()}% of the book",
                    style = MaterialTheme.typography.bodySmall,
                    color = p.muted,
                )
                Row(Modifier.padding(top = 8.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    SmallButton("A−", enabled = size > ReaderPrefs.MIN) {
                        size -= ReaderPrefs.STEP
                        prefs.size = size
                    }
                    SmallButton("A+", enabled = size < ReaderPrefs.MAX) {
                        size += ReaderPrefs.STEP
                        prefs.size = size
                    }
                    SmallButton("Contents", iconRes = R.drawable.ic_list) { panel = Panel.Contents }
                    SmallButton("Bookmarks", iconRes = R.drawable.ic_bookmark) { panel = Panel.Bookmarks }
                }
                Row(Modifier.padding(top = 8.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    SmallButton("Bookmark this page", iconRes = KitR.drawable.kit_ic_add) {
                        val at = here
                        val note = (chapter.ifEmpty { o.edition.title }) + " · ${(at.totalProgression * 100).toInt()}%"
                        scope.launch {
                            withContext(Dispatchers.IO) {
                                app.sync.addBookmark(o.edition.id, null, at, note)
                                app.sync.flush()
                            }
                            Toasts.show("Bookmark added", "ok")
                            marksRound += 1
                        }
                    }
                }
            }
        }
        panel?.let { which ->
            Column(Modifier.fillMaxSize().background(p.bg)) {
                BackBar(if (which == Panel.Contents) "Contents" else "Bookmarks") { panel = null }
                val go: (Int, Double) -> Unit = { i, at ->
                    index = i
                    view?.open(o.book.spine[i].href, at)
                    panel = null
                    controls = false
                }
                when (which) {
                    Panel.Contents ->
                        if (o.book.toc.isEmpty()) {
                            Text("This book has no table of contents.", Modifier.padding(Gutter), color = p.muted)
                        } else {
                            LazyColumn {
                                itemsIndexed(o.book.toc) { _, t ->
                                    Text(
                                        t.title,
                                        Modifier.fillMaxWidth().clickable { o.book.indexOf(t.href).takeIf { it >= 0 }?.let { go(it, 0.0) } }
                                            .padding(start = Gutter + (t.depth * 16).dp, end = Gutter, top = 10.dp, bottom = 10.dp),
                                        maxLines = 2,
                                        overflow = TextOverflow.Ellipsis,
                                    )
                                    RowDivider()
                                }
                            }
                        }

                    Panel.Bookmarks -> {
                        val marks = bookmarks
                        when {
                            marks == null -> Text("Bookmarks show when the server can be reached.", Modifier.padding(Gutter), color = p.muted)

                            marks.isEmpty() -> Text("No bookmarks yet. Tap the middle of a page, then Bookmark this page.", Modifier.padding(Gutter), color = p.muted)

                            else -> LazyColumn {
                                itemsIndexed(marks) { _, b ->
                                    val at = b.locator
                                    MarkRow(
                                        title = b.note.ifBlank { "Bookmark" },
                                        line = (at?.let { "${(it.totalProgression * 100).toInt()}% of the book" } ?: "") + if (b.id == 0L) " · waiting to be sent" else "",
                                        onOpen = { o.book.place(at, 0.0).let { (i, pr) -> go(i, pr) } },
                                        onRemove = {
                                            scope.launch {
                                                withContext(Dispatchers.IO) {
                                                    app.sync.removeBookmark(b, o.edition.id)
                                                    app.sync.flush()
                                                }
                                                Toasts.show("Bookmark removed")
                                                marksRound += 1
                                            }
                                        },
                                    )
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/** The place [at] as progress to save, finished on the last page of the last part. */
private fun progressOf(o: OpenBook, at: Locator, device: String): Progress {
    val last = o.book.indexOf(at.href) == o.book.spine.lastIndex && at.progression >= 0.999
    return Progress(o.edition.id, locator = at, fraction = at.totalProgression, finished = last, device = device, updatedAt = System.currentTimeMillis() / 1000.0)
}

/** A palette colour as CSS `#rrggbb`. */
private fun hex(c: Color): String = "#%06x".format(c.toArgb() and 0xFFFFFF)
