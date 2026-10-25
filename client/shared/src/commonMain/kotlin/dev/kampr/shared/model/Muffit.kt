package dev.kampr.shared.model

import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.isMuffit

// A muffit session has no PTY behind it (docs/18): its conversation is the whole pane, so nothing
// that reads, sizes or waits on a grid applies to it.
val PaneInfo?.paints: Boolean get() = this?.isMuffit != true
