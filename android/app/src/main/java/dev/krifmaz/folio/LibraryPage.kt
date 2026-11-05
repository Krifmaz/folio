package dev.krifmaz.folio

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
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
import androidx.compose.ui.draw.clip
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import coil3.compose.AsyncImage
import dev.krifmaz.kit.compose.core.ApiException
import dev.krifmaz.kit.compose.theme.palette
import dev.krifmaz.kit.compose.widgets.BackBar
import dev.krifmaz.kit.compose.widgets.ChoiceRow
import dev.krifmaz.kit.compose.widgets.EmptyState
import dev.krifmaz.kit.compose.widgets.ErrorBox
import dev.krifmaz.kit.compose.widgets.Gutter
import dev.krifmaz.kit.compose.widgets.KitField
import dev.krifmaz.kit.compose.widgets.RowDivider
import dev.krifmaz.kit.compose.widgets.Spinner
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext
import java.io.IOException
import dev.krifmaz.kit.compose.R as KitR

/** The library's format filter, in the order the choice row shows them: its words and the server's `format`. */
private val FORMATS = listOf("All" to "", "Audiobooks" to "audiobook", "Ebooks" to "ebook")

/**
 * The library: every work the server keeps, newest first, with a search (title or author) and a format filter.
 * Tapping a book opens its page ([WorkPage]).
 *
 * @param app the app (its server and image loader).
 * @param onBack back to the page below.
 * @param onOpen opens the work with this id.
 */
@Composable
fun LibraryPage(app: App, onBack: () -> Unit, onOpen: (Long) -> Unit) {
    var query by remember { mutableStateOf("") }
    var format by remember { mutableIntStateOf(0) }
    var round by remember { mutableIntStateOf(0) }
    var works by remember { mutableStateOf<List<WorkSummary>?>(null) }
    var error by remember { mutableStateOf("") }
    LaunchedEffect(query, format, round) {
        // wait for a pause in typing before asking the server
        if (query.isNotEmpty()) delay(300)
        error = ""
        try {
            val path = "/api/library/works" + LibraryRules.worksQuery(query, FORMATS[format].second)
            works = withContext(Dispatchers.IO) { LibraryRules.worksOf(app.server.getObject(path)) }
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            error = errorWords("The library couldn't be loaded", e)
        }
    }
    Column(Modifier.fillMaxSize()) {
        BackBar("Library", onBack)
        KitField(
            query,
            { query = it },
            Modifier.fillMaxWidth().padding(horizontal = Gutter),
            placeholder = "Search titles and authors",
            leadingIcon = KitR.drawable.kit_ic_search,
        )
        ChoiceRow(FORMATS.map { it.first }, format, { format = it }, Modifier.padding(start = Gutter, top = 8.dp, bottom = 4.dp))
        val list = works
        when {
            error.isNotEmpty() && list == null -> ErrorBox(error) { round += 1 }

            list == null -> Box(Modifier.fillMaxWidth().padding(24.dp), contentAlignment = Alignment.Center) { Spinner(24.dp) }

            list.isEmpty() ->
                EmptyState(
                    KitR.drawable.kit_ic_search,
                    if (query.isBlank()) "No books yet" else "Nothing found",
                    if (query.isBlank()) "Books you add on the web app show up here." else "No title or author has \"$query\" in it.",
                )

            else ->
                LazyColumn(Modifier.fillMaxSize()) {
                    if (error.isNotEmpty()) item { ErrorBox(error) { round += 1 } }
                    items(list, key = { it.id }) { w ->
                        WorkRow(app, w) { onOpen(w.id) }
                        RowDivider()
                    }
                }
        }
    }
}

/** One book in the list: its cover, title, authors, series and formats. */
@Composable
private fun WorkRow(app: App, w: WorkSummary, onClick: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().clickable(onClick = onClick).padding(horizontal = Gutter, vertical = 10.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Cover(app, w.coverUrl, 56.dp)
        Column(Modifier.weight(1f)) {
            Text(w.title, maxLines = 2, overflow = TextOverflow.Ellipsis)
            Text(
                LibraryRules.names(
                    w.authors.map {
                        it.name
                    },
                ),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.palette.muted,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            val line = listOf(LibraryRules.seriesLine(w.series), LibraryRules.formatsLine(w.formats)).filter { it.isNotEmpty() }.joinToString(" · ")
            if (line.isNotEmpty()) Text(line, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.palette.faint, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
    }
}

/** A book's cover at [width] (2:3), or an empty panel of that size when there is no picture. */
@Composable
fun Cover(app: App, url: String, width: Dp, modifier: Modifier = Modifier) {
    val shape = RoundedCornerShape(4.dp)
    val box = modifier.size(width, width * 1.5f).clip(shape).background(MaterialTheme.palette.panel2)
    if (url.isBlank()) {
        Box(box)
    } else {
        AsyncImage(model = url, contentDescription = null, imageLoader = app.imageLoader, contentScale = ContentScale.Crop, modifier = box)
    }
}

/** The words for a failed call: "[what]: [the server's sentence]" or why the server can't be reached. */
fun errorWords(what: String, e: Exception): String = when (e) {
    is ApiException -> "$what: ${e.message}"
    is IOException -> "$what. The server can't be reached: ${e.message ?: "no answer"}. Check the connection and try again."
    else -> "$what: ${e.message ?: e.javaClass.simpleName}"
}
