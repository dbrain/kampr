package dev.kampr.terminal.input

import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.ui.unit.dp
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.terminal.PaneSession

// The system puts a soft keyboard away on its own — Back, or the keyboard's own hide key — and says
// so only by handing its inset back. A pane that is not told goes on believing the keyboard is up,
// and the end of every gesture then asks for it again (`reclaimKeyboard`), so a scroll raised it.
// Only the fall counts: a desk or a hardware keyboard never has an inset to lose.
@Composable
fun ImeDismissal(session: PaneSession) {
    val up = LocalSafeArea.current.ime > 0.dp
    val wasUp = remember { mutableStateOf(false) }
    LaunchedEffect(up) {
        if (wasUp.value && !up) session.closeKeyboard()
        wasUp.value = up
    }
}
