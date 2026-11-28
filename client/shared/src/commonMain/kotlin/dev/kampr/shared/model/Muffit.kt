package dev.kampr.shared.model

import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.hasTerminal

val PaneInfo?.paints: Boolean get() = this?.hasTerminal != false
