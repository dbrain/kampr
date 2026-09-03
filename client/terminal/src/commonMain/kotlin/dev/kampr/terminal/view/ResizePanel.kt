package dev.kampr.terminal.view

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.BorderSpec
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.LabelText
import dev.kampr.shared.ui.Surface
import dev.kampr.shared.ui.action
import dev.kampr.shared.ui.edge
import dev.kampr.shared.ui.touchable
import dev.kampr.shared.wire.MIN_PANE_COLS
import dev.kampr.shared.wire.MIN_PANE_ROWS

// The sizes offered as one tap. Wide enough to be worth asking for and safe from any device —
// unlike "fit this to my screen", which on a phone asks for a pane no shell is usable in.
internal val SIZE_PRESETS = listOf(80 to 24, 120 to 40, 200 to 50)

// What the panel needs to know about the pane it is pointed at: the size it is now, the size this
// client could show without cropping, and whether a controller is currently held on it.
data class PaneSizing(
    val cols: Int,
    val rows: Int,
    // The grid this window would show, measured at the base cell rather than at the zoom the
    // operator happens to be reading at — see `viewGrid`. **Both controls below ask for this one
    // number.** Taken at the current zoom instead it is a function of the pane rather than of the
    // window, because the fit ladder picked that zoom to suit the pane's width: on a grid wider
    // than the window the chip offered the pane roughly the width it already had, and where the
    // standing hold was on it named a different size and undid the chip a moment later.
    val viewCols: Int,
    val viewRows: Int,
    val held: Boolean,
    val matching: Boolean = false,
    // False on a view too small to ask, and on a pane Kampr forked for a job of its own.
    val canMatch: Boolean = false,
)

internal fun PaneSizing.fitIsUsable() = viewCols >= MIN_PANE_COLS && viewRows >= MIN_PANE_ROWS

// Kampr's one deliberate exception to never reshaping a pane, and it is behind a panel rather than
// on the surface because of what it costs: claiming a PTY overrides whoever is at the desk for as
// long as it is held (#18), and an attached desk TUI is not told and simply renders wrong (#298).
//
// It exists because a pane can be born unusable and nothing else can reach it — a headless pane's
// PTY is whatever created it, and the already-shipped `pane.zoom` moves a PTY only when a client is
// attached (#265). Zoom cannot help: a 40-column pane really is 40 columns, and magnifying it just
// makes the crop bigger.
@Composable
internal fun ResizePanel(
    sizing: PaneSizing,
    onResize: (Int, Int) -> Unit,
    onHold: (Boolean) -> Unit,
    onMatchView: (Boolean) -> Unit = {},
    modifier: Modifier = Modifier,
) {
    val tokens = Kampr.tokens
    Column(modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Row(
            Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            LabelText("Pane size", tokens.type.sectionLabel, tokens.color.mute)
            KText(
                "now ${sizing.cols}×${sizing.rows}",
                tokens.type.metaSmall,
                tokens.color.mute,
            )
        }

        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(7.dp)) {
            for ((cols, rows) in SIZE_PRESETS) {
                SizeChip(
                    label = "$cols×$rows",
                    active = sizing.cols == cols && sizing.rows == rows,
                    enabled = true,
                    onClick = { onResize(cols, rows) },
                    modifier = Modifier.weight(1f),
                )
            }
        }

        // Offered, and refused out loud when it would do harm. A phone's viewport is narrower than
        // any shell is usable at, and a resize on a headless pane *persists* after the controller
        // goes (#219) — so fitting a pane to a small screen would leave it that narrow for every
        // other client, with nothing but another resize to undo it.
        val fits = sizing.fitIsUsable()
        SizeChip(
            label = if (fits) {
                "Match this view · ${sizing.viewCols}×${sizing.viewRows}"
            } else {
                "This view is only ${sizing.viewCols}×${sizing.viewRows}"
            },
            active = false,
            enabled = fits,
            onClick = { onResize(sizing.viewCols, sizing.viewRows) },
            modifier = Modifier.fillMaxWidth(),
        )
        if (!fits) {
            KText(
                "Too small to give a pane — it would stay this narrow everywhere. " +
                    "The smallest is ${MIN_PANE_COLS}×$MIN_PANE_ROWS.",
                tokens.type.captionSmall,
                tokens.color.mute,
                maxLines = 3,
            )
        }

        // **The switch behind the one automatic claim in the product** (ADR 0013), and the reason
        // that claim is not the write rule 3 forbids: an operator who did not want it can find it
        // and turn it off, and it says what it costs before they leave it on.
        if (sizing.canMatch) {
            Toggle(
                on = sizing.matching,
                title = "Match this view while it's open · ${sizing.viewCols}×${sizing.viewRows}",
                detail = "Holds the pane at this window's size until you leave it. Their screen at " +
                    "the desk is wrong while it is held, and the pane goes back to the size it was " +
                    "when you let go.",
                onChange = onMatchView,
            )
        }

        Toggle(
            on = sizing.held,
            title = "Hold while this panel is open",
            // The whole trade, in the place the decision is made. Off by default: a resize normally
            // hands the PTY straight back, and a pane with a desk attached takes its own geometry
            // back the moment it does (#19). Holding is what makes the size stick there, and the
            // cost is that the desk renders wrong for as long as it is held.
            detail = "Keeps the size on a pane someone has open at their desk. Their view is wrong " +
                "until you close this.",
            onChange = onHold,
        )
    }
}

@Composable
private fun SizeChip(
    label: String,
    active: Boolean,
    enabled: Boolean,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val tokens = Kampr.tokens
    val shape = RoundedCornerShape(tokens.radii.sm)
    Row(
        modifier
            .background(
                when {
                    !enabled -> tokens.color.surface2
                    active -> tokens.color.accentSoft
                    else -> tokens.color.raise
                },
                shape,
            )
            .edge(if (active) BorderSpec(1.dp, tokens.color.accent) else tokens.card, shape)
            .touchable()
            .action(label, onClick, shape, enabled = enabled, selected = active)
            .padding(horizontal = 10.dp, vertical = 7.dp),
        horizontalArrangement = Arrangement.Center,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        KText(
            label,
            tokens.type.buttonSmall,
            if (enabled) tokens.color.text else tokens.color.mute,
        )
    }
}
