package dev.kampr.shared.ui

import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

enum class Breakpoint { Desktop, Portrait, Landscape }

private val DESKTOP_MIN_WIDTH = 900.dp

// Width alone is not enough: a large phone in landscape is ~914 dp wide but only ~411 dp tall,
// and it wants the landscape layout, not the desktop sidebar.
private val DESKTOP_MIN_HEIGHT = 600.dp

fun breakpointOf(width: Dp, height: Dp): Breakpoint = when {
    width >= DESKTOP_MIN_WIDTH && height >= DESKTOP_MIN_HEIGHT -> Breakpoint.Desktop
    width > height -> Breakpoint.Landscape
    else -> Breakpoint.Portrait
}
