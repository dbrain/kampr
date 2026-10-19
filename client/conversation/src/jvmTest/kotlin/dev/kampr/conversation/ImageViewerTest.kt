package dev.kampr.conversation

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.InternalComposeUiApi
import androidx.compose.ui.Modifier
import androidx.compose.ui.backhandler.BackHandler
import androidx.compose.ui.backhandler.LocalCompatNavigationEventDispatcherOwner
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.pinch
import androidx.compose.ui.test.runComposeUiTest
import androidx.navigationevent.NavigationEventDispatcher
import androidx.navigationevent.NavigationEventDispatcherOwner
import androidx.navigationevent.NavigationEventInput
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.net.AttachmentBytes
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Turn
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private const val SAID = "Here is what the pane looked like."

private fun paneWithAPicture(): PaneState {
    val store = KamprStore()
    store.accept(
        ServerMsg.Convo(
            pane = PANE_ID,
            cursor = "a-1",
            more = false,
            turns = listOf(
                Turn("u-0", "user", "2026-08-24T09:00:00.000Z", listOf(Block.Md("show me"))),
                Turn(
                    "a-1",
                    "assistant",
                    "2026-08-24T09:00:06.000Z",
                    listOf(Block.Md(SAID), Block.Md(MARKER, SHOT)),
                ),
            ),
        ),
    )
    return store.pane(PANE_ID)
}

private const val VIEWER = "shot.png, pinch to zoom, double tap to fit"

// Android's back, delivered the way the platform delivers it, to whatever in the composition last
// claimed it.
private class BackWindow : NavigationEventDispatcherOwner {
    override val navigationEventDispatcher = NavigationEventDispatcher()
    private val button = object : NavigationEventInput() {
        fun press() = dispatchOnBackCompleted()
    }.also { navigationEventDispatcher.addInput(it) }

    fun press() = button.press()
}

// The app's own back, which leaves the pane for the herd. Registered outside the conversation, as
// `SystemBack` is, so anything inside that claims the gesture is asked first.
@OptIn(ExperimentalTestApi::class, ExperimentalComposeUiApi::class, InternalComposeUiApi::class)
private fun ComposeUiTest.openTheShot(width: Int, height: Int, window: BackWindow = BackWindow(), onLeave: () -> Unit = {}) {
    val node = NodeWithAttachments(mapOf(SHOT.id to AttachmentBytes.Ok(pngBytes(width, height), "image/png")))
    setContent {
        CompositionLocalProvider(
            LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
            LocalPaneIo provides node,
            LocalCompatNavigationEventDispatcherOwner provides window,
        ) {
            BackHandler(onBack = onLeave)
            Box(Modifier.size(PORTRAIT.first, PORTRAIT.second)) {
                ConversationView(paneWithAPicture(), demoInfo(), Modifier.fillMaxSize())
            }
        }
    }
    waitForIdle()
    onNodeWithContentDescription("Show image, shot.png").performClick()
    waitUntil(timeoutMillis = 5_000) {
        onAllNodesWithContentDescription("Open shot.png").fetchSemanticsNodes().isNotEmpty()
    }
    onNodeWithContentDescription("Open shot.png").performClick()
    waitForIdle()
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.viewerState(): String = onNodeWithContentDescription(VIEWER).fetchSemanticsNode().config[SemanticsProperties.StateDescription]

@OptIn(ExperimentalTestApi::class)
class ImageViewerTest {
    @Test
    fun backClosesThePictureAndLeavesThePaneOpen() = runComposeUiTest {
        val window = BackWindow()
        var left = 0
        openTheShot(900, 1400, window) { left++ }
        onNodeWithContentDescription(VIEWER).assertIsDisplayed()

        window.press()
        waitForIdle()
        assertTrue(
            onAllNodesWithContentDescription(VIEWER).fetchSemanticsNodes().isEmpty(),
            "back left the picture open",
        )
        assertEquals(0, left, "back went past the picture and left the pane")

        window.press()
        waitForIdle()
        assertEquals(1, left, "with nothing open, back is the pane's again")
    }

    @Test
    fun twoFingersSpreadingApartZoomThePictureIn() = runComposeUiTest {
        openTheShot(900, 1400)
        assertEquals("fitted", viewerState())
        onNodeWithContentDescription(VIEWER).performTouchInput {
            pinch(
                center - Offset(20f, 0f),
                center - Offset(200f, 0f),
                center + Offset(20f, 0f),
                center + Offset(200f, 0f),
            )
        }
        waitForIdle()
        assertTrue(viewerState().startsWith("zoomed"), "a pinch left the picture ${viewerState()}")
    }

    // A thumbnail exists to be opened. A screenshot of a 292-column pane is unreadable at 64 dp,
    // which is the whole reason it is 64 dp — the reading happens somewhere the picture has the
    // screen to itself and a gesture to make it bigger.
    @Test
    fun aThumbnailOpensThePictureOverTheWholePane() = runComposeUiTest {
        val node = NodeWithAttachments(mapOf(SHOT.id to AttachmentBytes.Ok(pngBytes(900, 1400), "image/png")))
        setContent {
            CompositionLocalProvider(
                LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
                LocalPaneIo provides node,
            ) {
                Box(Modifier.size(PORTRAIT.first, PORTRAIT.second)) {
                    ConversationView(paneWithAPicture(), demoInfo(), Modifier.fillMaxSize())
                }
            }
        }
        waitForIdle()
        onNodeWithContentDescription("Show image, shot.png").performClick()
        waitUntil(timeoutMillis = 5_000) {
            onAllNodesWithContentDescription("Open shot.png").fetchSemanticsNodes().isNotEmpty()
        }
        // The transcript is on screen behind it, and the picture is not open yet.
        onAllNodesWithText(SAID, substring = true).fetchSemanticsNodes().let {
            assertTrue(it.isNotEmpty(), "the transcript was not showing before the picture was opened")
        }

        onNodeWithContentDescription("Open shot.png").performClick()
        waitForIdle()
        onNodeWithContentDescription("shot.png, pinch to zoom, double tap to fit").assertIsDisplayed()
        assertTrue(
            onAllNodesWithText(SAID, substring = true).fetchSemanticsNodes().isEmpty(),
            "the transcript was still composed underneath a picture covering the whole pane",
        )

        onNodeWithContentDescription("Close shot.png").performClick()
        waitForIdle()
        assertTrue(
            onAllNodesWithText(SAID, substring = true).fetchSemanticsNodes().isNotEmpty(),
            "closing the picture did not give the transcript back",
        )
    }

    // The bytes are already in hand, so the device gets the file the reader is looking at rather
    // than a second authorised fetch of it.
    @Test
    fun theOpenPictureCanBeHandedToTheDevice() = runComposeUiTest {
        val node = NodeWithAttachments(mapOf(SHOT.id to AttachmentBytes.Ok(pngBytes(64, 64), "image/png")))
        setContent {
            CompositionLocalProvider(
                LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
                LocalPaneIo provides node,
            ) {
                Box(Modifier.size(PORTRAIT.first, PORTRAIT.second)) {
                    ConversationView(paneWithAPicture(), demoInfo(), Modifier.fillMaxSize())
                }
            }
        }
        waitForIdle()
        onNodeWithContentDescription("Show image, shot.png").performClick()
        waitUntil(timeoutMillis = 5_000) {
            onAllNodesWithContentDescription("Open shot.png").fetchSemanticsNodes().isNotEmpty()
        }
        onNodeWithContentDescription("Open shot.png").performClick()
        waitForIdle()
        onNodeWithContentDescription("Save shot.png to this device").assertIsDisplayed()
        assertTrue(node.asked.size == 1, "opening the picture fetched it a second time: ${node.asked}")
    }
}
