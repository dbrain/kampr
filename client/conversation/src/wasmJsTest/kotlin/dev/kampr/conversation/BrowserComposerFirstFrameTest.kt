package dev.kampr.conversation

import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.v2.runComposeUiTest
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.theme.KamprFonts
import dev.kampr.shared.theme.KamprTokens
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.theme.typography
import dev.kampr.shared.ui.LocalConnectionStatus
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.ui.SafeArea
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.shared.wire.Wire
import kotlin.test.Test
import kotlin.test.assertEquals

private const val PANE = "01JKAMPRNODE0000000000000/n:01kamprconvo"

private object Quiet : PaneIo {
    override fun send(msg: ClientMsg) = Unit
    override fun prefs(paneId: String) = PanePrefs()
}

private val KEYS = """"keys":{"back":"\u007f","left":"\u001b[D","right":"\u001b[C","newline":"\n"}"""

private fun deskLine(text: String) = """{"t":"convo.composer","pane":"$PANE","text":"$text","clear":"\u0003","caret":${text.length},$KEYS}"""

// The browser report: a page opened straight onto a conversation trapped once, "dereferencing a
// null pointer" under a Compose dispatch, and typing in the reply box crawled from then on.
//
// Only Kotlin/Wasm shows it. The `$stable` the Compose plugin passes for `ComposerMirror` lived in
// `ComposerMirror.kt`'s lazy property initialiser, which nothing ran until the mirror first read
// its clock, so the reply box's first composition was told one thing about the mirror and the next
// another, read a different number of slots, and handed `LaunchedEffect` the wrong one. A JVM
// static initialiser runs before the read, which is why `OneLineTest` never saw it.
//
// The page restores the pane before the herd says what it is, so the view composes once with no
// `info` and again when it lands. A trap tears through Compose's dispatcher without running its
// `finally`, so the defect shows here as a test that never finishes rather than a failed assert.
//
// It has to be the first thing in this module's browser tests to touch `ComposerMirror.kt`.
@OptIn(ExperimentalTestApi::class)
class BrowserComposerFirstFrameTest {
    @Test
    fun aPageOpenedOnTheConversationStillTakesUpTheDesksLine() = runComposeUiTest {
        mainClock.autoAdvance = false
        val fonts = KamprFonts(FontFamily.Default, FontFamily.Monospace, FontFamily.Monospace)
        val tokens = KamprTokens(SoftTheme, fonts, typography(fonts, SoftTheme.label, TypeScale.Phone))
        val store = KamprStore()
        val pane = store.pane(PANE)
        var info by mutableStateOf<PaneInfo?>(null)
        setContent {
            CompositionLocalProvider(
                LocalTokens provides tokens,
                LocalPaneIo provides Quiet,
                LocalConnectionStatus provides ConnectionStatus.Live("full"),
                LocalSafeArea provides SafeArea(top = 0.dp, bottom = 0.dp),
            ) {
                ConversationView(pane, info, Modifier.fillMaxSize())
            }
        }
        repeat(10) { mainClock.advanceTimeByFrame() }
        info = PaneInfo(id = PANE, nodeId = "01JKAMPRNODE0000000000000", agent = "claude", agentStatus = "idle", hasConversation = true)
        repeat(10) { mainClock.advanceTimeByFrame() }

        store.accept(requireNotNull(Wire.decode(deskLine("push the branch"))))
        repeat(10) { mainClock.advanceTimeByFrame() }

        assertEquals(
            "push the branch",
            onNodeWithContentDescription("Reply to claude").fetchSemanticsNode().config[SemanticsProperties.EditableText].text,
        )
    }
}
