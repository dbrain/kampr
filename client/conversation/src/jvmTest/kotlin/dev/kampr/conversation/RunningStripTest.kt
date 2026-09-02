package dev.kampr.conversation

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.height
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.Facets
import dev.kampr.shared.wire.Running
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Turn
import dev.kampr.shared.wire.Wire
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotEquals
import kotlin.test.assertNull
import kotlin.test.assertTrue

private const val LAUNCHED = "2026-08-27T09:04:00.000Z"

// 09:06:11 against a launch at 09:04:00 — two minutes and eleven seconds.
private const val NOW = 1787821571000.0

// The end of an answer that is longer than the screen, which is the only shape that can be sat
// on: a transcript short enough to fit leaves its last line nowhere near the foot, and a strip
// covering nothing proves nothing.
private const val LAST_LINE = "That paragraph is the end of the newest thing the agent said."

private fun tallAnswer(): String =
    (1..40).joinToString("\n\n") { "Paragraph $it of an answer that runs well past one screen." } +
        "\n\n" + LAST_LINE

private fun running(vararg entries: Running, tall: Boolean = false): PaneState =
    paneWithStore(*entries, tall = tall).second

private fun paneWithStore(vararg entries: Running, tall: Boolean = false): Pair<KamprStore, PaneState> {
    val store = KamprStore()
    store.accept(
        ServerMsg.Convo(
            pane = PANE_ID,
            cursor = "c-1",
            more = false,
            turns = listOfNotNull(
                Turn("c-1", "user", null, listOf(Block.Md("start the build"))),
                if (tall) Turn("a-1", "assistant", null, listOf(Block.Md(tallAnswer()))) else null,
            ),
        )
    )
    store.accept(ServerMsg.ConvoFacets(PANE_ID, Facets(running = entries.toList())))
    return store to store.pane(PANE_ID)
}

private val TWO = arrayOf(
    Running("t1", "agent", "Agent", "close the width gaps", LAUNCHED),
    Running("t2", "shell", "Bash", "the workspace build", LAUNCHED),
)

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.screen(pane: PaneState) {
    setContent {
        CompositionLocalProvider(
            LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
            LocalPaneIo provides RecordingIo,
        ) {
            Box(Modifier.size(PORTRAIT.first, PORTRAIT.second)) {
                ConversationView(pane, demoInfo(), Modifier.fillMaxSize(), clock = { NOW })
            }
        }
    }
    waitForIdle()
}

private const val OPEN = "Show what is running"

private const val SHUT = "Hide what is running"

// The operator, on 0.1.49: *"i was expecting some representation in a static location ... because
// sometimes claude leaves shells open forever and 'working' can mean nothing but 'a shell was left
// running'"*.
@OptIn(ExperimentalTestApi::class)
class RunningStripTest {
    private fun ComposeUiTest.spoken(): String =
        onNodeWithContentDescription("still running", substring = true)
            .fetchSemanticsNode().config[SemanticsProperties.ContentDescription].first()

    @Test
    fun whatIsStillRunningIsNamedWithAStopwatchRatherThanAnAge() {
        assertEquals(
            "2m 11s",
            runningSince(Running(call = "t1", kind = "shell", since = LAUNCHED), NOW),
            "an age would say 2m and then not move for the next forty-nine seconds",
        )
        assertEquals(
            "shell · the workspace build",
            runningLabel(Running(call = "t1", kind = "shell", name = "Bash", title = "the workspace build")),
        )
        assertEquals(
            "agent",
            runningLabel(Running(call = "t2", kind = "agent", name = "Agent")),
            "a launch the harness gave no description keeps its kind rather than inventing one",
        )
    }

    // `kind` is an open string on the wire on purpose. A word this release has never heard of is
    // the harness's own, and printing it is the only honest thing to do with it.
    @Test
    fun aKindThisReleaseHasNeverHeardOfIsPrintedRatherThanDefaulted() {
        assertEquals(
            "sandbox · pack the image",
            runningLabel(Running(call = "t3", kind = "sandbox", title = "pack the image")),
        )
        assertEquals(
            "Bash · a task",
            runningLabel(Running(call = "t4", kind = "", name = "Bash", title = "a task")),
        )
        assertNull(runningSince(Running(call = "t5", kind = "shell", since = null), NOW))
    }

    // Opened, every launch is named with a stopwatch of its own — which is what the operator
    // opens it for, and what the count alone cannot say.
    @Test
    fun anOpenStripNamesEveryLaunchWithAStopwatchOfItsOwn() = runComposeUiTest {
        screen(running(*TWO))
        onNodeWithContentDescription(OPEN).performClick()
        waitForIdle()
        val label = spoken()
        assertTrue(label.contains("agent · close the width gaps, 2m 11s"), label)
        assertTrue(label.contains("shell · the workspace build, 2m 11s"), label)
    }

    // Two launches is two rows over the transcript and eight is a wall of them, so the strip opens
    // shut and the count — the one thing it exists to say — is what stands there. The rows are one
    // press away and nothing about the press reaches the pane.
    @Test
    fun theStripOpensFoldedToItsCountAndTheRowsAreOnePressAway() = runComposeUiTest {
        screen(running(*TWO))
        RecordingIo.sent.clear()
        assertTrue(
            onAllNodesWithText("the workspace build", substring = true, useUnmergedTree = true)
                .fetchSemanticsNodes().isEmpty(),
            "the strip drew every row before anybody asked it to",
        )
        onNodeWithText("2 still running", ignoreCase = true, useUnmergedTree = true).assertIsDisplayed()

        onNodeWithContentDescription(OPEN).performClick()
        waitForIdle()
        onNodeWithText("shell · the workspace build", useUnmergedTree = true).assertIsDisplayed()
        onNodeWithText("2 still running", ignoreCase = true, useUnmergedTree = true).assertIsDisplayed()

        onNodeWithContentDescription(SHUT).performClick()
        waitForIdle()
        assertTrue(
            onAllNodesWithText("the workspace build", substring = true, useUnmergedTree = true)
                .fetchSemanticsNodes().isEmpty(),
            "the rows stayed up after the strip was folded again",
        )
        assertEquals(emptyList(), RecordingIo.sent, "folding the strip wrote to the pane")
    }

    // A fold takes the names off the screen and it must not take them off the accessibility line
    // with them: a reader with no eyes on a folded strip is told at least what a reader with eyes
    // on it can see, which is the count, and here rather more.
    @Test
    fun aFoldedStripStillNamesWhatIsRunning() = runComposeUiTest {
        screen(running(*TWO))
        val label = spoken()
        assertTrue(label.startsWith("2 still running, folded: "), label)
        assertTrue(label.contains("agent · close the width gaps"), label)
        assertTrue(label.contains("shell · the workspace build"), label)
    }

    // One launch has nothing to fold: the row names it and carries its stopwatch, which is more
    // than the count says, so it is on the screen without anybody pressing anything and there is
    // no chevron offering to hide it.
    @Test
    fun aSingleLaunchIsDrawnInFullWithNothingToPress() = runComposeUiTest {
        screen(running(Running("t2", "shell", "Bash", "the workspace build", LAUNCHED)))
        onNodeWithText("shell · the workspace build", useUnmergedTree = true).assertIsDisplayed()
        onNodeWithText("2m 11s", useUnmergedTree = true).assertIsDisplayed()
        assertEquals(
            0,
            onAllNodesWithContentDescription("what is running", substring = true).fetchSemanticsNodes().size,
            "a chevron offered to hide the only thing the strip was there to say",
        )
    }

    // Reported from a wasm desktop: *"it covers the bottom of the conversation text"*. The strip is
    // drawn over the transcript, so the transcript has to be handed that much of its own box — the
    // same treatment the question card above it gets, and for the same reason.
    @Test
    fun theEndOfTheTranscriptIsNotUnderTheStrip() = runComposeUiTest {
        screen(running(*TWO, tall = true))
        assertTrue(
            onAllNodesWithText(LAST_LINE, substring = true).fetchSemanticsNodes().isNotEmpty(),
            "the transcript never reached its own end, so nothing here proves anything",
        )
        val strip = onNodeWithContentDescription("still running", substring = true).getUnclippedBoundsInRoot()
        val end = onNodeWithText(LAST_LINE, substring = true).getUnclippedBoundsInRoot()
        assertTrue(
            end.bottom <= strip.top,
            "the last line ends at ${end.bottom}, under a strip that starts at ${strip.top}",
        )

        // And opening it, which is the reader growing the thing that stands on the transcript.
        onNodeWithContentDescription(OPEN).performClick()
        waitForIdle()
        val open = onNodeWithContentDescription("still running", substring = true).getUnclippedBoundsInRoot()
        val stillThere = onNodeWithText(LAST_LINE, substring = true).getUnclippedBoundsInRoot()
        assertTrue(
            stillThere.bottom <= open.top,
            "opening the strip pulled the last line under it: ${stillThere.bottom} against ${open.top}",
        )
    }

    // The band belongs to a strip that is standing there. Nothing measures a composable that
    // returned before it drew anything, so the last size a strip reported is the size it keeps
    // once it is gone — and the transcript would have paid a band of its own foot to a strip that
    // was not there any more, for the rest of the pane's life.
    @Test
    fun theBandGoesBackToTheTranscriptWhenTheLastLaunchFinishes() = runComposeUiTest {
        val (store, pane) = paneWithStore(*TWO, tall = true)
        screen(pane)
        val strip = onNodeWithContentDescription("still running", substring = true).getUnclippedBoundsInRoot()
        val before = onNodeWithText(LAST_LINE, substring = true).getUnclippedBoundsInRoot()

        store.accept(ServerMsg.ConvoFacets(PANE_ID, Facets()))
        waitForIdle()
        assertEquals(
            0,
            onAllNodesWithContentDescription("still running", substring = true).fetchSemanticsNodes().size,
        )
        val after = onNodeWithText(LAST_LINE, substring = true).getUnclippedBoundsInRoot()
        assertTrue(
            after.bottom - before.bottom >= strip.height,
            "the strip left ${after.bottom - before.bottom} of its ${strip.height} behind it",
        )
    }

    // An hour-old launch is the one somebody is watching hardest, and it was the one thing on the
    // screen that stood still: "1h 5m" moves once a minute, which reads as a stopped clock.
    @Test
    fun aLaunchOlderThanAnHourStillMovesEverySecond() {
        val run = Running(call = "t1", kind = "shell", since = LAUNCHED)
        assertEquals("1h 02m 11s", runningSince(run, NOW + 3_600_000))
        assertNotEquals(
            runningSince(run, NOW + 3_600_000),
            runningSince(run, NOW + 3_601_000),
            "a second passed on an hour-old launch and the row said the same thing",
        )
        assertEquals(
            runningSince(run, NOW + 3_600_000)?.length,
            runningSince(run, NOW + 3_660_000)?.length,
            "the row changes width as it ticks, so the layout dances",
        )
    }

    // And it is gone when there is nothing in flight, rather than sitting there empty and taking a
    // band of a phone screen off the transcript.
    @Test
    fun aSessionWithNothingInFlightDrawsNoStripAtAll() = runComposeUiTest {
        screen(running())
        assertEquals(
            0,
            onAllNodesWithContentDescription("still running", substring = true).fetchSemanticsNodes().size,
        )
    }

    // The wire half: `running` is a new optional field, so a node that has never heard of it sends
    // a facets frame exactly as it does today and the client draws nothing.
    @Test
    fun aNodeThatSendsNoRunningFieldIsReadAsNothingRunning() {
        val frame = Wire.decode(
            """{"t":"convo.facets","pane":"$PANE_ID","facets":{"queued":[{"text":"next"}]}}"""
        ) as ServerMsg.ConvoFacets
        assertEquals(emptyList(), frame.facets.running)
        assertEquals(1, frame.facets.queued.size)

        val full = Wire.decode(
            """{"t":"convo.facets","pane":"$PANE_ID","facets":{"running":[
               {"call":"toolu_1","kind":"shell","name":"Bash","title":"the build","since":"$LAUNCHED"},
               {"call":"toolu_2","kind":"agent"}]}}"""
        ) as ServerMsg.ConvoFacets
        assertEquals(listOf("toolu_1", "toolu_2"), full.facets.running.map { it.call })
        assertEquals("the build", full.facets.running[0].title)
        assertNull(full.facets.running[1].title)
    }
}
