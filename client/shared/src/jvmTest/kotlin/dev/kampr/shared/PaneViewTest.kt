package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.theme.Ground
import dev.kampr.shared.theme.KamprFonts
import dev.kampr.shared.theme.KamprTokens
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.theme.on
import dev.kampr.shared.theme.themeOf
import dev.kampr.shared.theme.typography
import dev.kampr.shared.ui.PaneScreenDesktop
import dev.kampr.shared.ui.PaneScreenMobile
import dev.kampr.shared.ui.PaneSurfaces
import dev.kampr.shared.ui.PaneView
import dev.kampr.shared.ui.named
import dev.kampr.shared.wire.PaneInfo
import kotlin.test.Test
import kotlin.test.assertEquals

private const val PANE_ID = "01JNODE/w1:p1"

private const val TERMINAL_SURFACE = "Terminal surface probe"
private const val CONVERSATION_SURFACE = "Conversation surface probe"
private const val ZOOM = "Zoom probe"

private const val TERMINAL_TAB = "Terminal view"
private const val CONVERSATION_TAB = "Conversation view"
private const val SPLIT_TAB = "Split view"

private fun pane(conversation: Boolean) = PaneInfo(
    id = PANE_ID,
    nodeId = "01JNODE",
    workspace = "kampr",
    cwd = "/home/dbrain/dev/kampr",
    agent = if (conversation) "claude" else null,
    agentStatus = "idle",
    cols = 94,
    rows = 40,
    hasConversation = conversation,
)

private class SurfaceProbe : PaneSurfaces {
    @Composable
    override fun Terminal(pane: PaneState, info: PaneInfo?, modifier: Modifier) =
        Box(modifier.named(TERMINAL_SURFACE))

    @Composable
    override fun Conversation(pane: PaneState, info: PaneInfo?, modifier: Modifier) =
        Box(modifier.named(CONVERSATION_SURFACE))

    @Composable
    override fun KeyRow(pane: PaneState, compact: Boolean, modifier: Modifier) = Box(modifier)

    @Composable
    override fun Zoom(pane: PaneState, modifier: Modifier) = Box(modifier.size(40.dp).named(ZOOM))
}

private fun tokens() = themeOf("soft").on(Ground.Dark).let { spec ->
    val fonts = KamprFonts(FontFamily.Default, FontFamily.Monospace, FontFamily.Monospace)
    KamprTokens(spec, fonts, typography(fonts, spec.label, TypeScale.Phone))
}

@Composable
private fun Themed(content: @Composable () -> Unit) {
    CompositionLocalProvider(LocalTokens provides tokens(), content = content)
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.mobile(width: Dp, height: Dp, landscape: Boolean, view: PaneView, conversation: Boolean) {
    setContent {
        Themed {
            Box(Modifier.size(width, height)) {
                PaneScreenMobile(
                    pane = PaneState(PANE_ID, StyleTable()),
                    info = pane(conversation),
                    view = view,
                    surfaces = SurfaceProbe(),
                    landscape = landscape,
                    readOnly = false,
                    onBack = {},
                    onView = {},
                    onAnswer = {},
                    modifier = Modifier.fillMaxSize(),
                )
            }
        }
    }
    waitForIdle()
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.desktop(view: PaneView, conversation: Boolean) {
    setContent {
        Themed {
            Box(Modifier.size(1280.dp, 860.dp)) {
                PaneScreenDesktop(
                    pane = PaneState(PANE_ID, StyleTable()),
                    info = pane(conversation),
                    view = view,
                    surfaces = SurfaceProbe(),
                    readOnly = false,
                    onView = {},
                    onAnswer = {},
                    modifier = Modifier.fillMaxSize(),
                )
            }
        }
    }
    waitForIdle()
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.count(label: String): Int =
    onAllNodesWithContentDescription(label).fetchSemanticsNodes().size

// `has_conversation` means "a transcript actually resolves", not "this pane is an agent". A shell —
// and an agent whose transcript has not appeared yet — must not be offered a view of nothing, and
// must not be dropped into one.
@OptIn(ExperimentalTestApi::class)
class PaneViewTest {
    @Test
    fun aShellPaneIsOfferedNoConversationTabPortrait() = runComposeUiTest {
        mobile(411.dp, 914.dp, landscape = false, view = PaneView.Terminal, conversation = false)
        assertEquals(0, count(CONVERSATION_TAB), "a shell pane offered a Conversation tab")
        assertEquals(0, count(TERMINAL_TAB), "a one-sided switcher is not a switcher")
        // Everything else in the second row has to survive the switcher going away.
        onNodeWithContentDescription(ZOOM).assertExists()
        onNodeWithContentDescription("Back to the herd").assertExists()
    }

    @Test
    fun anAgentPaneWithATranscriptStillGetsBothTabs() = runComposeUiTest {
        mobile(411.dp, 914.dp, landscape = false, view = PaneView.Terminal, conversation = true)
        onNodeWithContentDescription(TERMINAL_TAB).assertExists()
        onNodeWithContentDescription(CONVERSATION_TAB).assertExists()
    }

    @Test
    fun aShellPaneIsOfferedNoConversationTabLandscape() = runComposeUiTest {
        mobile(914.dp, 411.dp, landscape = true, view = PaneView.Terminal, conversation = false)
        assertEquals(0, count(CONVERSATION_TAB), "landscape offered a Conversation tab on a shell")
        onNodeWithContentDescription(ZOOM).assertExists()
    }

    // The desktop sidebar opens every pane in Split, so a shell landed in a two-column layout whose
    // right half only ever said there was nothing to put in it.
    @Test
    fun aShellPaneOnTheDesktopIsNeitherSplitNorOfferedTheChoice() = runComposeUiTest {
        desktop(view = PaneView.Split, conversation = false)
        assertEquals(0, count(SPLIT_TAB), "a shell pane offered a Split tab")
        assertEquals(0, count(CONVERSATION_TAB), "a shell pane offered a Conversation tab")
        assertEquals(0, count(CONVERSATION_SURFACE), "the conversation half was rendered anyway")
        onNodeWithContentDescription(TERMINAL_SURFACE).assertIsDisplayed()
    }

    @Test
    fun anAgentPaneOnTheDesktopKeepsAllThree() = runComposeUiTest {
        desktop(view = PaneView.Split, conversation = true)
        onNodeWithContentDescription(SPLIT_TAB).assertExists()
        onNodeWithContentDescription(TERMINAL_TAB).assertExists()
        onNodeWithContentDescription(CONVERSATION_TAB).assertExists()
        onNodeWithContentDescription(CONVERSATION_SURFACE).assertExists()
    }

    // A remembered preference, a deep link and the desktop's own default can all ask for a view
    // this pane has nothing to put in. The pane it names is still live, so the terminal is where
    // the operator goes — not an empty transcript.
    @Test
    fun aConversationAskedForOnAShellRendersTheTerminalInstead() = runComposeUiTest {
        mobile(411.dp, 914.dp, landscape = false, view = PaneView.Conversation, conversation = false)
        assertEquals(0, count(CONVERSATION_SURFACE), "a shell pane rendered a transcript view")
        onNodeWithContentDescription(TERMINAL_SURFACE).assertIsDisplayed()
    }

    // The desktop layout is the one a browser lands in, and the only one that carries no key row
    // — so nothing else on it can reach the zoom. Its header did not offer one at all.
    @Test
    fun theDesktopPaneOffersTheSameZoomThePhoneDoes() = runComposeUiTest {
        desktop(view = PaneView.Terminal, conversation = false)
        onNodeWithContentDescription(ZOOM).assertExists()
    }

    @Test
    fun aSplitDesktopPaneKeepsItsZoomBecauseTheTerminalHalfIsStillThere() = runComposeUiTest {
        desktop(view = PaneView.Split, conversation = true)
        onNodeWithContentDescription(ZOOM).assertExists()
    }

    // The zoom sheet is drawn by the terminal surface, so a zoom control on a screen showing only
    // the transcript is a control that opens nothing.
    @Test
    fun aPaneShowingOnlyItsTranscriptOffersNoZoom() = runComposeUiTest {
        mobile(411.dp, 914.dp, landscape = false, view = PaneView.Conversation, conversation = true)
        assertEquals(0, count(ZOOM), "a zoom control was offered with no terminal surface to zoom")
    }

    @Test
    fun aPaneShowingOnlyItsTranscriptOffersNoZoomInLandscapeEither() = runComposeUiTest {
        mobile(914.dp, 411.dp, landscape = true, view = PaneView.Conversation, conversation = true)
        assertEquals(0, count(ZOOM), "a zoom control was offered with no terminal surface to zoom")
    }

    @Test
    fun aDesktopPaneShowingOnlyItsTranscriptOffersNoZoom() = runComposeUiTest {
        desktop(view = PaneView.Conversation, conversation = true)
        assertEquals(0, count(ZOOM), "a zoom control was offered with no terminal surface to zoom")
    }

    // Reported as a mode: "wasm opening a terminal seems to go into observing mode rather than
    // letting me type". It never was one — the word is what the meta line says when nobody else
    // has the pane open, which is almost always, so it said nothing and read as read-only. #129
    // measured that this line ellipsises at `… · observ…` on a phone; the constant it was keeping
    // room for is the half that was never news.
    @Test
    fun aPaneNobodyElseIsWatchingSaysNothingAboutBeingWatched() = runComposeUiTest {
        mobile(411.dp, 914.dp, landscape = false, view = PaneView.Terminal, conversation = false)
        assertEquals(
            0,
            onAllNodesWithText("observing", substring = true).fetchSemanticsNodes().size,
            "the meta line called a lone operator an observer",
        )
    }

    @Test
    fun aDesktopPaneNobodyElseIsWatchingSaysNothingAboutBeingWatchedEither() = runComposeUiTest {
        desktop(view = PaneView.Terminal, conversation = false)
        assertEquals(
            0,
            onAllNodesWithText("observing", substring = true).fetchSemanticsNodes().size,
            "the meta line called a lone operator an observer",
        )
    }
}

// The value moves during a session: a shell becomes an agent when somebody runs `claude` in it, and
// the transcript resolves seconds after that. Both directions have to land without a reconnect.
@OptIn(ExperimentalTestApi::class)
class PaneViewTransitionTest {
    @Test
    fun theTabAppearsWhenATranscriptResolvesAndTheAskedForViewComesBack() = runComposeUiTest {
        var conversation by mutableStateOf(false)
        setContent {
            Themed {
                Box(Modifier.size(411.dp, 914.dp)) {
                    PaneScreenMobile(
                        pane = PaneState(PANE_ID, StyleTable()),
                        info = pane(conversation),
                        // What the operator last asked for, kept across the change.
                        view = PaneView.Conversation,
                        surfaces = SurfaceProbe(),
                        landscape = false,
                        readOnly = false,
                        onBack = {},
                        onView = {},
                        onAnswer = {},
                        modifier = Modifier.fillMaxSize(),
                    )
                }
            }
        }
        waitForIdle()
        assertEquals(0, count(CONVERSATION_TAB), "no transcript yet, but a tab was offered")
        onNodeWithContentDescription(TERMINAL_SURFACE).assertIsDisplayed()

        conversation = true
        waitForIdle()
        onNodeWithContentDescription(CONVERSATION_TAB).assertExists()
        onNodeWithContentDescription(CONVERSATION_SURFACE).assertIsDisplayed()
    }

    @Test
    fun anOperatorSittingInConversationIsMovedToTheTerminalWhenItGoesAway() = runComposeUiTest {
        var conversation by mutableStateOf(true)
        setContent {
            Themed {
                Box(Modifier.size(411.dp, 914.dp)) {
                    PaneScreenMobile(
                        pane = PaneState(PANE_ID, StyleTable()),
                        info = pane(conversation),
                        view = PaneView.Conversation,
                        surfaces = SurfaceProbe(),
                        landscape = false,
                        readOnly = false,
                        onBack = {},
                        onView = {},
                        onAnswer = {},
                        modifier = Modifier.fillMaxSize(),
                    )
                }
            }
        }
        waitForIdle()
        onNodeWithContentDescription(CONVERSATION_SURFACE).assertIsDisplayed()

        conversation = false
        waitForIdle()
        assertEquals(0, count(CONVERSATION_SURFACE), "left staring at a view with nothing in it")
        assertEquals(0, count(CONVERSATION_TAB), "the tab outlived the transcript")
        onNodeWithContentDescription(TERMINAL_SURFACE).assertIsDisplayed()
    }
}
