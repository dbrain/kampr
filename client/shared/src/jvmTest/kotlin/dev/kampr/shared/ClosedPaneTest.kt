package dev.kampr.shared

import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.platform.MemoryPrefs
import dev.kampr.shared.ui.AppState
import dev.kampr.shared.ui.Screen
import dev.kampr.shared.wire.Wire
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlin.test.Test
import kotlin.test.assertEquals

private const val SHELL = "01JNODE/w1:p1"
private const val OTHER = "01JNODE/w2:p1"

private fun pane(id: String) = """{"id":"$id","node_id":"01JNODE","agent_status":"unknown","updated_at":"7"}"""

private fun herd(vararg panes: String, nodes: String = """{"id":"01JNODE","name":"box","kind":"local"}""") = """{"t":"herd","nodes":[$nodes],"panes":[${panes.joinToString(",") { pane(it) }}]}"""

// The report: "quitting a pane (exit or ctrl+d) keeps the pane visible with a not found error
// instead of falling back to pane list". On a phone the pane is the whole screen, so a shell the
// operator ended leaves them staring at its corpse with a back press still to make.
class ClosedPaneTest {
    private fun app(compact: Boolean, body: (AppState, KamprStore) -> Unit) {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Unconfined)
        val store = KamprStore()
        val state = AppState(scope, store, MemoryPrefs(), null).apply {
            this.compact = compact
            go(Screen.Herd)
        }
        try {
            body(state, store)
        } finally {
            scope.cancel()
        }
    }

    private fun KamprStore.take(frame: String) = accept(Wire.decode(frame) ?: error("undecodable: $frame"))

    @Test
    fun aPhoneGoesBackToTheHerdWhenTheShellItIsShowingExits() = app(compact = true) { state, store ->
        store.take(herd(SHELL, OTHER))
        state.openPane(SHELL)
        assertEquals(SHELL, (state.screen as Screen.Pane).paneId)

        store.take(herd(OTHER))
        assertEquals(Screen.Herd, state.screen)
    }

    // The node leaving is not the shell ending: the machine may come back with the pane still on it,
    // and the "Node gone" strip says so.
    @Test
    fun aNodeThatLeftKeepsThePaneOnScreen() = app(compact = true) { state, store ->
        store.take(herd(SHELL))
        state.openPane(SHELL)
        store.take(herd(nodes = """{"id":"01JOTHER","name":"else","kind":"local"}"""))
        assertEquals(SHELL, (state.screen as Screen.Pane).paneId)
    }

    // A herdr that went down takes every pane off the herd while its node stays, which `gone` reads
    // as each shell closing. The panes are still there when it comes back.
    @Test
    fun aHerdrThatWentDownIsNotTheShellExiting() = app(compact = true) { state, store ->
        store.take(herd(SHELL))
        state.openPane(SHELL)
        store.take(herd(nodes = """{"id":"01JNODE","name":"box","kind":"local","online":false}"""))
        assertEquals(SHELL, (state.screen as Screen.Pane).paneId)
    }

    // A pane opened before the herd has it (a deep link, a peer still joining) has not closed.
    @Test
    fun aPaneThatHasNotArrivedYetIsNotClosed() = app(compact = true) { state, store ->
        store.take(herd(OTHER))
        state.openPane(SHELL)
        store.take(herd(OTHER))
        assertEquals(SHELL, (state.screen as Screen.Pane).paneId)
    }

    // A desk has the herd beside the pane already, and the last thing the shell printed — its exit
    // status, its stack trace — is worth keeping on screen there.
    @Test
    fun aDesktopKeepsTheClosedPaneWithItsLastOutput() = app(compact = false) { state, store ->
        store.take(herd(SHELL, OTHER))
        state.openPane(SHELL)
        store.take(herd(OTHER))
        assertEquals(SHELL, (state.screen as Screen.Pane).paneId)
    }
}
