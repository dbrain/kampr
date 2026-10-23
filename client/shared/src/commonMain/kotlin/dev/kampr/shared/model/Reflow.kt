package dev.kampr.shared.model

import dev.kampr.shared.wire.PaneInfo

// The node clamps a reflow to this; below it a view is a thumbnail, not something to read.
private const val MIN_REFLOW_COLS = 10

// The width to ask a native pane to be re-laid at, or none for its own grid (docs/17 §3). A phone
// re-lays a shell narrower than itself by default; an agent wraps its own lines at the PTY's width
// (#588), so it gets the pane's grid unless the operator asks. A mosaic cell is a thumbnail.
fun reflowCols(info: PaneInfo?, viewCols: Int, deskSized: Boolean, mosaic: Boolean, asked: Boolean?): Int? {
    if (info == null || !info.isNative || mosaic) return null
    val paneCols = info.cols ?: return null
    if (viewCols < MIN_REFLOW_COLS || (viewCols >= paneCols && asked != true)) return null
    val wanted = asked ?: (!deskSized && info.agent == null)
    return viewCols.takeIf { wanted }
}

fun reflowNote(served: String?, viewCols: Int, paneCols: Int): String {
    val own = "Showing the pane's own $paneCols columns"
    return when (served) {
        "on" -> "Re-laid at $viewCols columns for this screen; the pane is still $paneCols."
        "alternate" -> "$own: a full-screen program has the screen."
        "harness" -> "$own: an agent lays out its own lines."
        "unsupported" -> "$own: this pane can't be re-laid."
        else -> "$own: this node can't re-lay a pane."
    }
}
