package dev.kampr.shared.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.paneTitle
import dev.kampr.shared.model.statusOf
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.wire.PaneInfo

// The panes split beside this one, one tap apart. A phone has room for one grid, so a tab is a
// strip to flip along; a desk can draw the tab whole, which `onSideBySide` opens.
@Composable
fun TabStrip(
    tab: List<PaneInfo>,
    showing: String,
    onTab: (String) -> Unit,
    onSideBySide: (() -> Unit)?,
    modifier: Modifier = Modifier,
) {
    if (tab.size < 2) return
    val tokens = Kampr.tokens
    val shape = RoundedCornerShape(tokens.radii.sm)
    Row(
        modifier
            .fillMaxWidth()
            .background(tokens.color.bar)
            .edgeBottom()
            .padding(horizontal = 8.dp, vertical = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        Row(
            Modifier.weight(1f).horizontalScroll(rememberScrollState()),
            horizontalArrangement = Arrangement.spacedBy(4.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            for (pane in tab) {
                val here = pane.id == showing
                val title = paneTitle(pane)
                Row(
                    Modifier
                        .let { if (here) it.background(tokens.color.raise, shape) else it }
                        .action("$title, in this tab", { if (!here) onTab(pane.id) }, shape, role = Role.Tab, selected = here)
                        .padding(horizontal = 10.dp, vertical = 6.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(6.dp),
                ) {
                    Box(Modifier.padding(top = 1.dp)) { StatusMark(statusOf(pane), 6.dp) }
                    KText(title, tokens.type.meta, if (here) tokens.color.text else tokens.color.dim, maxLines = 1)
                }
            }
        }
        onSideBySide?.let { open ->
            GlyphAction(
                KamprIcons.split,
                "Show this tab's ${tab.size} panes side by side",
                tokens.color.dim,
                LANDSCAPE_TOUCH,
                onClick = open,
            )
        }
    }
}
