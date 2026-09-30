package dev.kampr.conversation

import dev.kampr.shared.model.DeskLine
import dev.kampr.shared.wire.EditKeys
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

private val KEYS = EditKeys(back = "\u007f", left = "\u001b[D", right = "\u001b[C", newline = "\n")
private const val BACK = "\u007f"
private const val LEFT = "\u001b[D"
private const val RIGHT = "\u001b[C"

private fun drawn(text: String, caret: Int = text.length) = DeskLine(text, "\u0003", caret, KEYS)

// A mirror that has heard the pane's box drawn and holding `pane`, with the reply box in step.
private fun inStep(pane: String = ""): ComposerMirror =
    ComposerMirror().also { it.heard(drawn(pane), box = pane, now = 0) }

class ComposerMirrorTest {
    private data class Edit(val name: String, val pane: String, val caret: Int, val after: String, val keys: List<String>)

    // Every edit to the reply box is the fewest measured keys that make the pane's box the same
    // line: typed text lands at the caret, one `\x7f` takes one character, and the arrows walk the
    // pane's caret to where the edit is before it is made.
    @Test
    fun anEditToTheReplyBoxIsTheKeysThatMakeThePanesBoxTheSameLine() {
        listOf(
            Edit("typing at the end", "push", 4, "push the", listOf(" the")),
            Edit("deleting at the end", "push the", 8, "push th", listOf(BACK)),
            Edit("a word in the middle", "push branch", 11, "push the branch", listOf(LEFT.repeat(6), "the ")),
            Edit("a word out of the middle", "push the branch", 15, "push branch", listOf(LEFT.repeat(6), BACK.repeat(4))),
            Edit("a caret left at the front", "push", 0, "git push", listOf("git ")),
            Edit("a caret left behind the edit", "push the", 0, "push the branch", listOf(RIGHT.repeat(8), " branch")),
            Edit("a replaced word", "push main", 9, "push dev", listOf(BACK.repeat(4), "dev")),
            Edit("a line broken in two", "push it", 7, "push it\nnow", listOf("\n", "now")),
        ).forEach { edit ->
            val mirror = ComposerMirror()
            mirror.heard(drawn(edit.pane, edit.caret), box = edit.pane, now = 0)
            assertEquals(edit.keys, mirror.typed(edit.pane, edit.after, live = true, now = 10), edit.name)
        }
    }

    // A pane typed into at the desk fills the reply box as it is typed — it is the same line — and
    // that is a read: nothing is sent to be told it.
    @Test
    fun whatTheDeskTypesFillsAReplyBoxThatWasInStep() {
        val mirror = inStep()
        assertEquals("push the branch", mirror.heard(drawn("push the branch"), box = "", now = 10))
        assertEquals("", mirror.heard(drawn(""), box = "push the branch", now = 20))
    }

    // **A reply box the operator has written in is theirs.** The desk's line is adopted only into a
    // box that is empty or still holds exactly what it last mirrored; anything else is shown beside
    // it, the way it always was.
    @Test
    fun aReplyBoxThatHasGoneItsOwnWayIsNeverOverwritten() {
        val mirror = ComposerMirror()
        assertNull(mirror.heard(drawn("push the branch"), box = "a reply of my own", now = 0))
        assertNull(mirror.typed("a reply of my own", "a reply of my own!", live = true, now = 10))
    }

    // **No loops.** The box's own keys come back as the pane's line a few frames later, a keystroke
    // or two behind the box. Taking that up would put the box back to where it was a moment ago
    // while the operator is still typing.
    @Test
    fun theReplyBoxsOwnKeysComingBackAreNotTakenUpAsTheDesksLine() {
        val mirror = inStep()
        mirror.typed("", "p", live = true, now = 0)
        mirror.typed("p", "pu", live = true, now = 50)
        mirror.typed("pu", "pus", live = true, now = 100)
        assertNull(mirror.heard(drawn("pu"), box = "pus", now = 150), "an echo two keys behind was adopted")
        assertNull(mirror.heard(drawn("pus"), box = "pus", now = 200))
        assertEquals(listOf("h"), mirror.typed("pus", "push", live = true, now = 250))
    }

    // A line that stops agreeing once the box has gone quiet is somebody at the desk, and it is
    // theirs to have: the reply box takes it up.
    @Test
    fun aLineTheDeskChangedWhileTheBoxWasQuietIsTakenUpOnceTheEchoesHaveSettled() {
        val mirror = inStep()
        mirror.typed("", "push", live = true, now = 0)
        assertNull(mirror.heard(drawn("push it"), box = "push", now = 100))
        assertNull(mirror.settled(box = "push", now = 200))
        assertEquals("push it", mirror.settled(box = "push", now = 5_000))
    }

    // **Rule 3.** A box that is not drawn, a harness nobody measured keys for, a dropped socket and a
    // question standing on the pane are all a pane that is not taking a line: an edit then stays in
    // the reply box and nothing is typed.
    @Test
    fun nothingIsTypedIntoABoxThatIsNotTakingALine() {
        val gone = inStep().also { it.heard(null, box = "", now = 10) }
        assertNull(gone.typed("", "p", live = true, now = 20), "typed into a box that is not drawn")

        val unmeasured = ComposerMirror().also { it.heard(DeskLine("", null, 0, null), box = "", now = 0) }
        assertNull(unmeasured.typed("", "p", live = true, now = 10), "typed with keys nobody measured")

        assertNull(inStep().typed("", "p", live = false, now = 10), "typed while not live")
    }

    // Past the length a harness turns into a `[Pasted text]` placeholder (#564) a paste is not
    // typed at all: the box holds it and the send carries it whole.
    @Test
    fun aPasteLongerThanAHarnessTypesIsHeldForTheSend() {
        val long = "x".repeat(ComposerMirror.BURST + 1)
        val mirror = inStep()
        assertNull(mirror.typed("", long, live = true, now = 0))
        assertEquals(listOf(long, "\r"), mirror.submit(long, now = 10))
    }

    // **The send is the Enter the line was waiting for.** A box in step with the pane's already
    // holds every word, so sending them again would submit the line twice over.
    @Test
    fun sendingALineThePaneAlreadyHoldsIsOnlyTheEnter() {
        val mirror = inStep()
        mirror.typed("", "push", live = true, now = 0)
        assertEquals(listOf("\r"), mirror.submit("push", now = 10))
    }

    // A box that went its own way is sent the way it always was when the pane's box is empty, and
    // otherwise the pane's line is taken back one measured character at a time first — never with
    // a clearing key, which is `ctrl+c` on Claude and an exit on an empty one.
    @Test
    fun aReplyBoxOutOfStepReplacesThePanesLineWithTheKeysThatEditIt() {
        assertEquals(listOf("mine", "\r"), ComposerMirror().submit("mine", now = 0))

        val held = ComposerMirror().also { it.heard(drawn("push", caret = 1), box = "a reply", now = 0) }
        assertEquals(listOf(RIGHT.repeat(3), BACK.repeat(4), "a reply", "\r"), held.submit("a reply", now = 10))
    }
}
