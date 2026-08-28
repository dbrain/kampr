package dev.kampr.conversation

import dev.kampr.shared.model.DeskLine
import kotlin.test.Test
import kotlin.test.assertFalse
import kotlin.test.assertTrue

private val CTRL_C = "\u0003"

class DeskStripTest {
    // The strip exists to say what sending would *do*, and what it does is append: herdr's
    // `pane.send_text` adds to the end of the line rather than replacing it, which is how a
    // sentence begun at the desk and a reply sent from a phone came to submit as one.
    @Test
    fun theStripSaysThatAReplyIsAddedToTheLineRatherThanReplacingIt() {
        val said = deskWords(DeskLine("push the branch when", CTRL_C), "claude")
        assertTrue(said.contains("claude"), "the strip did not name the agent whose box it is: $said")
        assertTrue(
            said.contains("added to the end"),
            "the strip did not say that a reply is appended, which is the whole of the surprise: $said",
        )
    }

    // A pane whose agent the node could not name is still a pane with a line in its box.
    @Test
    fun aPaneWithNoNamedAgentStillSaysWhoseBoxItIs() {
        assertTrue(deskWords(DeskLine("half a sentence", CTRL_C), null).contains("the agent"))
    }

    // **A guessed keystroke is worse than no button.** The three harnesses do not agree on what
    // empties a composer — `ctrl+u` takes one visual row of Claude's wrapped buffer and `ctrl+c`
    // arms an exit on agy — so the node sends the key it measured, or none, and none means the
    // control is absent rather than disabled or guessed.
    @Test
    fun aHarnessWithNoMeasuredKeyOffersNoTakeoverAtAll() {
        assertFalse(deskTakeable(DeskLine("half a sentence", null), enabled = true))
        assertTrue(deskTakeable(DeskLine("half a sentence", CTRL_C), enabled = true))
    }

    // A device that may not type may not clear a pane's box either — it is a write, and it goes
    // down the same `input` path the node already refuses with `not_writer`.
    @Test
    fun aReadOnlyDeviceIsOfferedNoTakeoverEither() {
        assertFalse(deskTakeable(DeskLine("half a sentence", CTRL_C), enabled = false))
    }

    // Nothing at the desk is nothing to take over.
    @Test
    fun anEmptyBoxOffersNothing() {
        assertFalse(deskTakeable(null, enabled = true))
    }
}
