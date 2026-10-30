package dev.kampr.shared

import dev.kampr.shared.model.hasDesk
import dev.kampr.shared.model.nativeActions
import dev.kampr.shared.model.nativeFacts
import dev.kampr.shared.model.nativeSpoken
import dev.kampr.shared.wire.ManageOp
import dev.kampr.shared.wire.NativeExit
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.PaneInfo
import kotlin.test.Test
import kotlin.test.assertEquals

private val HERDR = PaneInfo("01JNODE/w1:p1", "01JNODE", label = "shell")
private val LIVE = PaneInfo("01JNODE/n:01ja", "01JNODE", label = "build", provider = "native", sessionId = "01ja", onDesk = "w1:p3")
private val DETACHED = LIVE.copy(onDesk = null, detached = true)
private val EXITED = DETACHED.copy(exited = NativeExit(code = 0))
private val KILLED = DETACHED.copy(exited = NativeExit(signal = 9))

class NativePaneTest {
    @Test
    fun aKamprSessionIsTheDefaultSoOnlyItsEndIsWorthSaying() {
        val cases = listOf(
            HERDR to emptyList(),
            LIVE to emptyList(),
            DETACHED to emptyList(),
            EXITED to listOf("exited 0"),
            KILLED to listOf("killed by signal 9"),
            DETACHED.copy(exited = NativeExit()) to listOf("exited"),
        )
        for ((pane, facts) in cases) assertEquals(facts, nativeFacts(pane), "$pane")
        assertEquals(null, nativeSpoken(HERDR))
        assertEquals(null, nativeSpoken(DETACHED))
        assertEquals("killed by signal 9", nativeSpoken(KILLED))
    }

    @Test
    fun theActionsFollowWhetherItRunsAndWhereItIsShown() {
        val cases = listOf(
            Triple(HERDR, true, emptyList()),
            Triple(LIVE, true, listOf(ManageOp.NativeClose(LIVE.id))),
            Triple(DETACHED, true, listOf(ManageOp.NativeDesk(DETACHED.id), ManageOp.NativeClose(DETACHED.id))),
            Triple(DETACHED, false, listOf(ManageOp.NativeClose(DETACHED.id))),
            Triple(EXITED, true, listOf(ManageOp.NativeForget(EXITED.id))),
            Triple(KILLED, false, listOf(ManageOp.NativeForget(KILLED.id))),
        )
        for ((pane, desk, ops) in cases) {
            assertEquals(ops, nativeActions(pane, desk).map { it.op }, "$pane desk=$desk")
        }
    }

    @Test
    fun aMachineHasADeskWhenItsHerdrIsUpOrHasEverBeenNamed() {
        assertEquals(true, NodeInfo("a").hasDesk)
        assertEquals(true, NodeInfo("a", online = false, herdrVersion = "0.8.2").hasDesk)
        assertEquals(false, NodeInfo("a", online = false).hasDesk)
    }
}
