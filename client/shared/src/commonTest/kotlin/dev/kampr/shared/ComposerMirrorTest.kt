package dev.kampr.shared

import dev.kampr.shared.model.ComposerMirror
import dev.kampr.shared.model.DeskLine
import dev.kampr.shared.model.keepTokensWhole
import dev.kampr.shared.wire.EditKeys
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

private val KEYS = EditKeys(back = "\u007f", left = "\u001b[D", right = "\u001b[C", newline = "\n")
private const val BACK = "\u007f"
private const val LEFT = "\u001b[D"
private const val RIGHT = "\u001b[C"

private fun drawn(text: String, caret: Int = text.length) = DeskLine(text, "\u0003", caret, KEYS)

// Claude's and Codex's box: a pasted picture is one `[Image #n]` a single key takes whole (#620).
private val PICTURES = KEYS.copy(images = true)
private fun pictured(text: String, caret: Int = text.length) = DeskLine(text, "\u0003", caret, PICTURES)
private const val LINE = "look [Image #1] here"

// A mirror that has heard the pane's box drawn and holding `pane`, with the reply box in step.
private fun inStep(pane: String = ""): ComposerMirror = ComposerMirror().also { it.heard(drawn(pane), box = pane, now = 0) }

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

    // **A box the node cannot see for a moment still holds what this one typed into it.** Claude
    // repaints with its caret off the box (#552), and a send in that gap typed the whole line again
    // after the copy already in the pane.
    @Test
    fun sendingWhileThePanesBoxCannotBeSeenIsStillOnlyTheEnter() {
        val mirror = inStep()
        mirror.typed("", "push", live = true, now = 0)
        mirror.heard(null, box = "push", now = 50)
        assertEquals(listOf("\r"), mirror.submit("push", now = 60))
    }

    // A file goes where the reply box's caret is: the pane's caret is walked there first, and the
    // node types it at that caret. Only into a box in step that may type.
    @Test
    fun aPictureIsPlacedAtTheReplyBoxsCaret() {
        val mirror = ComposerMirror().also { it.heard(pictured("alpha gamma"), box = "alpha gamma", now = 0) }
        assertEquals(listOf(LEFT.repeat(5)), mirror.place("alpha gamma", at = 6, live = true, now = 10))
        assertEquals(emptyList(), mirror.place("alpha gamma", at = 6, live = true, now = 20))

        val words = ComposerMirror().also { it.heard(drawn("alpha gamma"), box = "alpha gamma", now = 0) }
        assertEquals(listOf(LEFT.repeat(5)), words.place("alpha gamma", at = 6, live = true, now = 10))
        val apart = ComposerMirror().also { it.heard(pictured("alpha"), box = "mine", now = 0) }
        assertNull(apart.place("mine", at = 2, live = true, now = 10))
        assertNull(mirror.place("alpha gamma", at = 6, live = false, now = 30))
    }

    // The token comes back as the pane's line once the box's own keys have settled, and the box
    // takes it with the caret where the pane's is: after the picture.
    @Test
    fun thePlacedTokenIsTakenUpWithTheCaretAfterIt() {
        val mirror = ComposerMirror().also { it.heard(pictured("alpha gamma"), box = "alpha gamma", now = 0) }
        mirror.place("alpha gamma", at = 6, live = true, now = 10)
        val after = 10 + ComposerMirror.QUIET + 1
        assertEquals("alpha [Image #1]gamma", mirror.heard(pictured("alpha [Image #1]gamma", 16), box = "alpha gamma", now = after))
        assertEquals(16, mirror.at)
    }

    // One key steps over or takes the whole token, so the keys count it as one character.
    @Test
    fun theTokenIsOneKeyToTheHarness() {
        listOf(
            Edit("the token deleted whole", LINE, 15, "look  here", listOf(BACK)),
            Edit("a word after it, from before it", LINE, 5, "look [Image #1] now here", listOf(RIGHT.repeat(2), "now ")),
            Edit("a word before it, from the end", LINE, 20, " [Image #1] here", listOf(LEFT.repeat(7), BACK.repeat(4))),
        ).forEach { edit ->
            val mirror = ComposerMirror()
            mirror.heard(pictured(edit.pane, edit.caret), box = edit.pane, now = 0)
            assertEquals(edit.keys, mirror.typed(edit.pane, edit.after, live = true, now = 10), edit.name)
        }
    }

    // The reply box treats a token the way the harness does: an edit that cuts into one takes it
    // whole, and text typed inside one lands after it.
    @Test
    fun anEditIntoATokenTakesItWhole() {
        assertEquals("look  here" to 5, keepTokensWhole(LINE, "look [Image #1 here"))
        assertEquals("look  here" to 5, keepTokensWhole(LINE, "look Image #1] here"))
        assertEquals("look [Image #1]X here" to 16, keepTokensWhole(LINE, "look [ImXage #1] here"))
        assertNull(keepTokensWhole(LINE, "look [Image #1] there"))
        assertNull(keepTokensWhole(LINE, " [Image #1] here"))
    }

    // A caret against a word gets a space first, so the picture is not glued to it.
    @Test
    fun aPicturePlacedAgainstAWordIsSetApartFromIt() {
        val mirror = ComposerMirror().also { it.heard(pictured("alpha gamma"), box = "alpha gamma", now = 0) }
        assertEquals(listOf(LEFT.repeat(6), " "), mirror.place("alpha gamma", at = 5, live = true, now = 10))
    }

    // **A box out of step types only where it differs.** Taking the whole line back and typing it
    // again turned a picture into its `[Image #n]` as words, and the Enter after a retyped line
    // was kept as a newline (#619).
    @Test
    fun aBoxOutOfStepKeepsThePicturesThePaneAlreadyHolds() {
        val mirror = adopted()
        assertNull(mirror.typed("look [Image #1]here", "look [Image #1]here!", live = false, now = 2_000))
        assertEquals(listOf(RIGHT.repeat(4), "!", "\r"), mirror.submit("look [Image #1]here!", now = 2_010))
    }

    // A picture the pane has lost goes back in as the file it was, not as its name.
    @Test
    fun aPictureThePaneLostIsPastedAgainNotTyped() {
        val mirror = adopted()
        assertNull(mirror.heard(pictured("look here", 9), box = "look [Image #1]here!", now = 3_000))
        assertEquals(listOf(BACK.repeat(4), PASTE, "here!", "\r"), mirror.submit("look [Image #1]here!", now = 3_010))
    }

    // A file typed as its path draws no token, so it is never the picture the next token is.
    @Test
    fun aPathPlacedBeforeAPictureIsNotTakenForIt() {
        val mirror = ComposerMirror()
        mirror.heard(pictured("look "), box = "look ", now = 0)
        mirror.placed("/p/notes.txt ")
        mirror.heard(pictured("look /p/notes.txt "), box = "look ", now = 10)
        mirror.placed(PASTE)
        mirror.heard(pictured("look /p/notes.txt [Image #1]"), box = "look /p/notes.txt ", now = 20)
        assertNull(mirror.heard(pictured("look /p/notes.txt "), box = "look /p/notes.txt [Image #1]!", now = 3_000))
        assertEquals(listOf(PASTE, "!", "\r"), mirror.submit("look /p/notes.txt [Image #1]!", now = 3_010))
    }

    // Codex numbers from 1 again once its line holds no picture (#620), so a token the pane's own
    // keyboard pasted after the line was sent or cleared is not the file this box placed before.
    @Test
    fun aTokenNumberUsedAgainAfterTheLineEmptiedIsNotTheOldPicture() {
        val mirror = adopted()
        mirror.heard(pictured(""), box = "look [Image #1]here", now = 3_000)
        mirror.heard(pictured("[Image #1]"), box = "", now = 6_000)
        assertNull(mirror.heard(pictured(""), box = "[Image #1]!", now = 9_000))
        assertEquals(listOf("[Image #1]!", "\r"), mirror.submit("[Image #1]!", now = 9_010))
    }
}

private const val PASTE = "\u001b[200~/p/shot.png\u001b[201~"

// A Claude box that held `look here`, had a picture placed after `look ` and took the token back.
private fun adopted(): ComposerMirror = ComposerMirror().also {
    it.heard(pictured("look here"), box = "look here", now = 0)
    it.place("look here", at = 5, live = true, now = 10)
    it.placed(PASTE)
    it.heard(pictured("look [Image #1]here", 15), box = "look here", now = 10 + ComposerMirror.QUIET + 1)
}
