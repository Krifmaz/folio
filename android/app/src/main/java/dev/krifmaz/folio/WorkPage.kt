package dev.krifmaz.folio

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.krifmaz.kit.compose.theme.palette
import dev.krifmaz.kit.compose.widgets.BackBar
import dev.krifmaz.kit.compose.widgets.CardBox
import dev.krifmaz.kit.compose.widgets.ErrorBox
import dev.krifmaz.kit.compose.widgets.Gutter
import dev.krifmaz.kit.compose.widgets.SectionHead
import dev.krifmaz.kit.compose.widgets.Spinner
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/**
 * One book: its cover, title, authors and series, the blurb, and every exact edition kept for it with its facts
 * (narrator, runtime, abridged or not, language, pages). [actions] draws each edition's buttons (Listen, Read,
 * Download), so this page doesn't need to know the player or the reader.
 *
 * @param app the app (its server and image loader).
 * @param id the work's id.
 * @param onBack back to the page below.
 * @param actions the buttons under an edition.
 */
@Composable
fun WorkPage(app: App, id: Long, onBack: () -> Unit, actions: @Composable (Edition) -> Unit = {}) {
    var work by remember { mutableStateOf<Work?>(null) }
    var error by remember { mutableStateOf("") }
    var round by remember { mutableIntStateOf(0) }
    LaunchedEffect(id, round) {
        error = ""
        try {
            work = withContext(Dispatchers.IO) { LibraryRules.workOf(app.server.getObject("/api/library/works/$id")) }
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            error = errorWords("The book couldn't be loaded", e)
        }
    }
    Column(Modifier.fillMaxSize()) {
        BackBar(work?.summary?.title ?: "Book", onBack)
        val w = work
        when {
            w == null && error.isNotEmpty() -> ErrorBox(error) { round += 1 }

            w == null -> Box(Modifier.fillMaxWidth().padding(24.dp), contentAlignment = Alignment.Center) { Spinner(24.dp) }

            else -> WorkBody(app, w, actions)
        }
    }
}

/** The page's content once the work is loaded. */
@Composable
private fun WorkBody(app: App, w: Work, actions: @Composable (Edition) -> Unit) {
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState())) {
        Row(Modifier.padding(Gutter), horizontalArrangement = Arrangement.spacedBy(16.dp)) {
            Cover(app, w.summary.coverUrl, 104.dp)
            Column(Modifier.weight(1f)) {
                Text(w.summary.title, style = MaterialTheme.typography.titleLarge)
                Text(LibraryRules.names(w.summary.authors.map { it.name }), color = MaterialTheme.palette.muted)
                LibraryRules.seriesLine(w.summary.series).takeIf {
                    it.isNotEmpty()
                }?.let { Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.palette.muted) }
                w.summary.year?.let { Text("$it", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.palette.faint) }
            }
        }
        if (w.description.isNotBlank()) {
            Text(w.description, Modifier.padding(horizontal = Gutter), style = MaterialTheme.typography.bodyMedium)
        }
        SectionHead("Editions", subtitle = if (w.editions.size == 1) "1 edition" else "${w.editions.size} editions")
        Column(Modifier.padding(horizontal = Gutter), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            w.editions.forEach { e -> EditionCard(e, actions) }
        }
        Box(Modifier.height(32.dp))
    }
}

/** One edition: its title (when it differs), its facts on one line, its ASIN or ISBN and publisher, its buttons. */
@Composable
private fun EditionCard(e: Edition, actions: @Composable (Edition) -> Unit) {
    CardBox(Modifier.fillMaxWidth()) {
        Column(Modifier.padding(12.dp)) {
            Text(LibraryRules.factsLine(e), style = MaterialTheme.typography.titleSmall)
            if (e.subtitle.isNotBlank()) Text(e.subtitle, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.palette.muted)
            val ids = listOfNotNull(
                e.asin.takeIf { it.isNotBlank() }?.let { "ASIN $it" },
                e.isbn13.takeIf { it.isNotBlank() }?.let { "ISBN $it" },
                e.publisher.takeIf { it.isNotBlank() },
                e.releaseDate.takeIf { it.isNotBlank() },
            ).joinToString(" · ")
            if (ids.isNotEmpty()) Text(ids, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.palette.faint)
            Row(Modifier.padding(top = 8.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) { actions(e) }
        }
    }
}
