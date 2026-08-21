package dev.kampr.shared.ui

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.theme.Scan
import dev.kampr.shared.util.qrEncode

private val BOX = 148.dp
private const val QUIET = 3

// Dark on white whatever the theme is doing. A camera reads contrast, not a palette, and an
// inverted symbol is one many scanners refuse outright.
@Composable
fun PairingQr(url: String, pairing: Boolean, modifier: Modifier = Modifier) {
    val tokens = Kampr.tokens
    val code = remember(url) { qrEncode(url) } ?: return
    val spoken = if (pairing) "Scan to pair: $url" else "Scan to open: $url"
    Row(
        modifier,
        horizontalArrangement = Arrangement.spacedBy(13.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Canvas(
            Modifier
                .size(BOX)
                .named(spoken)
                .background(Scan.paper, RoundedCornerShape(tokens.radii.sm))
                .padding(8.dp),
        ) {
            val step = size.minDimension / (code.size + QUIET * 2)
            for (y in 0 until code.size) {
                for (x in 0 until code.size) {
                    if (!code.dark(x, y)) continue
                    drawRect(
                        color = Scan.ink,
                        topLeft = Offset((x + QUIET) * step, (y + QUIET) * step),
                        size = Size(step, step),
                    )
                }
            }
        }
        Column(verticalArrangement = Arrangement.spacedBy(5.dp)) {
            KText(
                if (pairing) "Scan it with the phone you are adding" else "Scan it to open Kampr there",
                tokens.type.captionSmall,
                tokens.color.text,
                maxLines = 2,
            )
            KText(
                if (pairing) "The code rides along, so the phone only has to say yes."
                else "Ask for a pairing code first and the code rides along too.",
                tokens.type.captionSmall,
                tokens.color.mute,
                maxLines = 3,
            )
        }
    }
}
