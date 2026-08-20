package dev.kampr.desktop

import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Window
import androidx.compose.ui.window.application
import androidx.compose.ui.window.rememberWindowState
import dev.kampr.shared.ui.KamprApp

fun main() = application {
    val width = (System.getenv("KAMPR_WIDTH")?.toIntOrNull() ?: 1440).dp
    val height = (System.getenv("KAMPR_HEIGHT")?.toIntOrNull() ?: 900).dp
    Window(
        onCloseRequest = ::exitApplication,
        title = "Kampr",
        state = rememberWindowState(width = width, height = height),
    ) {
        KamprApp()
    }
}
