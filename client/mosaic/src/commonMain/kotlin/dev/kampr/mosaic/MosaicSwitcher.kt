package dev.kampr.mosaic

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.AgentStatus
import dev.kampr.shared.model.Herd
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.model.paneTitle
import dev.kampr.shared.model.statusOf
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.Dot
import dev.kampr.shared.ui.GlyphAction
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.KamprIcons
import dev.kampr.shared.ui.IconGlyph
import dev.kampr.shared.ui.LocalPaneChrome
import dev.kampr.shared.ui.PaneChrome
import dev.kampr.shared.ui.PaneSurfaces
import dev.kampr.shared.ui.edge
import dev.kampr.shared.ui.edgeBottom
import dev.kampr.shared.ui.selectedEdge
import dev.kampr.shared.ui.statusColor
import dev.kampr.shared.util.formatLatency
import kotlin.math.abs

// P4.5.9: a four-way split on a 390 px screen is unreadable, so the phone gets the same set of
// panes one at a time. The strip is the mosaic — swiping it or tapping a chip is the layout.
private val SWITCHER_PORTRAIT = 44.dp
private val SWITCHER_STRIP = 54.dp
private val SWITCHER_LANDSCAPE = 48.dp
private const val SWIPE_PX = 40f

@Composable
fun MosaicSwitcher(
    store: KamprStore,
    mosaic: MosaicState,
    herd: Herd,
    surfaces: PaneSurfaces,
    landscape: Boolean,
    onHerd: () -> Unit,
    onAdd: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val tokens = Kampr.tokens
    val paneId = mosaic.focused
    val chrome: Dp = if (landscape) SWITCHER_LANDSCAPE else SWITCHER_PORTRAIT + SWITCHER_STRIP

    Box(modifier.fillMaxSize().background(tokens.color.surface2)) {
        if (paneId == null) {
            SwitcherEmpty(onAdd)
        } else {
            val info = herd.panes.firstOrNull { it.id == paneId }
            val node = herd.nodes.firstOrNull { it.id == (info?.nodeId ?: paneId.substringBefore('/')) }
            val pane = store.pane(paneId)
            CompositionLocalProvider(LocalPaneChrome provides PaneChrome(chrome)) {
                surfaces.Terminal(pane, info, Modifier.fillMaxSize())
                Column(Modifier.align(Alignment.BottomStart).fillMaxWidth()) {
                    surfaces.KeyRow(pane, compact = landscape, Modifier.fillMaxWidth())
                }
            }
            if (node != null && !node.online) {
                Box(Modifier.fillMaxSize()) { SwitcherUnavailable(node.detail ?: "${node.name} is not connected") }
            }
        }

        Column(Modifier.align(Alignment.TopStart).fillMaxWidth()) {
            if (landscape) {
                Row(
                    Modifier.fillMaxWidth().background(tokens.color.bar).edgeBottom().height(SWITCHER_LANDSCAPE),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    IconGlyph(
                        KamprIcons.chevronLeft, 17.dp, tokens.color.dim,
                        Modifier.padding(horizontal = 14.dp).clickable(onClick = onHerd),
                    )
                    Strip(mosaic, herd, Modifier.weight(1f))
                    Trailing(mosaic, herd, onAdd)
                }
            } else {
                Row(
                    Modifier.fillMaxWidth().background(tokens.color.bar).height(SWITCHER_PORTRAIT),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    IconGlyph(
                        KamprIcons.chevronLeft, 17.dp, tokens.color.dim,
                        Modifier.padding(horizontal = 14.dp).clickable(onClick = onHerd),
                    )
                    val info = paneId?.let { id -> herd.panes.firstOrNull { it.id == id } }
                    KText(
                        info?.let(::paneTitle) ?: "Mosaic",
                        tokens.type.paneTitle,
                        tokens.color.text,
                        Modifier.weight(1f),
                    )
                    Trailing(mosaic, herd, onAdd)
                }
                Row(
                    Modifier.fillMaxWidth().background(tokens.color.bar).edgeBottom().height(SWITCHER_STRIP),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Strip(mosaic, herd, Modifier.weight(1f))
                }
            }
        }
    }
}

@Composable
private fun Trailing(mosaic: MosaicState, herd: Herd, onAdd: () -> Unit) {
    val tokens = Kampr.tokens
    val paneId = mosaic.focused
    val info = paneId?.let { id -> herd.panes.firstOrNull { it.id == id } }
    val node = herd.nodes.firstOrNull { it.id == info?.nodeId }
    Row(
        Modifier.padding(end = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        node?.rttMs?.let { KText(formatLatency(it), tokens.type.meta, tokens.color.mute) }
        if (paneId != null) {
            GlyphAction(KamprIcons.cross, tokens.color.mute, 34.dp, chip = 22.dp) { mosaic.remove(paneId) }
        }
        GlyphAction(KamprIcons.plus, tokens.color.accent, 34.dp, chip = 22.dp, onClick = onAdd)
    }
}

@Composable
private fun Strip(mosaic: MosaicState, herd: Herd, modifier: Modifier = Modifier) {
    val tokens = Kampr.tokens
    val scroll = rememberScrollState()
    Row(
        modifier
            .horizontalScroll(scroll)
            .pointerInput(mosaic.panes) {
                var travel = 0f
                detectHorizontalDragGestures(
                    onDragStart = { travel = 0f },
                    onDragEnd = { if (abs(travel) > SWIPE_PX) mosaic.step(if (travel < 0) 1 else -1) },
                ) { _, delta -> travel += delta }
            }
            .padding(horizontal = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(7.dp),
    ) {
        for (paneId in mosaic.panes) {
            val info = herd.panes.firstOrNull { it.id == paneId }
            val node = herd.nodes.firstOrNull { it.id == info?.nodeId }
            val status = info?.let(::statusOf) ?: AgentStatus.Unknown
            val active = mosaic.focused == paneId
            val shape = RoundedCornerShape(tokens.radii.sm)
            Row(
                Modifier
                    .background(if (active) tokens.color.raise else tokens.color.surface, shape)
                    .edge(if (active) selectedEdge() else tokens.card, shape)
                    .clickable { mosaic.focus(paneId) }
                    .padding(horizontal = 11.dp, vertical = 6.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(7.dp),
            ) {
                Dot(
                    if (node?.online == false) tokens.color.blocked else statusColor(status),
                    6.dp,
                    hollow = status == AgentStatus.Idle || status == AgentStatus.Unknown,
                )
                Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
                    KText(
                        info?.let(::paneTitle) ?: paneId,
                        tokens.type.captionSmall.copy(fontWeight = androidx.compose.ui.text.font.FontWeight.W700),
                        if (active) tokens.color.text else tokens.color.dim,
                    )
                    KText(node?.name ?: "", tokens.type.metaSmall, tokens.color.mute)
                }
            }
        }
    }
}

@Composable
private fun SwitcherEmpty(onAdd: () -> Unit) {
    val tokens = Kampr.tokens
    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        Column(
            Modifier.padding(horizontal = 28.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(9.dp),
        ) {
            KText("Nothing in the mosaic yet", tokens.type.paneTitle, tokens.color.dim)
            KText(
                "Add up to $MAX_CELLS panes and switch between them here.",
                tokens.type.caption,
                tokens.color.mute,
            )
            GlyphAction(KamprIcons.plus, tokens.color.accent, 44.dp, onClick = onAdd)
        }
    }
}

@Composable
private fun SwitcherUnavailable(detail: String) {
    val tokens = Kampr.tokens
    Box(
        Modifier.fillMaxSize().background(tokens.color.bg.copy(alpha = 0.78f)),
        contentAlignment = Alignment.Center,
    ) {
        Column(
            Modifier.padding(horizontal = 24.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            KText("Unavailable", tokens.type.paneTitle, tokens.color.blocked)
            KText(detail, tokens.type.caption, tokens.color.dim, maxLines = 3)
            KText("recovers on its own", tokens.type.micro, tokens.color.mute)
        }
    }
}
