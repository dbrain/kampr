package dev.kampr.shared.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay
import dev.kampr.shared.model.WatchPresence
import dev.kampr.shared.model.othersWatching
import dev.kampr.shared.model.watchersTag
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.wire.PaneInfo

private const val SETTLE_TICK_MS = 200L

// The frame clock rather than a wall clock: it is the one clock a Compose test can wind. The loop
// runs only while a change is in flight, and it sleeps between reads rather than taking a frame
// callback sixty times a second for the length of a six-second window.
@Composable
fun rememberWatchPresence(paneId: String, info: PaneInfo?): WatchPresence {
    val presence = remember(paneId) { WatchPresence() }
    val others = othersWatching(info)
    LaunchedEffect(presence, others) {
        withFrameNanos { presence.observe(others, it / 1_000_000) }
        while (presence.pending()) {
            delay(SETTLE_TICK_MS)
            withFrameNanos { presence.tick(it / 1_000_000) }
        }
    }
    return presence
}

// It floats over the pane and it leaves on its own. A badge that stayed up would be worn by every
// pane the desk has open, which is most of them, and would say nothing by the second day.
@Composable
fun WatchNotice(presence: WatchPresence, modifier: Modifier = Modifier) {
    val notice = presence.notice ?: return
    val tokens = Kampr.tokens
    Pill(modifier.announce(notice), background = tokens.color.raise) {
        IconGlyph(KamprIcons.alsoOpen, 13.dp, tokens.color.accent)
        KText(watchersTag(presence.others).orEmpty(), tokens.type.badge, tokens.color.text)
    }
}

// The quiet, permanent form, for a surface that is already an inventory of panes. It is named by
// the row that carries it rather than announcing itself: forty rows announcing every join is
// chatter, and none of it is about the pane the operator is actually typing into.
@Composable
fun WatchersTag(others: Int, modifier: Modifier = Modifier, glyph: Dp = 12.dp) {
    val tag = watchersTag(others) ?: return
    val tokens = Kampr.tokens
    Row(
        modifier,
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        IconGlyph(KamprIcons.alsoOpen, glyph, tokens.color.dim)
        KText(tag, tokens.type.micro, tokens.color.mute)
    }
}
