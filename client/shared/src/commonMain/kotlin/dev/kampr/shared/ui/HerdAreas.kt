package dev.kampr.shared.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.DisableSelection
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Popup
import androidx.compose.ui.window.PopupProperties
import dev.kampr.shared.model.Area
import dev.kampr.shared.model.NodeGroup
import dev.kampr.shared.model.areas
import dev.kampr.shared.theme.Kampr

private val TAB_INK = 22.dp

fun shortCount(n: Int): String = if (n > 99) "9+" else n.toString()

private fun NodeGroup.count(area: Area): Int? = when (area) {
    Area.Terminals -> null
    Area.Containers -> node.containers.count { it.state == "running" }
    Area.Muffit -> background.size
}

private fun areaIcon(area: Area): Icon = when (area) {
    Area.Terminals -> KamprIcons.shell
    Area.Containers -> KamprIcons.docker
    Area.Muffit -> KamprIcons.muffit
}

private fun areaSpoken(area: Area, count: Int?): String = when (area) {
    Area.Terminals -> "Terminals"
    Area.Containers -> "Containers, ${count ?: 0} running"
    Area.Muffit -> "Muffit sessions, ${count ?: 0}"
}

// Which of a host's lists is under it. Nothing is drawn for a host that only has terminals: a
// switch with one position is a label.
@Composable
fun AreaTabs(group: NodeGroup, ctx: HerdRowsCtx) {
    val areas = group.areas()
    if (areas.size < 2) return
    val tokens = Kampr.tokens
    val shown = ctx.folds.area(group)
    val ground = tokens.color.surface
    val line = tokens.card
    Row(
        Modifier
            .height(ctx.look.control)
            .group()
            .drawBehind {
                val ink = TAB_INK.toPx() + 4.dp.toPx()
                val top = (size.height - ink) / 2
                val radius = CornerRadius(ink / 2)
                drawRoundRect(ground, Offset(0f, top), Size(size.width, ink), radius)
                if (line.visible) drawRoundRect(line.color, Offset(0f, top), Size(size.width, ink), radius, style = Stroke(line.width.toPx()))
            }
            .padding(horizontal = 2.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        for (area in areas) {
            val active = area == shown
            val count = group.count(area)
            val shape = RoundedCornerShape(TAB_INK / 2)
            Box(
                Modifier
                    .height(ctx.look.control)
                    .action(areaSpoken(area, count), { ctx.onArea(group.node.id, area) }, shape, role = Role.Tab, selected = active),
                contentAlignment = Alignment.Center,
            ) {
                Row(
                    Modifier
                        .height(TAB_INK)
                        .let { if (active) it.background(tokens.color.raise, shape) else it }
                        .padding(horizontal = 7.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(3.dp),
                ) {
                    val tint = if (active) tokens.color.text else tokens.color.mute
                    IconGlyph(areaIcon(area), 13.dp, tint)
                    if (count != null && count > 0) DisableSelection { TwoCells(shortCount(count), tint) }
                }
            }
        }
    }
}

// Sized by the widest thing it can say, so a count ticking from 9 to 10 does not shift the switch.
@Composable
private fun TwoCells(text: String, tint: Color) {
    val style = Kampr.tokens.type.micro
    val measurer = rememberTextMeasurer()
    val density = LocalDensity.current
    val widest = remember(style, density) {
        with(density) { listOf("99", "9+").maxOf { measurer.measure(it, style).size.width }.toDp() }
    }
    Box(Modifier.widthIn(min = widest)) { KText(text, style, tint) }
}

// Everything about a machine that is not the one thing done most, which is the + beside this.
@Composable
fun NodeOverflow(group: NodeGroup, ctx: HerdRowsCtx) {
    val tokens = Kampr.tokens
    val node = group.node
    val manage = LocalManage.current
    val keystrokes = LocalKeystrokeMs.current
    val latency = nodeLatency(node, keystrokes, node.rttMs ?: ctx.localRtt)
    var open by remember(node.id) { mutableStateOf(false) }
    Box {
        GlyphAction(KamprIcons.ellipsis, "More for ${node.name}", tokens.color.dim, ctx.look.control, chip = 22.dp) { open = true }
        if (open) {
            val close = { open = false }
            val below = with(androidx.compose.ui.platform.LocalDensity.current) { ctx.look.control.roundToPx() }
            Popup(
                alignment = Alignment.TopEnd,
                offset = IntOffset(0, below),
                onDismissRequest = close,
                properties = PopupProperties(focusable = true),
            ) {
                val shape = RoundedCornerShape(tokens.radii.sm)
                Column(
                    Modifier
                        .width(OVERFLOW_WIDTH)
                        .background(tokens.color.bar, shape)
                        .edge(tokens.chrome, shape)
                        .padding(vertical = 5.dp),
                ) {
                    KText(
                        listOf(nodeReach(node), latency).joinToString(" · "),
                        tokens.type.captionSmall,
                        tokens.color.mute,
                        Modifier.padding(horizontal = 14.dp, vertical = 6.dp).named("${node.name}, ${nodeReach(node)}, $latency"),
                    )
                    if (manage.browses && node.isReachable) {
                        OverflowItem("Browse files", "Browse files on ${node.name}") {
                            close()
                            manage.browse(node)
                        }
                    }
                    if (manage.enabled && node.isReachable) {
                        OverflowItem("New…", "New on ${node.name}, with every option") {
                            close()
                            manage.newOn(node.id)
                        }
                    }
                }
            }
        }
    }
}

private val OVERFLOW_WIDTH: Dp = 200.dp

@Composable
private fun OverflowItem(text: String, label: String, onClick: () -> Unit) {
    val tokens = Kampr.tokens
    Box(
        Modifier
            .touchable(LANDSCAPE_TOUCH)
            .action(label, onClick)
            .padding(horizontal = 14.dp, vertical = 8.dp),
        contentAlignment = Alignment.CenterStart,
    ) {
        DisableSelection { KText(text, tokens.type.body, tokens.color.text) }
    }
}
