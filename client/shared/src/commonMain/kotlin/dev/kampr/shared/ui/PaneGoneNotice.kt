package dev.kampr.shared.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.absolutePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.PaneGone
import dev.kampr.shared.theme.Kampr

// A shell exits, its pane leaves the herd, and this screen is left sitting on the last grid it
// printed. The only thing that used to change was the title, which fell back to the raw pane id —
// a node ULID and a herdr coordinate. That reads as the app having lost its place rather than as
// the shell having finished, which is why the report was about a title and not about a pane.
//
// So: the header keeps the name the pane had, and the news gets said. Two readings, because they
// are not the same news — a shell that exited is over, and a node that left the herd is a machine
// that may be back on its own with the pane still on it.
private val WORDS = mapOf(
    PaneGone.Shell to Gone(
        "Closed",
        "Closed — this pane is no longer in the herd",
        "This pane has closed — what is on screen is the last thing it printed",
    ),
    PaneGone.Node to Gone(
        "Node gone",
        "Node gone — the node this pane was on is no longer in the herd",
        "The node this pane was on has left the herd — what is on screen is the last thing it printed",
    ),
)

private class Gone(val badge: String, val spokenBadge: String, val sentence: String)

@Composable
internal fun GoneBadge(gone: PaneGone) {
    val tokens = Kampr.tokens
    val words = WORDS.getValue(gone)
    StatusBadge(words.badge, tokens.color.blocked, tokens.color.blockedBg, label = words.spokenBadge)
}

// A band across the chrome rather than a card over the grid. The grid below is the last true thing
// the shell printed — its exit message, its stack trace, whatever it was doing when it stopped —
// and covering that is the mistake `StreamNotice` already documents one file over. The band sits
// inside the Column the header measures, so the terminal insets past it and no row hides behind it.
@Composable
fun GoneStrip(gone: PaneGone, modifier: Modifier = Modifier) {
    val tokens = Kampr.tokens
    val safe = LocalSafeArea.current
    val words = WORDS.getValue(gone)
    Row(
        modifier
            .background(tokens.color.blockedBg)
            .edgeBottom()
            .announce(words.sentence, urgent = true)
            .absolutePadding(left = 16.dp + safe.left, right = 16.dp + safe.right)
            .padding(vertical = 9.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(9.dp),
    ) {
        Box(Modifier.padding(top = 1.dp)) { IconGlyph(KamprIcons.warning, 14.dp, tokens.color.blocked) }
        KText(words.sentence, tokens.type.caption, tokens.color.text, maxLines = 3)
    }
}
