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
    // An arrow onto a floor, not a bare chevron: the bar's other two arrows step between search
    // matches, and this one goes all the way to the end.
    val toEnd = Icon(14f, 1.8f, trace("M7 2.4v6.6", "M3.9 5.9 7 9 10.1 5.9", "M3.2 11.9h7.6"))
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
    // A cone and two arcs. The waves are what tells it apart from the play triangle beside it at
    // 13 dp, where a bare speaker body and a bare triangle are the same wedge.
    val sound = Icon(
        16f, 1.6f,
        trace("M3.4 6.2h2.2L8.6 3.4v9.2L5.6 9.8H3.4Z", "M10.8 6.2a2.6 2.6 0 0 1 0 3.6", "M12.6 4.4a5.2 5.2 0 0 1 0 7.2"),
    )
    val play = Icon(16f, 1.6f, trace("M5.4 3.2 12.2 8l-6.8 4.8Z"))
    val pause = Icon(16f, 1.8f, trace("M6 3.4v9.2M10 3.4v9.2"))
    val diff = Icon(16f, 1.6f, trace("M3 5h10M3 11h10M8 2.6v4.8", "M5.4 11H10.6"))
    // A conversation this one launched: a line that leaves the trunk and an arrow onto its own
    // thread.
    val branch = Icon(
        16f, 1.6f,
        trace("M4 2.6v6.4a2.4 2.4 0 0 0 2.4 2.4h5.2", "M9.4 9 12.4 11.4 9.4 13.8"),
    )
    val file = Icon(
        16f, 1.6f,
        listOf(Glyph.Frame(3f, 1.8f, 10f, 12.4f, 1.6f)) + trace("M5.6 5.6h4.8M5.6 8h4.8M5.6 10.4h3"),
    )
    // The clip an operator looks for to hand something over, and nothing else on this bar is a
    // curve.
    val attach = Icon(
        18f, 1.6f,
        trace("M12.6 5.2 6.4 11.4a2.2 2.2 0 0 0 3.1 3.1l6.2-6.2a3.9 3.9 0 0 0-5.5-5.5L3.9 9.1"),
    )
    val speech = Icon(
        18f, 1.6f,
        listOf(Glyph.Frame(2f, 3f, 14f, 10f, 2.4f)) + trace("M6 16l1.6-3"),
    )
}
