package dev.kampr.shared

import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Wire
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull
import kotlin.test.assertTrue

private const val PANE = "01JNODE/w1:p1"

// Claude's, measured. It is written as an escape rather than as the byte because a control
// character in a source file is one nobody reviewing this can see.
private const val CTRL_C = "\u0003"

private fun decode(frame: String): ServerMsg = Wire.decode(frame) ?: error("undecodable: $frame")

private val TYPED =
    """{"t":"convo.composer","pane":"$PANE","text":"push the branch when","clear":"$CTRL_C"}"""

class DeskLineTest {
    // The defect: `input` is herdr's `pane.send_text` and it appends to whatever is already on the
    // pane's line, so a sentence begun at the desk and a reply sent from a phone submit as one
    // run-on line — and nothing on the phone had ever shown the first half.
    @Test
    fun theHalfSentenceLeftAtTheDeskReachesTheClientItIsDrawnFor() {
        val store = KamprStore()
        store.accept(decode(TYPED))
        val desk = store.pane(PANE).desk
        assertEquals("push the branch when", desk?.text)
        assertEquals(CTRL_C, desk?.clear)
    }

    // `text: null` is how the strip comes down. A client that only ever heard about a composer
    // with something in it would go on claiming a line the operator emptied minutes ago.
    @Test
    fun anEmptiedComposerTakesTheStripDownRatherThanLeavingItStanding() {
        val store = KamprStore()
        store.accept(decode(TYPED))
        store.accept(decode("""{"t":"convo.composer","pane":"$PANE","text":null}"""))
        assertNull(store.pane(PANE).desk, "an emptied composer left its last line standing")
    }

    // The clearing keystroke is a per-harness measurement the node owns: `ctrl+u` empties Codex's
    // whole box and takes one visual row of Claude's, and `ctrl+c` empties Claude's and arms an
    // **exit** on agy. A harness nobody has measured one for sends none, and the client has to be
    // able to tell that from a key rather than reach for one of its own.
    @Test
    fun aHarnessWithNoMeasuredClearingKeyCarriesNoneAndIsNotGuessedAt() {
        val store = KamprStore()
        store.accept(decode("""{"t":"convo.composer","pane":"$PANE","text":"half a sentence"}"""))
        val desk = store.pane(PANE).desk
        assertEquals("half a sentence", desk?.text)
        assertNull(desk?.clear, "a clearing key nobody measured was invented")
    }

    // A `convo.composer` naming no pane is not about anything.
    @Test
    fun aDeskLineWithNoPaneIsNotAFrame() {
        assertNull(Wire.decode("""{"t":"convo.composer","text":"push the branch"}"""))
    }

    // It is the pane's own state and not this client's, so a device that may not type still sees
    // what is sitting in the box it is not allowed to add to.
    @Test
    fun aReadOnlyDeviceStillSeesWhatIsWaitingAtTheDesk() {
        val store = KamprStore()
        store.accept(decode(TYPED))
        store.accept(decode("""{"t":"role","role":"readonly"}"""))
        assertTrue(store.readOnly)
        assertEquals("push the branch when", store.pane(PANE).desk?.text)
    }
}
