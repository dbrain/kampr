package dev.kampr.conversation

import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

// A box inside a reply is as wide as what is in it.
//
// The turn frame takes the whole column and keeps it: that is the reader's place on the screen and
// moving it about would be the change nobody asked for. Nothing *inside* the frame earns the same
// width by being inside it. A tool card reading "Read · Palette.kt · 12 lines" stretched across a
// desktop column is a sentence with a chevron a hand's width away from it; a two-column table
// stretched the same way is a table with a hole in the middle. Claude Code's own output in a
// terminal is the reference and it is the obvious one: the box is the width of the widest thing in
// the box.
//
// `IntrinsicSize.Max` rather than a measured guess, because the width wanted here is precisely the
// one the content would take if nothing constrained it. It is clamped from below so that a card
// carrying two short words is still a card, and from above by the incoming constraint — content
// wider than the frame keeps its own horizontal scroller and the transcript never moves sideways.
//
// **Not over a subtree holding a `MarkdownTable`.** That is a `BoxWithConstraints`, and a
// subcompose layout has no intrinsics to answer with. A table sizes itself against the width it is
// handed, which arrives at the same place from the other end.
val FIT_MIN = 260.dp

fun Modifier.fitContent(min: Dp = FIT_MIN): Modifier = widthIn(min = min).width(IntrinsicSize.Max)
