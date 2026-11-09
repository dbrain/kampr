package dev.kampr.shared.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.Area
import dev.kampr.shared.model.Fold
import dev.kampr.shared.model.Folds
import dev.kampr.shared.model.NodeGroup
import dev.kampr.shared.model.sections
import dev.kampr.shared.model.strayChildren
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.wire.PaneInfo

// How one machine's list is drawn on each of the three herd layouts. The sidebar's rows are the
// compact ones and know which pane is open beside them; its controls are a pointer's.
enum class HerdLook(val rows: Boolean, val gap: Dp, val inset: Dp, val headingTop: Dp, val headingBottom: Dp, val control: Dp) {
    Portrait(false, 8.dp, 6.dp, 6.dp, 0.dp, LANDSCAPE_TOUCH),
    Landscape(false, 6.dp, 8.dp, 6.dp, 0.dp, LANDSCAPE_TOUCH),
    Sidebar(true, 2.dp, 8.dp, 8.dp, 2.dp, POINTER_TOUCH),
}

typealias OnFold = (node: String, fold: Fold, open: Boolean) -> Unit

val NO_FOLD: OnFold = { _, _, _ -> }

typealias OnArea = (node: String, area: Area) -> Unit

val NO_AREA: OnArea = { _, _ -> }

// Everything a herd row needs that is not the machine itself, passed down as one value so the
// nesting below a container does not grow a parameter per level.
class HerdRowsCtx(
    val look: HerdLook,
    val now: Double,
    val localRtt: Double?,
    val folds: Folds,
    val onFold: OnFold,
    val onOpenPane: (String) -> Unit,
    val activePaneId: String? = null,
    val onArea: OnArea = NO_AREA,
)

@Composable
fun GroupBody(group: NodeGroup, ctx: HerdRowsCtx, modifier: Modifier = Modifier) {
    val look = ctx.look
    val tokens = Kampr.tokens
    Column(modifier, verticalArrangement = Arrangement.spacedBy(look.gap)) {
        when (ctx.folds.area(group)) {
            Area.Terminals -> {
                if (group.panes.isEmpty() && group.node.containers.isEmpty() && group.children.isEmpty()) {
                    NodeQuiet(group.node, Modifier.padding(start = look.inset, bottom = 4.dp))
                } else if (group.terminals.isEmpty()) {
                    KText("no terminals here", tokens.type.captionSmall, tokens.color.mute, Modifier.padding(start = look.inset, bottom = 2.dp))
                }
                for (section in group.sections) {
                    section.heading?.let {
                        WorkspaceHeading(it, Modifier.padding(start = look.inset, top = look.headingTop, bottom = look.headingBottom))
                    }
                    for (pane in section.panes) PaneItem(pane, ctx)
                }
            }

            Area.Containers -> ContainerSection(group, ctx)

            Area.Muffit -> for (pane in group.background) PaneItem(pane, ctx)
        }
        for (child in group.strayChildren) Nested(look.gap) { ChildNode(child, ctx) }
    }
}

@Composable
fun PaneItem(pane: PaneInfo, ctx: HerdRowsCtx) {
    if (ctx.look.rows) {
        PaneRow(pane, ctx.now, pane.id == ctx.activePaneId) { ctx.onOpenPane(pane.id) }
    } else {
        PaneCard(pane, ctx.now, { ctx.onOpenPane(pane.id) }, Modifier.fillMaxWidth())
    }
}

@Composable
fun ChildNode(child: NodeGroup, ctx: HerdRowsCtx) {
    Column(verticalArrangement = Arrangement.spacedBy(ctx.look.gap)) {
        NodeHeader(child, ctx, PaddingValues(start = ctx.look.inset, end = 0.dp, top = 4.dp, bottom = 2.dp))
        GroupBody(child, ctx)
    }
}

// One level down: 8 dp and a hairline, never more, so a nested title keeps its width.
@Composable
fun Nested(gap: Dp, content: @Composable ColumnScope.() -> Unit) {
    val line = Kampr.tokens.color.line
    Column(
        Modifier
            .fillMaxWidth()
            .drawBehind { drawLine(line, Offset(0.5f, 0f), Offset(0.5f, size.height), 1f) }
            .padding(start = NEST),
        verticalArrangement = Arrangement.spacedBy(gap),
        content = content,
    )
}

val NEST: Dp = 8.dp
