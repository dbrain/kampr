package dev.kampr.shared.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr

// A pane whose node cannot run herdr never paints, and used to look exactly like a pane that had
// not painted *yet*. Both halves of the difference live here: a mark for the operator who already
// knows, and the whole sentence for the one who does not.
//
// Neither is a strip. A strip is dismissed and does not come back; this is the state of the pane
// underneath it, and it goes when the node can stream again — which it retries for ever to do.
private const val NO_PICTURE = "This pane has no picture"

private val NOTICE_MAX_WIDTH = 420.dp

@Composable
internal fun StreamBadge() {
    val tokens = Kampr.tokens
    Box(Modifier.announce(NO_PICTURE)) {
        StatusBadge("No stream", tokens.color.blocked, tokens.color.blockedBg)
    }
}

// Where the grid would have been, and only there: a pane that has a grid already has the truth on
// its surface, and covering a last-known screen with a card is the stale badge's job done worse.
@Composable
fun StreamNotice(detail: String, modifier: Modifier = Modifier) {
    val tokens = Kampr.tokens
    // The one sentence on a pane the node wrote itself, and the one a reader quotes when they
    // report that a pane never painted. The pane around it is the terminal's, which selects its
    // own way, so this card asks for its own.
    Surface(modifier.widthIn(max = NOTICE_MAX_WIDTH).announce(detail, urgent = true)) {
        SelectionContainer {
            Column(
                Modifier.padding(horizontal = 16.dp, vertical = 14.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(9.dp),
                ) {
                    IconGlyph(KamprIcons.warning, 15.dp, tokens.color.blocked)
                    KText(NO_PICTURE, tokens.type.bodyStrong, tokens.color.text)
                }
                KText(detail, tokens.type.caption, tokens.color.mute, maxLines = 10)
            }
        }
    }
}
