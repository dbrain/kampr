package dev.kampr.conversation

import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.v2.runComposeUiTest
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.height
import dev.kampr.shared.ui.Breakpoint
import dev.kampr.shared.wire.Abilities
import dev.kampr.shared.wire.Billing
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.Control
import dev.kampr.shared.wire.ControlOption
import dev.kampr.shared.wire.Facets
import dev.kampr.shared.wire.Limits
import dev.kampr.shared.wire.LiveTool
import dev.kampr.shared.wire.LiveTurn
import dev.kampr.shared.wire.Queued
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.SessionFacts
import dev.kampr.shared.wire.Trouble
import dev.kampr.shared.wire.TroubleFix
import dev.kampr.shared.wire.TroubleRetry
import kotlinx.serialization.json.JsonPrimitive
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private val CODEX = Abilities(input = listOf("queue_next", "send_now"), steer = listOf("interrupt"), asks = listOf("permission"), facts = listOf("limits"))
private val TERMINAL = Abilities(asks = listOf("permission"), facts = listOf("trouble"))

private val FACTS = arrayOf<ServerMsg>(
    ServerMsg.ConvoSession(PANE_ID, SessionFacts("s1", model = "claude-sonnet-4-5", billing = Billing(provider = "Anthropic"))),
    ServerMsg.ConvoLimits(PANE_ID, Limits("Max 5x", JsonPrimitive(72), "2026-11-27T14:30:00Z")),
)

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.texts(): List<String> = onAllNodes(SemanticsMatcher.keyIsDefined(SemanticsProperties.Text), useUnmergedTree = true)
    .fetchSemanticsNodes()
    .flatMap { node -> node.config[SemanticsProperties.Text].map { it.text } }

@OptIn(ExperimentalTestApi::class)
class SeamPiecesTest {
    // src-headless-claude-phone.png: the one line was cut before "billed by", the fact it is for.
    @Test
    fun theFactsLineSaysWhoPaysFirst() = runComposeUiTest {
        seamView(PORTRAIT, seamPane(*FACTS))
        val line = texts().single { "claude-sonnet-4-5" in it }
        assertTrue(line.startsWith("billed by Anthropic"), line)
    }

    // hl-permission-phone.png: a headless agent's question said it was answered by keys into a
    // pane it does not have. An older node's "transcript" and "screen" keep their words.
    @Test
    fun aConversationSourcesQuestionSaysItIsAnsweredDirectly() {
        val cases = listOf(
            "conversation" to "Asked by claude, and answered to it directly.",
            "transcript" to "Read from the transcript, answered by keys into the pane.",
            "screen" to "Read from the pane, answered by keys into the pane.",
        )
        for ((source, said) in cases) {
            runComposeUiTest {
                seamView(PORTRAIT, seamPane(INDENT_ASK.copy(source = source)))
                assertEquals(1, onAllNodesWithText(said).fetchSemanticsNodes().size, "$source: ${texts().filter { "answer" in it }}")
            }
        }
    }

    // hl-running-queued-live-phone.png: a live call was drawn as a bare "Bash", its command never.
    @Test
    fun aLiveToolSaysWhatItIsRunning() = runComposeUiTest {
        val live = LiveTurn("Running the tests.", tools = listOf(LiveTool("t1", "Bash", """{"command":"cargo test -p kampr-node --lib""")))
        seamView(PORTRAIT, seamPane(ServerMsg.ConvoState(PANE_ID, "running"), ServerMsg.ConvoLive(PANE_ID, live)))
        assertTrue(texts().any { "cargo test -p kampr-node --lib" in it }, texts().toString())
    }

    // The queue was drawn above the turn in flight, which read as already answered.
    @Test
    fun theQueueIsDrawnUnderTheTurnInFlight() = runComposeUiTest {
        seamView(
            PORTRAIT,
            seamPane(
                ServerMsg.ConvoState(PANE_ID, "running"),
                ServerMsg.ConvoLive(PANE_ID, LiveTurn("I'll start by reading the relay.")),
                ServerMsg.ConvoFacets(PANE_ID, Facets(queued = listOf(Queued("and then run the client gate too")))),
            ),
        )
        val liveTop = onNodeWithText("I'll start by reading the relay.", substring = true, useUnmergedTree = true).getUnclippedBoundsInRoot().top
        val queuedTop = onNodeWithText("and then run the client gate too", substring = true, useUnmergedTree = true).getUnclippedBoundsInRoot().top
        assertTrue(queuedTop > liveTop, "queued at $queuedTop, the live turn at $liveTop")
    }

    // A source that takes a message back is told each one under an id, and the card it sent has a
    // Remove that names that id and puts the words back in the box.
    @Test
    fun aQueuedMessageCanBeTakenBackWhereTheSourceSaysSo() = runComposeUiTest {
        RecordingIo.sent.clear()
        val pane = seamPane(ServerMsg.ConvoState(PANE_ID, "running"))
        pane.draft = "and then run the client gate too"
        seamView(PORTRAIT, pane)
        onNodeWithContentDescription("Queue this reply", substring = true).performClick()
        waitForIdle()
        val sent = RecordingIo.sent.filterIsInstance<ClientMsg.InputText>().single()
        assertEquals("next", sent.queue)
        val id = requireNotNull(sent.id) { "no id the queue can name" }
        onNodeWithContentDescription("Take this queued message back").performClick()
        waitForIdle()
        assertEquals(listOf(ClientMsg.InputUnqueue(PANE_ID, id)), RecordingIo.sent.filterIsInstance<ClientMsg.InputUnqueue>())
        assertEquals("and then run the client gate too", pane.draft)
    }

    @Test
    fun aSourceWithoutUnqueueOffersNoRemove() = runComposeUiTest {
        RecordingIo.sent.clear()
        val pane = seamPane(ServerMsg.ConvoState(PANE_ID, "running"), abilities = CODEX)
        pane.draft = "one more thing"
        seamView(PORTRAIT, pane)
        onNodeWithContentDescription("Queue this reply", substring = true).performClick()
        waitForIdle()
        assertEquals(0, onAllNodesWithContentDescription("Take this queued message back").fetchSemanticsNodes().size)
    }

    // hl-session-sheet-phone.png: the raw value, a Change that opened nothing, and the current
    // option drawn fainter than the rest.
    @Test
    fun aChoiceSaysItsOptionsNameAndMarksTheCurrentOne() = runComposeUiTest {
        val control = Control(
            "permission",
            "Permissions",
            "choice",
            JsonPrimitive("acceptEdits"),
            listOf(ControlOption("default", "Default"), ControlOption("acceptEdits", "Accept edits")),
        )
        val pane = seamPane(ServerMsg.ConvoControls(PANE_ID, listOf(control)))
        seamScreen(PORTRAIT) { SessionSheet(pane, RecordingIo, Breakpoint.Portrait, onDismiss = {}, now = SEAM_NOW) }
        assertEquals(0, texts().count { it == "acceptEdits" }, "the raw value is drawn")
        assertEquals(0, onAllNodesWithContentDescription("Change Permissions").fetchSemanticsNodes().size, "a Change that opens nothing")
        assertEquals(1, texts().count { it == "✓" }, "the current option is not ticked")
    }

    // "retry in 32s" stood still, and "held until" and "resets" were ISO stamps.
    @Test
    fun timesCountDownInTheReadersOwnClock() = runComposeUiTest {
        mainClock.autoAdvance = false
        var now = SEAM_NOW
        val trouble = Trouble("api", "API overloaded (529)", retry = TroubleRetry(2, 10, 8_000), fix = TroubleFix.Wait("2026-11-27T14:30:00Z"))
        val pane = seamPane(ServerMsg.ConvoTrouble(PANE_ID, trouble), *FACTS)
        seamScreen(PORTRAIT) { ConversationView(pane, demoInfo(status = "idle", converses = true), Modifier, clock = { now }) }
        assertTrue(texts().any { "retry 2 of 10 in 8s" in it }, texts().toString())
        now += 3_000
        mainClock.advanceTimeBy(3_100)
        waitForIdle()
        val said = texts()
        assertTrue(said.any { "retry 2 of 10 in 5s" in it }, said.toString())
        assertTrue(said.none { "2026-11-27T14:30" in it }, "an ISO stamp is drawn: $said")
        assertTrue(said.any { Regex("""held until [^(]*\d\d:\d\d \(in 2h 29m\)""").containsMatchIn(it) }, said.toString())
        assertTrue(said.any { Regex("""resets in 2h 29m \([^)]*\d\d:\d\d\)""").containsMatchIn(it) }, said.toString())
    }

    // tc-subagent-from-task.png: "agent — Explore — Map…", the task's kind taken for an agent type.
    @Test
    fun aSubagentOpenedFromItsTaskIsTitledByTheTaskAlone() = runComposeUiTest {
        seamView(PORTRAIT, seamPane(ServerMsg.ConvoTasks(PANE_ID, SEAM_TASKS)))
        onNodeWithContentDescription("Show the tasks").performClick()
        waitForIdle()
        onNodeWithContentDescription("Open Explore — Map the manage op end-to-end path").performClick()
        waitForIdle()
        assertTrue(texts().any { it.startsWith("Explore — Map") }, texts().toString())
        assertTrue(texts().none { it.startsWith("agent — ") }, texts().filter { "Explore" in it }.toString())
    }

    // The folded bar said "3 running" and nothing of what.
    @Test
    fun theFoldedTaskBarNamesTheNewestTask() = runComposeUiTest {
        seamView(PORTRAIT, seamPane(ServerMsg.ConvoTasks(PANE_ID, SEAM_TASKS)))
        assertTrue(texts().any { it.startsWith("· cargo test --workspace") }, texts().toString())
    }

    // tc-task-sheet.png: the subtitle was the pane's raw id.
    @Test
    fun theTaskSheetSaysHowLongItHasRun() = runComposeUiTest {
        val pane = seamPane(ServerMsg.ConvoTasks(PANE_ID, SEAM_TASKS))
        seamScreen(PORTRAIT) { TaskOutputSheet(pane, SEAM_TASKS[1], RecordingIo, Breakpoint.Portrait, onDismiss = {}, now = SEAM_NOW) }
        val said = texts()
        assertTrue(said.contains("shell · running 8m 6s"), said.toString())
        assertTrue(said.none { PANE_ID in it }, said.toString())
    }

    // Attaching on a source that takes no files is a press that cannot work.
    @Test
    fun theAttachButtonIsOfferedOnlyWhereTheSourceTakesFiles() {
        for ((abilities, offered) in listOf(SEAM_HEADLESS to true, CODEX to false, TERMINAL to true)) {
            runComposeUiTest {
                seamView(PORTRAIT, seamPane(abilities = abilities))
                assertEquals(
                    if (offered) 1 else 0,
                    onAllNodesWithContentDescription("Attach a file", substring = true).fetchSemanticsNodes().size,
                    "$abilities",
                )
            }
        }
    }

    // LANDSCAPE_TOUCH was used on a portrait phone: 36 dp where the rule says 44.
    @Test
    fun controlsAreFullSizeExceptOnARotatedPhone() {
        for ((size, least) in listOf(PORTRAIT to 44.dp, LANDSCAPE to 36.dp)) {
            runComposeUiTest {
                seamView(size, seamPane(ServerMsg.ConvoTasks(PANE_ID, SEAM_TASKS)))
                val fold = onNodeWithContentDescription("Show the tasks").getUnclippedBoundsInRoot()
                assertTrue(fold.height >= least, "$size: ${fold.height}")
                if (least == 36.dp) assertTrue(fold.height < 44.dp, "landscape keeps its room: ${fold.height}")
            }
        }
    }
}
