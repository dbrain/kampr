package dev.kampr.shared.model

import dev.kampr.shared.wire.PaneInfo

// One blocked agent and, when the node has it, the question it is blocked on. The question is
// optional because it is read off the screen and a harness can be blocked before its dialog has
// finished painting — a row with no question is still a row that needs you.
data class TriageItem(val pane: PaneInfo, val question: String?)
