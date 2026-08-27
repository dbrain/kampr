package dev.kampr.shared.util

// Per instant, not per device: a zone's offset moves twice a year and a transcript routinely
// spans the move. Asking for the offset *now* and applying it to a message written in August
// draws every one of them an hour out for half the year.
expect fun localOffsetMillis(atMillis: Double): Double
