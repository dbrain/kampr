package dev.kampr.mosaic

import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import dev.kampr.shared.ui.AppState
import dev.kampr.shared.ui.Breakpoint
import dev.kampr.shared.ui.MosaicHost
import dev.kampr.shared.ui.PaneSurfaces

// The arrangement outlives the screen — leaving the mosaic drops the streams, not the layout.
class MosaicSurfaces : MosaicHost {
    private var held: MosaicState? = null

    private fun stateFor(app: AppState): MosaicState =
        held ?: MosaicState(app.prefs, app.connection).also {
            it.restore()
            held = it
        }

    @Composable
    override fun Mosaic(
        state: AppState,
        breakpoint: Breakpoint,
        surfaces: PaneSurfaces,
        modifier: Modifier,
    ) {
        val mosaic = remember(state) { stateFor(state) }
        val herd by state.store.herd.collectAsState()
        val connectionStatus by state.store.status.collectAsState()
        val hello by state.store.hello.collectAsState()
        var picking by remember { mutableStateOf(false) }

        DisposableEffect(mosaic) {
            mosaic.attach()
            onDispose { mosaic.detach() }
        }
        LaunchedEffect(herd) { mosaic.reconcile(herd) }

        if (breakpoint == Breakpoint.Desktop) {
            MosaicScreen(
                store = state.store,
                mosaic = mosaic,
                herd = herd,
                connectionStatus = connectionStatus,
                build = hello?.build,
                surfaces = surfaces,
                onHerd = state::back,
                onAdd = { picking = true },
                modifier = modifier,
            )
        } else {
            MosaicSwitcher(
                store = state.store,
                mosaic = mosaic,
                herd = herd,
                surfaces = surfaces,
                landscape = breakpoint == Breakpoint.Landscape,
                onHerd = state::back,
                onAdd = { picking = true },
                modifier = modifier,
            )
        }

        if (picking) {
            PanePicker(
                herd = herd,
                breakpoint = breakpoint,
                chosen = mosaic.panes,
                full = mosaic.full,
                onPick = {
                    mosaic.add(it)
                    picking = false
                },
                onDismiss = { picking = false },
            )
        }
    }
}
