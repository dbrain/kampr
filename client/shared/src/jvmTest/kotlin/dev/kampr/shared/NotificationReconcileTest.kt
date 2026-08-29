package dev.kampr.shared

import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.platform.MemoryPrefs
import dev.kampr.shared.push.NoPush
import dev.kampr.shared.push.PushPlatform
import dev.kampr.shared.ui.AppState
import dev.kampr.shared.wire.Wire
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private class RecordingPush : PushPlatform by NoPush() {
    val reconciled = mutableListOf<Boolean>()

    override fun reconcile(anyBlocked: Boolean) {
        reconciled += anyBlocked
    }
}

// An undecodable frame is accepted as an empty herd and every assertion below then passes for the
// wrong reason — a pane with a misspelt field is dropped without a word (`Codec.decodeList`).
private fun KamprStore.take(frame: String) {
    accept(Wire.decode(frame) ?: error("undecodable: $frame"))
    check(herd.value.panes.isNotEmpty()) { "no pane survived decoding: $frame" }
}

private fun herd(vararg statuses: Pair<String, String>): String {
    val panes = statuses.joinToString(",") { (id, status) ->
        """{"id":"01JNODE/$id","node_id":"01JNODE","agent":"claude","agent_status":"$status"}"""
    }
    return """{"t":"herd","nodes":[{"id":"01JNODE","name":"box","kind":"local"}],"panes":[$panes]}"""
}

// The node's notification is a summary of the moment it was sent, and a phone that was asleep when
// the answer happened at the desk cannot be told by anything but a second push. When the app is
// actually running, the herd it holds is fresher than any notification — so it takes its own
// prompt down rather than waiting.
class NotificationReconcileTest {
    private fun app(): Triple<AppState, KamprStore, CoroutineScope> {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Unconfined)
        val store = KamprStore()
        return Triple(AppState(scope, store, MemoryPrefs(), null, push), store, scope)
    }

    private val push = RecordingPush()

    @Test
    fun aHerdWithNothingBlockedTakesTheNotificationDown() {
        val (_, store, scope) = app()
        try {
            store.take(herd("w1:p1" to "blocked"))
            assertEquals(listOf(true), push.reconciled)

            store.take(herd("w1:p1" to "idle"))
            assertEquals(
                listOf(true, false),
                push.reconciled,
                "answering the last blocked agent anywhere has to take the prompt off this device",
            )
        } finally {
            scope.cancel()
        }
    }

    // A tap opens the app onto a herd it has not fetched yet. An unloaded herd has no blocked panes
    // either, and reconciling against it would take down the very notification that was tapped.
    @Test
    fun aHerdThatHasNotArrivedYetIsNotEvidenceThatNothingIsBlocked() {
        val (_, _, scope) = app()
        try {
            assertTrue(
                push.reconciled.isEmpty(),
                "an empty herd nobody has fetched is not an answered prompt",
            )
        } finally {
            scope.cancel()
        }
    }

    // One agent answered out of two leaves the other one waiting, and the notification naming it
    // has to stay. Correcting its wording is the node's job — it is the only place that holds the
    // questions — and this client must not mistake "fewer" for "none".
    @Test
    fun oneOfTwoBeingAnsweredLeavesTheNotificationStanding() {
        val (_, store, scope) = app()
        try {
            store.take(herd("w1:p1" to "blocked", "w2:p1" to "blocked"))
            store.take(herd("w1:p1" to "idle", "w2:p1" to "blocked"))
            assertTrue(
                push.reconciled.all { it },
                "a herd that still has a blocked agent never says the prompt is finished",
            )
        } finally {
            scope.cancel()
        }
    }
}
