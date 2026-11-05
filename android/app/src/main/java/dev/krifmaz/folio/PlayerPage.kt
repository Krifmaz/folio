package dev.krifmaz.folio

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Slider
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.media3.common.Player
import dev.krifmaz.kit.compose.theme.palette
import dev.krifmaz.kit.compose.toasts.Toasts
import dev.krifmaz.kit.compose.widgets.BackBar
import dev.krifmaz.kit.compose.widgets.EmptyState
import dev.krifmaz.kit.compose.widgets.ErrorBox
import dev.krifmaz.kit.compose.widgets.Gutter
import dev.krifmaz.kit.compose.widgets.MenuEntry
import dev.krifmaz.kit.compose.widgets.PopMenu
import dev.krifmaz.kit.compose.widgets.RowDivider
import dev.krifmaz.kit.compose.widgets.SectionHead
import dev.krifmaz.kit.compose.widgets.SmallButton
import dev.krifmaz.kit.compose.widgets.Spinner
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import dev.krifmaz.kit.compose.R as KitR

/** What the player page loads before it can play: the edition, its audio files and where to go on from. */
private data class Opened(val book: NowPlaying.Book, val start: Progress?)

/**
 * The audiobook player: cover, chapter, a slider through the chapter, Back 15 s / Play / Forward 30 s, previous
 * and next chapter, speed, the sleep timer, bookmarks and the chapter list. It goes on from the newer of this
 * phone's place and the server's (another device's), and says so when it came from elsewhere.
 *
 * @param app the app (its server, sync and player).
 * @param editionId the audiobook edition to play.
 * @param onBack back to the page below.
 */
@Composable
fun PlayerPage(app: App, editionId: Long, onBack: () -> Unit) {
    val controller by app.player.controller.collectAsState()
    val loaded by NowPlaying.book.collectAsState()
    var opened by remember { mutableStateOf<Opened?>(null) }
    var error by remember { mutableStateOf("") }
    var round by remember { mutableIntStateOf(0) }
    LaunchedEffect(Unit) { app.player.connect() }
    LaunchedEffect(editionId, round) {
        if (loaded?.edition?.id == editionId) return@LaunchedEffect
        error = ""
        try {
            opened = withContext(Dispatchers.IO) {
                val (edition, files) = BookFiles.editionWithFiles(app, editionId)
                val audio = PlaybackRules.audio(files)
                Opened(NowPlaying.Book(edition, audio, PlaybackRules.bookChapters(audio, edition.chapters)), app.sync.open(editionId))
            }
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            error = errorWords("The audiobook couldn't be opened", e)
        }
    }
    // load it into the player once both the files and the player are there
    LaunchedEffect(opened, controller) {
        val o = opened ?: return@LaunchedEffect
        if (controller == null || o.book.audio.isEmpty() || loaded?.edition?.id == editionId) return@LaunchedEffect
        app.player.load(o.book, o.start?.positionMs ?: 0, app.playerPrefs.speed) { f -> app.downloads.uriOf(editionId, f) }
        PlaybackRules.movedNote(o.start, app.sync.device)?.let { Toasts.show(it) }
    }
    val book = loaded?.takeIf { it.edition.id == editionId }
    Column(Modifier.fillMaxSize()) {
        BackBar(book?.edition?.title ?: opened?.book?.edition?.title ?: "Player", onBack)
        val c = controller
        when {
            error.isNotEmpty() -> ErrorBox(error) { round += 1 }

            opened?.book?.audio?.isEmpty() == true ->
                EmptyState(KitR.drawable.kit_ic_file, "Not here yet", "This audiobook has no files on the server yet. It plays once it has been downloaded and checked.")

            book == null || c == null -> Box(Modifier.fillMaxWidth().padding(24.dp), contentAlignment = Alignment.Center) { Spinner(24.dp) }

            else -> PlayerBody(app, c, book)
        }
    }
}

/** The player's controls for a loaded [book]. */
@Composable
private fun PlayerBody(app: App, c: Player, book: NowPlaying.Book) {
    val scope = rememberCoroutineScope()
    val sleep by NowPlaying.sleep.collectAsState()
    var position by remember { mutableLongStateOf(NowPlaying.positionOf(c)) }
    var playing by remember { mutableStateOf(c.isPlaying) }
    var speed by remember { mutableFloatStateOf(c.playbackParameters.speed) }
    var sleepLeft by remember { mutableStateOf<Long?>(null) }
    // a drag on the slider: where the thumb is until it is let go (null while not dragging)
    var dragging by remember { mutableStateOf<Float?>(null) }
    var speedMenu by remember { mutableStateOf(false) }
    var sleepMenu by remember { mutableStateOf(false) }
    var bookmarks by remember { mutableStateOf<List<Bookmark>?>(null) }
    var marksRound by remember { mutableIntStateOf(0) }
    LaunchedEffect(c) {
        while (true) {
            position = NowPlaying.positionOf(c)
            playing = c.isPlaying || (c.playWhenReady && c.playbackState == Player.STATE_BUFFERING)
            speed = c.playbackParameters.speed
            sleepLeft = NowPlaying.sleepLeftMs(c)
            delay(500)
        }
    }
    LaunchedEffect(book.edition.id, marksRound) {
        bookmarks = try {
            withContext(Dispatchers.IO) { app.sync.flush().let { app.sync.bookmarks(book.edition.id) } }
        } catch (e: CancellationException) {
            throw e
        } catch (_: Exception) {
            // offline: the list says so
            null
        }
    }
    val chapterIndex = PlaybackRules.chapterAt(book.chapters, position)
    val chapter = book.chapters.getOrNull(chapterIndex)
    val from = chapter?.startMs ?: 0L
    val to = chapter?.endMs?.takeIf { it > from } ?: book.totalMs
    val span = (to - from).coerceAtLeast(1)

    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState())) {
        Box(Modifier.fillMaxWidth().padding(top = 8.dp), contentAlignment = Alignment.Center) { Cover(app, book.edition.coverUrl, 180.dp) }
        Text(
            book.edition.title,
            Modifier.fillMaxWidth().padding(horizontal = Gutter, vertical = 8.dp),
            style = MaterialTheme.typography.titleLarge,
            textAlign = TextAlign.Center,
        )
        Text(
            LibraryRules.names(book.edition.authors.map { it.name }) + if (book.edition.narrators.isEmpty()) "" else " · read by " + LibraryRules.names(book.edition.narrators),
            Modifier.fillMaxWidth().padding(horizontal = Gutter),
            color = MaterialTheme.palette.muted,
            textAlign = TextAlign.Center,
        )
        if (chapter != null) {
            Text(
                chapter.title + "  ·  ${chapterIndex + 1} of ${book.chapters.size}",
                Modifier.fillMaxWidth().padding(horizontal = Gutter, vertical = 8.dp),
                style = MaterialTheme.typography.bodySmall,
                textAlign = TextAlign.Center,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        val shown = dragging ?: ((position - from).toFloat() / span).coerceIn(0f, 1f)
        Slider(
            value = shown,
            onValueChange = { dragging = it },
            onValueChangeFinished = {
                dragging?.let { app.player.seekTo(from + (it * span).toLong()) }
                dragging = null
            },
            modifier = Modifier.padding(horizontal = Gutter),
        )
        Row(Modifier.fillMaxWidth().padding(horizontal = Gutter)) {
            val at = from + (shown * span).toLong()
            Text(PlaybackRules.clock(at - from), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.palette.muted)
            Text(
                PlaybackRules.left(book.totalMs, at, speed),
                Modifier.weight(1f),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.palette.muted,
                textAlign = TextAlign.Center,
            )
            Text("-" + PlaybackRules.clock(to - at), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.palette.muted)
        }
        Row(Modifier.fillMaxWidth().padding(vertical = 8.dp), horizontalArrangement = Arrangement.SpaceEvenly, verticalAlignment = Alignment.CenterVertically) {
            Control(R.drawable.ic_skip_previous, "Previous chapter") { app.player.seekTo(PlaybackRules.previousChapterStart(book.chapters, position)) }
            Control(R.drawable.ic_rewind, "Back 15 seconds") { app.player.skip(-PlaybackRules.SKIP_BACK_MS) }
            IconButton(
                onClick = { if (playing) c.pause() else c.play() },
                modifier = Modifier.size(64.dp).clip(CircleShape),
            ) {
                Icon(
                    painterResource(if (playing) R.drawable.ic_pause else R.drawable.ic_play),
                    contentDescription = if (playing) "Pause" else "Play",
                    tint = MaterialTheme.palette.accent,
                    modifier = Modifier.size(48.dp),
                )
            }
            Control(R.drawable.ic_forward, "Forward 30 seconds") { app.player.skip(PlaybackRules.SKIP_FORWARD_MS) }
            Control(R.drawable.ic_skip_next, "Next chapter") { PlaybackRules.nextChapterStart(book.chapters, position)?.let { app.player.seekTo(it) } }
        }
        Row(Modifier.padding(horizontal = Gutter), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Box {
                SmallButton("Speed " + PlaybackRules.speedLabel(speed)) { speedMenu = true }
                PopMenu(
                    speedMenu,
                    { speedMenu = false },
                    PlaybackRules.SPEEDS.map { s ->
                        MenuEntry(PlaybackRules.speedLabel(s), badge = if (s == speed) "now" else null) {
                            c.setPlaybackSpeed(s)
                            app.playerPrefs.speed = s
                            speedMenu = false
                        }
                    },
                )
            }
            Box {
                val label = when {
                    sleep == null -> "Sleep timer"
                    sleep?.timer == SleepTimer.EndOfChapter -> "Sleep: end of chapter"
                    else -> "Sleep in " + PlaybackRules.clock(sleepLeft ?: 0)
                }
                SmallButton(label, iconRes = R.drawable.ic_timer) { sleepMenu = true }
                PopMenu(
                    sleepMenu,
                    { sleepMenu = false },
                    PlaybackRules.SLEEP_MINUTES.map { m ->
                        MenuEntry("$m minutes") {
                            NowPlaying.setSleep(SleepTimer.After(m * 60_000L))
                            sleepMenu = false
                        }
                    } + listOfNotNull(
                        MenuEntry("End of chapter") {
                            NowPlaying.setSleep(SleepTimer.EndOfChapter)
                            sleepMenu = false
                        },
                        if (sleep != null) {
                            MenuEntry("Turn off") {
                                NowPlaying.setSleep(null)
                                sleepMenu = false
                            }
                        } else {
                            null
                        },
                    ),
                )
            }
            SmallButton("Bookmark", iconRes = R.drawable.ic_bookmark) {
                val note = (chapter?.title?.let { "$it · " } ?: "") + PlaybackRules.clock(position)
                scope.launch {
                    withContext(Dispatchers.IO) {
                        app.sync.addBookmark(book.edition.id, position, null, note)
                        app.sync.flush()
                    }
                    Toasts.show("Bookmark added at ${PlaybackRules.clock(position)}", "ok")
                    marksRound += 1
                }
            }
        }

        SectionHead("Bookmarks")
        val marks = bookmarks
        when {
            marks == null -> Text("Bookmarks show when the server can be reached.", Modifier.padding(horizontal = Gutter), color = MaterialTheme.palette.muted)

            marks.isEmpty() -> Text("No bookmarks yet. Bookmark keeps the place you are at.", Modifier.padding(horizontal = Gutter), color = MaterialTheme.palette.muted)

            else -> marks.forEach { b ->
                MarkRow(
                    title = b.note.ifBlank { PlaybackRules.clock(b.positionMs) },
                    line = PlaybackRules.clock(b.positionMs) + if (b.id == 0L) " · waiting to be sent" else "",
                    onOpen = { app.player.seekTo(b.positionMs) },
                    onRemove = {
                        scope.launch {
                            withContext(Dispatchers.IO) {
                                app.sync.removeBookmark(b, book.edition.id)
                                app.sync.flush()
                            }
                            Toasts.show("Bookmark removed")
                            marksRound += 1
                        }
                    },
                )
            }
        }

        if (book.chapters.isNotEmpty()) {
            SectionHead("Chapters", subtitle = "${book.chapters.size} chapters")
            book.chapters.forEachIndexed { i, ch ->
                Row(
                    Modifier.fillMaxWidth().clickable { app.player.seekTo(ch.startMs) }.padding(horizontal = Gutter, vertical = 10.dp),
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    Text(
                        ch.title,
                        Modifier.weight(1f),
                        color = if (i == chapterIndex) MaterialTheme.palette.accent else MaterialTheme.palette.text,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                    Text(PlaybackRules.clock(ch.startMs), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.palette.muted)
                }
                RowDivider()
            }
        }
        Box(Modifier.height(32.dp))
    }
}

/** A round transport button with [label] read out by TalkBack. */
@Composable
private fun Control(icon: Int, label: String, onClick: () -> Unit) {
    IconButton(onClick = onClick) { Icon(painterResource(icon), contentDescription = label, tint = MaterialTheme.palette.text, modifier = Modifier.size(32.dp)) }
}

/** A bookmark in a list: tap to go there, Remove to drop it (shared by the player and the reader). */
@Composable
fun MarkRow(title: String, line: String, onOpen: () -> Unit, onRemove: () -> Unit) {
    Row(Modifier.fillMaxWidth().clickable(onClick = onOpen).padding(horizontal = Gutter, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
        Column(Modifier.weight(1f)) {
            Text(title, maxLines = 2, overflow = TextOverflow.Ellipsis)
            Text(line, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.palette.muted)
        }
        SmallButton("Remove", iconRes = KitR.drawable.kit_ic_delete, onClick = onRemove)
    }
    RowDivider()
}
