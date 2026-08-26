package dev.kampr.shared.ui

import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

// Measured, not chosen. At 520 dp a pane card has its badge, its title, the longest cwd anybody
// types, the watchers tag and the time all reaching their own edges with nothing left over; at 600
// a hole opens between the path and the clock and every dp after that widens it. It is the same
// number a settings card wants for a readable measure of prose, arrived at from the other end.
val COLUMN_MAX = 520.dp

// The same number from the other end, and measured the same way: a theme card rendered at a ladder
// of widths with the real font files, watching what stops working first. Below 295 dp the longest
// credit — `Instrument Sans · JetBrains Mono · 14px radii` — takes a second line, and it is the
// line that says what a theme *is*. The card survives down to 195 dp before anything is actually
// ellipsised, but that floor is where the content stops fitting rather than where it stops working.
val THEME_COLUMN_MIN = 295.dp

// Four is a board, five is a wall. Nothing laid out this way is a sequence read downwards — the
// columns are grouped by machine and scanned — so the count that stops helping is the one past a
// glance.
private const val COLUMN_LIMIT = 4

data class ColumnPlan(val count: Int, val width: Dp)

fun columnWidth(available: Dp, gap: Dp, count: Int): Dp =
    minOf(COLUMN_MAX, (available - gap * (count - 1)) / count)

// `wanted` caps the count at what there is to put in the columns: an empty column is the same
// canyon this exists to close.
// `min` is the width a column stops working below, and defaults to `COLUMN_MAX` because a pane
// card has only the one measure: it wants 520 or it wants a column to itself, and there is no
// useful width in between. A theme card has both ends, so it passes its own.
fun columnPlan(
    available: Dp,
    gap: Dp,
    wanted: Int,
    limit: Int = COLUMN_LIMIT,
    min: Dp = COLUMN_MAX,
): ColumnPlan {
    val ceiling = minOf(wanted, limit).coerceAtLeast(1)
    val count = ((available + gap) / (min + gap)).toInt().coerceIn(1, ceiling)
    return ColumnPlan(count, columnWidth(available, gap, count))
}
