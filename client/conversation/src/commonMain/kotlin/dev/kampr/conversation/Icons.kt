package dev.kampr.conversation

import dev.kampr.shared.ui.Glyph
import dev.kampr.shared.ui.Icon

private fun trace(vararg d: String) = d.map { Glyph.Trace(it) }

object ConversationIcons {
    val copy = Icon(
        14f, 1.5f,
        listOf(Glyph.Frame(4f, 4f, 9f, 9f, 1.6f)) +
            trace("M10 4V2.6A1.6 1.6 0 0 0 8.4 1H2.6A1.6 1.6 0 0 0 1 2.6v5.8A1.6 1.6 0 0 0 2.6 10H4"),
    )
    val search = Icon(18f, 1.7f, listOf(Glyph.Round(7.6f, 7.6f, 5.4f)) + trace("M11.6 11.6 16 16"))
    val close = Icon(14f, 1.7f, trace("M3 3l8 8M11 3l-8 8"))
    val send = Icon(20f, 1.9f, trace("M10 16V4M4.8 9.2 10 4l5.2 5.2"))
    val chevronDown = Icon(14f, 1.8f, trace("M2.5 4.5 7 9l4.5-4.5"))
    val chevronUp = Icon(14f, 1.8f, trace("M2.5 9.5 7 5l4.5 4.5"))
    val up = Icon(14f, 1.8f, trace("M7 11V3M3.5 6.5 7 3l3.5 3.5"))
    val down = Icon(14f, 1.8f, trace("M7 3v8M3.5 7.5 7 11l3.5-3.5"))
    val history = Icon(
        16f, 1.6f,
        listOf(Glyph.Round(8f, 8f, 6f)) + trace("M8 4.6V8l2.4 1.6"),
    )
    val image = Icon(
        16f, 1.6f,
        listOf(Glyph.Frame(1.5f, 2.5f, 13f, 11f, 2f), Glyph.Round(5.4f, 6.2f, 1.4f)) +
            trace("M2.2 12.4 6.2 8.6l2.4 2.2 2.2-1.8 2.6 2.4"),
    )
    val film = Icon(
        16f, 1.6f,
        listOf(Glyph.Frame(1.5f, 3f, 13f, 10f, 1.8f)) +
            trace("M4.4 3v10M11.6 3v10M1.5 8h13"),
    )
    val download = Icon(16f, 1.6f, trace("M8 2.2v7.6M4.8 6.8 8 10.2l3.2-3.4", "M2.6 13.2h10.8"))
    val diff = Icon(16f, 1.6f, trace("M3 5h10M3 11h10M8 2.6v4.8", "M5.4 11H10.6"))
    val speech = Icon(
        18f, 1.6f,
        listOf(Glyph.Frame(2f, 3f, 14f, 10f, 2.4f)) + trace("M6 16l1.6-3"),
    )
}
