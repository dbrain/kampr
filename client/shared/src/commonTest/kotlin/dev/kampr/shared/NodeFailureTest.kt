package dev.kampr.shared

import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.model.paneTitle
import dev.kampr.shared.model.saidOutLoud
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Wire
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNull
import kotlin.test.assertTrue

private const val HERE = "01JHERE/w1:p1"
private const val THERE = "01JTHERE/w2:p4"

private fun failure(frame: String): ServerMsg.Failure =
    Wire.decode(frame) as? ServerMsg.Failure ?: error("undecodable: $frame")

private const val OFFLINE =
    """{"t":"error","code":"node_offline","message":"workbox is offline","node":"01JTHERE"}"""

// The operator's rule, verbatim: "i just don't want to hear about something not relevant to the
// node im on loudly on mobile, if the thing im using disconnects thats a different thing."
class NodeFailureTest {
    @Test
    fun anErrorNamesTheNodeItIsAbout() {
        assertEquals("01JTHERE", failure(OFFLINE).node)
        assertNull(failure(OFFLINE).pane)
    }

    // Absent is what every error carried before the field existed, and an installed client that
    // has never heard of it must go on behaving exactly as it does.
    @Test
    fun anErrorAboutNothingInParticularStillNamesNothing() {
        assertNull(failure("""{"t":"error","code":"not_writer","message":"this device is read-only"}""").node)
    }

    @Test
    fun aNodeGoingOfflineElsewhereDoesNotInterruptThePaneInHand() {
        assertFalse(saidOutLoud(failure(OFFLINE), HERE), "a node nobody is looking at took the screen")
    }

    @Test
    fun theNodeTheOperatorIsOnIsStillSaidOutLoud() {
        assertTrue(saidOutLoud(failure(OFFLINE), THERE), "the pane in hand went away in silence")
    }

    // Auth, a revocation, the socket itself: there is nowhere quieter for these to go.
    @Test
    fun aRefusalAboutNeitherAPaneNorANodeIsLoudWherever() {
        val refusal = failure("""{"t":"error","code":"not_writer","message":"this device is read-only"}""")
        assertTrue(saidOutLoud(refusal, HERE))
        assertTrue(saidOutLoud(refusal, null))
    }

    @Test
    fun aPaneScopedRefusalIsLoudOnlyOnThatPane() {
        val stream = failure(
            """{"t":"error","code":"stream_unavailable","pane":"$THERE","message":"kampr cannot run herdr"}"""
        )
        assertTrue(saidOutLoud(stream, THERE))
        assertFalse(saidOutLoud(stream, HERE))
    }

    // Quiet is not lost. The pane keeps what was refused about it, so the operator is told when
    // they arrive at the empty pane rather than while they are working on another one.
    @Test
    fun aPaneKeepsWhatWasRefusedAboutItForWhoeverOpensIt() {
        val store = KamprStore()
        store.pane(THERE)
        store.accept(
            failure("""{"t":"error","code":"stream_unavailable","pane":"$THERE","message":"kampr cannot run herdr"}""")
        )
        assertEquals("kampr cannot run herdr", store.pane(THERE).refusal)
        assertNull(store.pane(HERE).refusal, "a refusal about one pane was recorded against another")
    }
}

// The session's own name, off the wire and into the sidebar. Without it the template's `title`
// token is permanently unresolved and every pane in one repository is named after the same
// working directory.
class PaneTitleFieldTest {
    private fun paneOf(frame: String) =
        (Wire.decode(frame) as ServerMsg.Herd).panes.single()

    @Test
    fun aPaneCarriesTheHarnessesOwnNameForItsSession() {
        val pane = paneOf(
            """{"t":"herd","nodes":[],"panes":[{"id":"$HERE","node_id":"01JHERE","rows":30,
               "cwd":"/home/u/dev/kampr","title":"the width inference","agent":"claude"}]}"""
        )
        assertEquals("the width inference", pane.title)
        assertEquals("the width inference · claude", paneTitle(pane))
    }

    // Generated loses to typed, at every level.
    @Test
    fun aLabelTheOperatorTypedStillWins() {
        val pane = paneOf(
            """{"t":"herd","nodes":[],"panes":[{"id":"$HERE","node_id":"01JHERE","rows":30,
               "cwd":"/home/u/dev/kampr","title":"the width inference","label":"build","agent":"claude"}]}"""
        )
        assertEquals("build · claude", paneTitle(pane))
    }

    @Test
    fun aPaneWithNoTitleIsNamedTheWayItAlwaysWas() {
        val pane = paneOf(
            """{"t":"herd","nodes":[],"panes":[{"id":"$HERE","node_id":"01JHERE","rows":30,
               "cwd":"/home/u/dev/kampr","agent":"claude"}]}"""
        )
        assertNull(pane.title)
        assertEquals("kampr · claude", paneTitle(pane))
    }
}
