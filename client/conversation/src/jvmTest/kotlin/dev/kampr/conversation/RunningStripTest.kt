package dev.kampr.conversation

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.runComposeUiTest
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
import kotlin.test.assertNull
import kotlin.test.assertTrue

private const val LAUNCHED = "2026-08-27T09:04:00.000Z"

// 09:06:11 against a launch at 09:04:00 — two minutes and eleven seconds.
private const val NOW = 1787821571000.0

private fun running(vararg entries: Running): PaneState {
    val store = KamprStore()
    store.accept(
        ServerMsg.Convo(
            pane = PANE_ID,
            cursor = "c-1",
            more = false,
            turns = listOf(Turn("c-1", "user", null, listOf(Block.Md("start the build")))),
        )
    )
    store.accept(ServerMsg.ConvoFacets(PANE_ID, Facets(running = entries.toList())))
    return store.pane(PANE_ID)
}

// The operator, on 0.1.49: *"i was expecting some representation in a static location ... because
// sometimes claude leaves shells open forever and 'working' can mean nothing but 'a shell was left
// running'"*.
@OptIn(ExperimentalTestApi::class)
class RunningStripTest {
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

    @Test
    fun theStripStandsAtTheFootOfTheViewWhileAnythingIsInFlight() = runComposeUiTest {
        setContent {
            CompositionLocalProvider(
                LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
                LocalPaneIo provides RecordingIo,
            ) {
                Box(Modifier.size(PORTRAIT.first, PORTRAIT.second)) {
                    ConversationView(
                        running(
                            Running("t1", "agent", "Agent", "close the width gaps", LAUNCHED),
                            Running("t2", "shell", "Bash", "the workspace build", LAUNCHED),
                        ),
                        demoInfo(),
                        Modifier.fillMaxSize(),
                        clock = { NOW },
                    )
                }
            }
        }
        waitForIdle()
        val strip = onNodeWithContentDescription("2 still running", substring = true).fetchSemanticsNode()
        val label = strip.config[SemanticsProperties.ContentDescription].first()
        assertTrue(label.contains("agent · close the width gaps, 2m 11s"), label)
        assertTrue(label.contains("shell · the workspace build, 2m 11s"), label)
    }

    // And it is gone when there is nothing in flight, rather than sitting there empty and taking a
    // band of a phone screen off the transcript.
    @Test
    fun aSessionWithNothingInFlightDrawsNoStripAtAll() = runComposeUiTest {
        setContent {
            CompositionLocalProvider(
                LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
                LocalPaneIo provides RecordingIo,
            ) {
                Box(Modifier.size(PORTRAIT.first, PORTRAIT.second)) {
                    ConversationView(running(), demoInfo(), Modifier.fillMaxSize(), clock = { NOW })
                }
            }
        }
        waitForIdle()
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
