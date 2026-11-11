package dev.kampr.shared.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
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
import androidx.compose.ui.unit.IntRect
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Popup
import androidx.compose.ui.window.PopupPositionProvider
import androidx.compose.ui.window.PopupProperties
import dev.kampr.shared.model.Area
import dev.kampr.shared.model.NodeGroup
import dev.kampr.shared.model.areas
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.wire.NodeInfo

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
            val spoken = areaSpoken(area, count)
            Box(
                Modifier
                    .height(ctx.look.control)
                    .action(spoken, { ctx.onArea(group.node.id, area) }, shape, role = Role.Tab, selected = active),
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
                Hint(spoken)
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

// Everything about a machine that is not the one thing done most, which is the + beside this. A
// pointer gets a menu hanging off the glyph; a finger gets the same rows as a sheet.
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
            val rows: @Composable ColumnScope.() -> Unit = {
                NodeStatus(node, latency)
                if (manage.browses && node.isReachable) {
                    MenuRow("Browse files", {
                        close()
                        manage.browse(node)
                    }, KamprIcons.folder, label = "Browse files on ${node.name}")
                }
                if (manage.enabled && node.isReachable) {
                    MenuRow("New with options…", {
                        close()
                        manage.newOn(node.id)
                    }, KamprIcons.plus, label = "New on ${node.name}, with every option")
                }
            }
            if (ctx.look == HerdLook.Sidebar) {
                val below = with(LocalDensity.current) { ctx.look.control.roundToPx() }
                Popup(
                    alignment = Alignment.TopEnd,
                    offset = IntOffset(0, below),
                    onDismissRequest = close,
                    properties = PopupProperties(focusable = true),
                ) {
                    MenuPanel(content = rows)
                }
            } else {
                Popup(popupPositionProvider = WholeWindow, onDismissRequest = close, properties = PopupProperties(focusable = true)) {
                    val breakpoint = if (ctx.look == HerdLook.Portrait) Breakpoint.Portrait else Breakpoint.Landscape
                    BottomSheet(breakpoint, close) {
                        SheetHeader(node.name, null, null, close, breakpoint != Breakpoint.Portrait, dismiss = "Close the menu")
                        Column(Modifier.padding(bottom = 12.dp), content = rows)
                    }
                }
            }
        }
    }
}

// The sheet covers the window rather than hanging off the glyph that opened it.
private object WholeWindow : PopupPositionProvider {
    override fun calculatePosition(anchorBounds: IntRect, windowSize: IntSize, layoutDirection: LayoutDirection, popupContentSize: IntSize): IntOffset = IntOffset.Zero
}

@Composable
private fun NodeStatus(node: NodeInfo, latency: String) {
    val tokens = Kampr.tokens
    val serving = node.isServing
    val tone = if (serving) tokens.color.done else tokens.color.blocked
    val status = if (serving) "Online" else "Offline"
    Row(
        Modifier.padding(horizontal = 20.dp, vertical = 6.dp).named("${node.name}, ${status.lowercase()}, ${nodeReach(node)}, $latency"),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(7.dp),
    ) {
        Mark(tone, if (serving) MarkShape.Bar else MarkShape.Ring, 8.dp)
        KText(status, tokens.type.captionSmall, tone)
        KText(listOf(nodeReach(node), latency).joinToString(" · "), tokens.type.captionSmall, tokens.color.mute)
    }
}
