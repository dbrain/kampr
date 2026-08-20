package dev.kampr.desktop

import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Window
import androidx.compose.ui.window.application
import androidx.compose.ui.window.rememberWindowState
import dev.kampr.shared.ui.KamprApp
import dev.kampr.conversation.ConversationSurfaces
import dev.kampr.mosaic.MosaicSurfaces
import dev.kampr.terminal.TerminalSurfaces
import dev.kampr.terminal.bench.TerminalBenchApp

// ConversationSurfaces wraps: it renders the transcript and delegates the terminal and the
// key row to its base, so both halves of the pane are live.
private val surfaces = ConversationSurfaces(TerminalSurfaces())
private val mosaic = MosaicSurfaces()

fun main() = application {
    val width = (System.getenv("KAMPR_WIDTH")?.toIntOrNull() ?: 1440).dp
    val height = (System.getenv("KAMPR_HEIGHT")?.toIntOrNull() ?: 900).dp
    val bench = System.getenv("KAMPR_BENCH") != null
    Window(
        onCloseRequest = ::exitApplication,
        title = if (bench) "Kampr terminal bench" else "Kampr",
        state = rememberWindowState(width = width, height = height),
    ) {
        if (bench) TerminalBenchApp() else KamprApp(surfaces, mosaic = mosaic)
    }
}
