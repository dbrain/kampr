package dev.kampr.shared

import dev.kampr.shared.ui.AppState
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

// The status strip stands there saying "no lease held — desktop shape untouched", and that sentence
// is false exactly while a pane is held: a held controller overrides whoever is at the desk (#18)
// and leaves their screen wrong without telling them (#298). So the strip has to know.
class HeldPaneTest {
    private fun state() = AppState(CoroutineScope(Dispatchers.Unconfined))

    @Test
    fun nothingIsHeldUntilSomethingIs() {
        assertNull(state().heldPane.value, "a fresh client holds nothing")
    }

    @Test
    fun aHeldPaneIsNamedAndLettingGoClearsIt() {
        val state = state()
        state.holdingPane("01JNODE/w1:p1", true)
        assertEquals("01JNODE/w1:p1", state.heldPane.value)
        state.holdingPane("01JNODE/w1:p1", false)
        assertNull(state.heldPane.value, "letting go has to clear the claim, or the strip lies the other way")
    }

    // One at a time: the panel that starts a hold releases the previous one, and the node refuses a
    // second controller regardless (#21). A stale release from the pane that no longer holds must
    // not clear the one that does.
    @Test
    fun aStaleReleaseDoesNotClearSomebodyElsesHold() {
        val state = state()
        state.holdingPane("01JNODE/w1:p1", true)
        state.holdingPane("01JNODE/w2:p9", true)
        assertEquals("01JNODE/w2:p9", state.heldPane.value, "the newer hold is the live one")
        state.holdingPane("01JNODE/w1:p1", false)
        assertEquals("01JNODE/w2:p9", state.heldPane.value, "the old pane's release is not the new one's")
    }
}
