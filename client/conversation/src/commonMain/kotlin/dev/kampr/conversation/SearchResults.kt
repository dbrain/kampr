package dev.kampr.conversation

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicText
import androidx.compose.foundation.text.selection.DisableSelection
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.LabelText
import dev.kampr.shared.ui.action
import dev.kampr.shared.ui.edge
import dev.kampr.shared.ui.glyphFallback
import dev.kampr.shared.ui.touchable

// The matches and nothing else, which is the one thing stepping cannot give a reader: forty turns
// of prose between two hits is forty turns of scrolling to compare them. Each row is the line the
// hit is on, who said it, when, and how far back — and pressing one aims the transcript at it,
// paging back for a turn this device does not hold yet.
//
// A list of *turns*, not of occurrences: a turn that says the word eleven times is one row that
// says so, because a row per occurrence is the same card eleven times over. It is the same rule
// the counter beside it keeps.
@Composable
fun SearchResults(
    results: List<Result>,
    focus: Int,
    query: String,
    agent: String?,
    now: Double,
    // Whether the node searched the transcript. Where it did not, the count is over the turns this
    // device holds, and the foot of the list says so rather than letting a tidy list imply it was
    // the whole conversation.
    whole: Boolean,
    // Every matching turn the node found, where that is more than the list it handed over. A list
    // of fifty under a search that found two hundred is not the search, and the one thing a reader
    // can do about it — ask something narrower — only occurs to them if they are told.
    found: Int?,
    onPick: (Int) -> Unit,
    modifier: Modifier = Modifier,
) {
    val tokens = Kampr.tokens
    LazyColumn(
        modifier.fillMaxSize().background(tokens.color.bg),
        contentPadding = PaddingValues(16.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        itemsIndexed(results, key = { _, hit -> hit.turn }) { at, hit ->
            ResultRow(hit, at == focus, query, agent, now) { onPick(at) }
        }
        item(key = "reach") {
            DisableSelection {
                KText(
                    when {
                        found != null && found > results.size ->
                            "$found matches in the transcript — the newest ${results.size} are listed, " +
                                "so narrow the search to reach the rest"
                        whole -> "the whole transcript was searched"
                        else -> "searched the turns this device holds — older ones have not been asked for"
                    },
                    tokens.type.meta,
                    tokens.color.mute,
                    Modifier.fillMaxWidth().padding(top = 6.dp),
                    maxLines = 3,
                )
            }
        }
    }
}

@Composable
private fun ResultRow(
    hit: Result,
    standing: Boolean,
    query: String,
    agent: String?,
    now: Double,
    onPick: () -> Unit,
) {
    val tokens = Kampr.tokens
    val palette = rememberConversationPalette()
    val skin = speakerSkin(if (hit.role == "user") Speaker.You else Speaker.Agent, agent)
    val shape = RoundedCornerShape(tokens.radii.sm)
    val stamp = turnStamp(hit.at, now)
    val depth = hit.fromEnd?.let { if (it == 0) "newest" else "$it back" }
    val style = tokens.type.caption.copy(color = tokens.color.text)
    val line = remember(hit.text, query, palette) {
        AnnotatedString(hit.text).markMatches(query, palette.match)
    }
    // The row the transcript is aimed at is marked rather than merely remembered: a list where
    // every row looks the same loses the reader's place the moment they scroll it.
    Column(
        Modifier
            .fillMaxWidth()
            .background(if (standing) skin.ground else tokens.color.surface, shape)
            .edge(tokens.card, shape)
            .touchable()
            .action("Go to ${skin.label}'s turn${stamp?.let { ", $it" } ?: ""}: ${hit.text}", onPick, shape)
            .padding(horizontal = 11.dp, vertical = 9.dp),
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        Row(
            Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(7.dp),
        ) {
            LabelText(skin.label, tokens.type.metaSmall, skin.rail)
            KText(stamp.orEmpty(), tokens.type.meta, tokens.color.mute, Modifier.weight(1f))
            if (hit.hits > 1) KText("${hit.hits}×", tokens.type.meta, tokens.color.mute)
            depth?.let { KText(it, tokens.type.meta, tokens.color.mute) }
        }
        BasicText(line.glyphFallback(style), style = style, maxLines = 3)
    }
}
