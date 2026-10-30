package dev.kampr.shared.model

import androidx.compose.runtime.Immutable

// Whether anybody can see this app, and `at` counted up every time somebody comes back to it: a
// phone brought to the front, a browser tab shown or its window focused. A hold is somebody
// looking at the pane (ADR 0013), so a view nobody can see holds nothing and a return is when a
// view whose hold was taken asks again.
@Immutable
data class Looking(val shown: Boolean = true, val at: Int = 0) {
    fun turned(shown: Boolean) = Looking(shown, if (shown) at + 1 else at)
}
