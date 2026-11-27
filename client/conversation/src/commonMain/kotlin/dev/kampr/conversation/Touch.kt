package dev.kampr.conversation

import androidx.compose.runtime.compositionLocalOf
import dev.kampr.shared.ui.TOUCH

// The target every control on the pane is sized to, set by the pane from its own box: the 36 dp
// landscape size only where a rotated phone has no height to spare, 44 dp everywhere else.
val LocalTouch = compositionLocalOf { TOUCH }
