package dev.krifmaz.folio

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import dev.krifmaz.kit.compose.core.formatSize
import dev.krifmaz.kit.compose.dialogs.ConfirmDialog
import dev.krifmaz.kit.compose.theme.palette
import dev.krifmaz.kit.compose.toasts.Toasts
import dev.krifmaz.kit.compose.widgets.BackBar
import dev.krifmaz.kit.compose.widgets.EmptyState
import dev.krifmaz.kit.compose.widgets.Gutter
import dev.krifmaz.kit.compose.widgets.RowDivider
import dev.krifmaz.kit.compose.widgets.SmallButton
import dev.krifmaz.kit.compose.widgets.Spinner
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import dev.krifmaz.kit.compose.R as KitR

/**
 * An edition's Download button: Download, the percent while it comes (tap to stop), Try again when it stopped (the
 * reason shows in a toast with Copy), and On this phone (tap to remove, after asking) once it is kept.
 */
@Composable
fun DownloadButton(app: App, edition: Edition) {
    val active by app.downloads.active.collectAsState()
    val kept by app.downloads.kept.collectAsState()
    val state = active[edition.id]
    val isKept = edition.id in kept
    var askRemove by remember { mutableStateOf(false) }
    SmallButton(DownloadRules.label(isKept, state), iconRes = KitR.drawable.kit_ic_download) {
        when {
            isKept -> askRemove = true

            state == null -> app.downloads.start(edition.id)

            // it stopped: the files that came whole stay, the rest is asked for again
            state.error.isNotEmpty() -> app.downloads.start(edition.id)

            else -> {
                app.downloads.remove(edition.id)
                Toasts.show("Download stopped")
            }
        }
    }
    LaunchedEffect(state?.error) {
        state?.error?.takeIf { it.isNotEmpty() }?.let { Toasts.show(it, "err") }
    }
    if (askRemove) {
        ConfirmDialog(
            title = "Remove from this phone?",
            text = "\"${edition.title}\" stays on the server and in your library; only this phone's copy goes. Your place is kept.",
            okLabel = "Remove",
            danger = true,
            onDismiss = { askRemove = false },
            onConfirm = {
                app.downloads.remove(edition.id)
                Toasts.show("Removed from this phone")
            },
        )
    }
}

/**
 * The books kept on this phone, with the room each takes, to open (offline too) or remove; downloads on their way
 * show at the top.
 *
 * @param app the app (its downloads).
 * @param onBack back to the page below.
 * @param onOpen opens a kept book in the player or the reader.
 */
@Composable
fun DownloadsPage(app: App, onBack: () -> Unit, onOpen: (Edition) -> Unit) {
    val kept by app.downloads.kept.collectAsState()
    val active by app.downloads.active.collectAsState()
    var books by remember { mutableStateOf<List<KeptBook>?>(null) }
    var askRemove by remember { mutableStateOf<KeptBook?>(null) }
    LaunchedEffect(kept) { books = withContext(Dispatchers.IO) { app.downloads.list() } }
    Column(Modifier.fillMaxSize()) {
        BackBar("Downloads", onBack)
        val list = books
        when {
            list == null -> Box(Modifier.fillMaxWidth().padding(24.dp), contentAlignment = Alignment.Center) { Spinner(24.dp) }

            list.isEmpty() && active.isEmpty() ->
                EmptyState(KitR.drawable.kit_ic_download, "Nothing on this phone", "Download a book from its page to listen or read without the server.")

            else -> LazyColumn(Modifier.fillMaxSize()) {
                items(active.values.toList(), key = { "a${it.editionId}" }) { s ->
                    Column(Modifier.fillMaxWidth().padding(horizontal = Gutter, vertical = 10.dp)) {
                        Text(DownloadRules.label(false, s))
                        if (s.error.isNotEmpty()) Text(s.error, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.palette.muted)
                    }
                    RowDivider()
                }
                items(list, key = { "k${it.edition.id}" }) { b ->
                    Row(
                        Modifier.fillMaxWidth().clickable { onOpen(b.edition) }.padding(horizontal = Gutter, vertical = 10.dp),
                        horizontalArrangement = Arrangement.spacedBy(12.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Cover(app, b.edition.coverUrl, 40.dp)
                        Column(Modifier.weight(1f)) {
                            Text(b.edition.title, maxLines = 2, overflow = TextOverflow.Ellipsis)
                            Text(
                                (if (b.edition.isAudiobook) "Audiobook" else "Ebook") + " · " + formatSize(b.bytes),
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.palette.muted,
                            )
                        }
                        SmallButton("Remove", iconRes = KitR.drawable.kit_ic_delete) { askRemove = b }
                    }
                    RowDivider()
                }
            }
        }
    }
    askRemove?.let { b ->
        ConfirmDialog(
            title = "Remove from this phone?",
            text = "\"${b.edition.title}\" stays on the server and in your library; only this phone's copy goes (${formatSize(b.bytes)}). Your place is kept.",
            okLabel = "Remove",
            danger = true,
            onDismiss = { askRemove = null },
            onConfirm = {
                app.downloads.remove(b.edition.id)
                Toasts.show("Removed from this phone")
            },
        )
    }
}
