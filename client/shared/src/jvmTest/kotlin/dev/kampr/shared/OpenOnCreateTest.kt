package dev.kampr.shared

import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.model.createdPane
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
import kotlin.test.assertNull

// The operator, on 0.1.49: *"when i create a new workspace from the session list it just appends to
// the bottom of the list and doesn't open it"*. The ack has carried the created id since
// `managed` existed; nothing did anything with it.
class OpenOnCreateTest {
    private fun app(): Triple<AppState, KamprStore, CoroutineScope> {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Unconfined)
        val store = KamprStore()
        // A state with no endpoint opens on Setup, which is not what this is about.
        val state = AppState(scope, store, MemoryPrefs(), null).apply { go(Screen.Herd) }
        return Triple(state, store, scope)
    }

    private fun KamprStore.take(frame: String) {
        accept(Wire.decode(frame) ?: error("undecodable: $frame"))
    }

    private fun herd(vararg panes: String): String {
        val entries = panes.joinToString(",") { it }
        return """{"t":"herd","nodes":[{"id":"01JNODE","name":"box","kind":"local"}],"panes":[$entries]}"""
    }

    private fun pane(id: String, workspace: String? = null, tab: String? = null): String {
        val ws = workspace?.let { ""","workspace_id":"$it"""" }.orEmpty()
        val tb = tab?.let { ""","tab_id":"$it"""" }.orEmpty()
        return """{"id":"01JNODE/$id","node_id":"01JNODE","updated_at":"7"$ws$tb}"""
    }

    // `workspace.create`'s ack names the workspace, never the pane herdr made inside it, so the id
    // has to be resolved through the patch rather than opened directly.
    @Test
    fun aWorkspaceThatWasJustCreatedIsOpenedAtTheFirstPaneItArrivesWith() {
        val (state, store, scope) = app()
        try {
            store.take(herd(pane("w1:p1", workspace = "01JNODE/w1")))
            assertEquals(Screen.Herd, state.screen)

            state.opening("01JNODE/w2")
            assertEquals(
                Screen.Herd,
                state.screen,
                "the ack lands before the sweep that finds the pane, so there is nothing to open yet",
            )

            store.take(
                herd(
                    pane("w1:p1", workspace = "01JNODE/w1"),
                    pane("w2:p1", workspace = "01JNODE/w2"),
                )
            )
            assertEquals("01JNODE/w2:p1", (state.screen as Screen.Pane).paneId)
        } finally {
            scope.cancel()
        }
    }

    // A patch that is not the one carrying the pane must not spend the intent: the node does not
    // settle a structural op before its ack, so an unrelated sweep can easily land in front.
    @Test
    fun aPatchThatDoesNotCarryItYetDoesNotSpendTheIntent() {
        val (state, store, scope) = app()
        try {
            state.opening("01JNODE/w2")
            store.take(herd(pane("w1:p1", workspace = "01JNODE/w1")))
            assertEquals(Screen.Herd, state.screen)

            store.take(
                herd(
                    pane("w1:p1", workspace = "01JNODE/w1"),
                    pane("w2:p1", workspace = "01JNODE/w2"),
                )
            )
            assertEquals("01JNODE/w2:p1", (state.screen as Screen.Pane).paneId)
        } finally {
            scope.cancel()
        }
    }

    // And once it has opened one it is done: the same ids come round again on a later herd, and an
    // intent nobody cancelled would take the operator off whatever they had moved to.
    @Test
    fun anIntentIsSpentOnceAndDoesNotFireAgainOnALaterHerd() {
        val (state, store, scope) = app()
        try {
            state.opening("01JNODE/w2")
            store.take(herd(pane("w2:p1", workspace = "01JNODE/w2")))
            assertEquals("01JNODE/w2:p1", (state.screen as Screen.Pane).paneId)

            state.go(Screen.Herd)
            store.take(herd(pane("w2:p1", workspace = "01JNODE/w2")))
            assertEquals(Screen.Herd, state.screen)
        } finally {
            scope.cancel()
        }
    }

    // A pane id carries its workspace and never its tab, so `tab.create`'s ack can only be matched
    // on the id the node sends beside the label.
    @Test
    fun aTabIsMatchedOnTheIdTheNodeSendsBesideItAndNotOnThePaneId() {
        val herd = Wire.decode(
            herd(
                pane("w1:p1", workspace = "01JNODE/w1", tab = "01JNODE/w1:t1"),
                pane("w1:p2", workspace = "01JNODE/w1", tab = "01JNODE/w1:t2"),
            )
        ).let { KamprStore().apply { accept(it!!) }.herd.value }

        assertEquals("01JNODE/w1:p2", herd.createdPane("01JNODE/w1:t2")?.id)
        assertEquals("01JNODE/w1:p1", herd.createdPane("01JNODE/w1")?.id, "the workspace opens at its first pane")
        assertEquals("01JNODE/w1:p1", herd.createdPane("01JNODE/w1:p1")?.id, "a split names the pane itself")
        assertNull(herd.createdPane("01JNODE/w9"), "nothing in the herd stands for it")
    }

    // An older node sends no `workspace_id`, and the id grammar is the only thing left.
    @Test
    fun aPaneFromANodeThatSendsNoWorkspaceIdIsStillFoundByItsOwn() {
        val herd = Wire.decode(herd(pane("w4:p7")))
            .let { KamprStore().apply { accept(it!!) }.herd.value }
        assertEquals("01JNODE/w4:p7", herd.createdPane("01JNODE/w4")?.id)
    }
}
