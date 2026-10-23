package dev.kampr.shared

import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.model.saidOutLoud
import dev.kampr.shared.platform.MemoryPrefs
import dev.kampr.shared.ui.AppManage
import dev.kampr.shared.ui.AppState
import dev.kampr.shared.ui.Screen
import dev.kampr.shared.ui.quickSessionOp
import dev.kampr.shared.wire.ManageOp
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.Wire
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

private const val MANAGING =
    """{"t":"hello","protocol":1,"node_id":"01JHUB","node_name":"comingclean","build":"test",""" +
        """"role":"full","caps":{"manage":true},"security":{"tier":0,"passkeys":false}}"""

private const val EMPTY_HERD =
    """{"t":"herd","nodes":[{"id":"01JHUB","name":"comingclean","kind":"local","online":true,"native":true}],"panes":[]}"""

private const val MADE =
    """{"t":"herd","nodes":[{"id":"01JHUB","name":"comingclean","kind":"local","online":true,"native":true}],""" +
        """"panes":[{"id":"01JHUB/n:s1","node_id":"01JHUB","provider":"native","updated_at":"7"}]}"""

private const val OLD_HERD =
    """{"t":"herd","nodes":[{"id":"01JHUB","name":"comingclean","kind":"local","online":true}],"panes":[]}"""

private const val OLD_MADE =
    """{"t":"herd","nodes":[{"id":"01JHUB","name":"comingclean","kind":"local","online":true}],""" +
        """"panes":[{"id":"01JHUB/w7:p1","node_id":"01JHUB","workspace_id":"01JHUB/w7","updated_at":"7"}]}"""

// The + beside a machine starts a Kampr session, shown on the machine's desk when it has one: the
// operator, on 0.1.119, *"lets default to kampr sessions (+ next to host) ... default kampr attached
// for all sessions unless that breaks functionality"*.
//
// It has no sheet in front of it, and a sheet is what has always done the two
// things that happen after a create: open what was made, and say when the node refused. Both of
// those are the whole of this — without them the press is a button that appears to do nothing and a
// workspace that turns up at the foot of the herd, which is the report `OpenOnCreateTest` exists
// for, arriving a second way.
class QuickSessionAckTest {
    private fun app(): Triple<AppState, KamprStore, CoroutineScope> {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Unconfined)
        val store = KamprStore()
        val state = AppState(scope, store, MemoryPrefs(), null).apply { go(Screen.Herd) }
        store.take(MANAGING)
        store.take(EMPTY_HERD)
        return Triple(state, store, scope)
    }

    private fun KamprStore.take(frame: String) = accept(Wire.decode(frame) ?: error("undecodable: $frame"))

    // A node from before 0.1.119 keeps no sessions and says nothing about it, so its + is the
    // workspace it always made: a herd is upgraded a host at a time.
    @Test
    fun aHostsPlusAsksForAKamprSessionOnTheDeskOnlyWhereThereIsOne() {
        val desk = NodeInfo(id = "01JHUB", name = "comingclean", kind = "local", online = true, native = true)
        val bare = NodeInfo(id = "01JBARE", name = "nodesk", kind = "peer", online = false, reachable = true, native = true)
        val old = NodeInfo(id = "01JOLD", name = "older", kind = "peer", online = true)
        assertEquals(ManageOp.NativeCreate("01JHUB", desk = true), quickSessionOp(desk))
        assertEquals(ManageOp.NativeCreate("01JBARE", desk = false), quickSessionOp(bare))
        assertEquals(ManageOp.WorkspaceCreate("01JOLD"), quickSessionOp(old))
    }

    @Test
    fun aWorkspaceMadeByAnOlderHostsPlusStillOpens() {
        val (state, store, scope) = app()
        try {
            store.take(OLD_HERD)
            AppManage(state).quickSession("01JHUB")
            store.take("""{"t":"managed","op":"workspace.create","ok":true,"id":"01JHUB/w7"}""")
            store.take(OLD_MADE)
            assertEquals("01JHUB/w7:p1", (state.screen as Screen.Pane).paneId)
        } finally {
            scope.cancel()
        }
    }

    @Test
    fun aSessionStartedFromAHostsPlusOpensWhenItsPaneArrives() {
        val (state, store, scope) = app()
        try {
            AppManage(state).quickSession("01JHUB")
            store.take("""{"t":"managed","op":"native.create","ok":true,"id":"01JHUB/n:s1"}""")
            assertEquals(Screen.Herd, state.screen, "the ack lands before the patch carrying the pane")

            store.take(MADE)
            assertEquals("01JHUB/n:s1", (state.screen as Screen.Pane).paneId)
        } finally {
            scope.cancel()
        }
    }

    // The desk is the extra, and herdr not answering must not cost the operator the session: the
    // press is asked again without it, and only that answer is the press's.
    @Test
    fun aDeskThatCannotBeReachedIsDroppedRatherThanTheSession() {
        val (state, store, scope) = app()
        try {
            AppManage(state).quickSession("01JHUB")
            store.take(
                """{"t":"managed","op":"native.create","ok":false,""" +
                    """"code":"herdr_unavailable","message":"herdr would not start"}""",
            )
            assertNull(store.failure.value, "a desk herdr could not give was announced as the press failing")
            store.take("""{"t":"managed","op":"native.create","ok":true,"id":"01JHUB/n:s1"}""")
            store.take(MADE)
            assertEquals("01JHUB/n:s1", (state.screen as Screen.Pane).paneId)
        } finally {
            scope.cancel()
        }
    }

    // A keeper that would not start, a machine that has gone since the herd was drawn: the node
    // knows why and nothing else does.
    @Test
    fun aRefusalIsSaidOutLoudRatherThanLeavingTheButtonLookingDead() {
        val (state, store, scope) = app()
        try {
            AppManage(state).quickSession("01JHUB")
            store.take(
                """{"t":"managed","op":"native.create","ok":false,""" +
                    """"code":"herdr_unavailable","message":"herdr would not start"}""",
            )
            store.take(
                """{"t":"managed","op":"native.create","ok":false,"id":null,""" +
                    """"code":"bad_request","message":"the keeper would not start"}""",
            )
            val failure = assertNotNull(store.failure.value, "a refused quick session said nothing at all")
            assertEquals("the keeper would not start", failure.message)
            assertTrue(
                saidOutLoud(failure, paneOnScreen = null),
                "the refusal was addressed to a pane or a node, so the herd screen would swallow it",
            )
        } finally {
            scope.cancel()
        }
    }

    // Every other ack on that socket belongs to somebody else — the sheet's own creates, a rename,
    // a close — and one of them arriving must not be read as this press coming back. A split is the
    // one that proves it: its ack names a pane that *is* in the herd, so an unfiltered reader opens
    // it and lands the operator somewhere nobody asked to go.
    @Test
    fun anAckForSomethingElseIsNotReadAsThisOne() {
        val (state, store, scope) = app()
        try {
            AppManage(state).quickSession("01JHUB")
            store.take("""{"t":"managed","op":"pane.split","ok":true,"id":"01JHUB/n:s1"}""")
            store.take(MADE)
            assertEquals(Screen.Herd, state.screen, "another op's ack was read as this press coming back")
        } finally {
            scope.cancel()
        }
    }

    // Nothing was pressed, so nothing is waiting: an ack for a session the sheet made is the sheet's
    // to act on, and this must not open a second thing behind it.
    @Test
    fun anAckWithNoPressBehindItOpensNothing() {
        val (state, store, scope) = app()
        try {
            store.take("""{"t":"managed","op":"native.create","ok":false,"code":"nope","message":"no"}""")
            assertNull(store.failure.value, "a refusal nobody here asked for was announced anyway")
            store.take("""{"t":"managed","op":"native.create","ok":true,"id":"01JHUB/n:s1"}""")
            store.take(MADE)
            assertEquals(Screen.Herd, state.screen)
        } finally {
            scope.cancel()
        }
    }
}
