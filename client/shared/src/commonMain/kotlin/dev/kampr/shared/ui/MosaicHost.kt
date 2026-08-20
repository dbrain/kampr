package dev.kampr.shared.ui

import androidx.compose.runtime.Composable
import androidx.compose.runtime.ProvidableCompositionLocal
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.Dp
import dev.kampr.shared.theme.Kampr

// The mosaic is several renderers at once over the merged herd, so it lives outside shared for
// the same reason the terminal does: shared owns the seam, the module owns the surface.
interface MosaicHost {
    val available: Boolean get() = true

    @Composable
    fun Mosaic(
        state: AppState,
        breakpoint: Breakpoint,
        surfaces: PaneSurfaces,
        modifier: Modifier,
    )
}

object NoMosaic : MosaicHost {
    override val available: Boolean get() = false

    @Composable
    override fun Mosaic(
        state: AppState,
        breakpoint: Breakpoint,
        surfaces: PaneSurfaces,
        modifier: Modifier,
    ) = Unit
}

val LocalMosaic: ProvidableCompositionLocal<(() -> Unit)?> = staticCompositionLocalOf { null }

@Composable
fun MosaicAction(target: Dp = TOUCH, modifier: Modifier = Modifier) {
    val open = LocalMosaic.current ?: return
    GlyphAction(KamprIcons.mosaic, "Mosaic, several panes at once", Kampr.tokens.color.dim, target, modifier, onClick = open)
}
