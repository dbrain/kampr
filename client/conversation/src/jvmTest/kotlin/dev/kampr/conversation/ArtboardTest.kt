package dev.kampr.conversation

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.PhosphorTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.PaneScreenDesktop
import dev.kampr.shared.ui.PaneScreenMobile
import dev.kampr.shared.ui.PaneView
import java.io.File
import kotlin.test.Test
import kotlin.test.assertTrue

private val OUT = File("build/artboards")

class ArtboardTest {
    private fun mobile(landscape: Boolean): @Composable () -> Unit = {
        val (_, pane) = demoPane(RICH_CONVO)
        PaneScreenMobile(
            pane = pane,
            info = demoInfo(),
            view = PaneView.Conversation,
            surfaces = ConversationSurfaces(),
            landscape = landscape,
            readOnly = false,
            onBack = {},
            onView = {},
            onAnswer = {},
        )
    }

    @Test
    fun portraitArtboardRenders() {
        val image = renderArtboard(
            PORTRAIT.first, PORTRAIT.second, SoftTheme, TypeScale.Phone,
            File(OUT, "conversation-portrait.png"), content = mobile(landscape = false),
        )
        assertTrue(image.width == 780 && image.height == 1688, "${image.width}x${image.height}")
    }

    @Test
    fun landscapeArtboardRenders() {
        renderArtboard(
            LANDSCAPE.first, LANDSCAPE.second, SoftTheme, TypeScale.Phone,
            File(OUT, "conversation-landscape.png"), content = mobile(landscape = true),
        )
    }

    @Test
    fun desktopArtboardRenders() {
        renderArtboard(DESKTOP.first, DESKTOP.second, SoftTheme, TypeScale.Desk, File(OUT, "conversation-desktop.png")) {
            val (_, pane) = demoPane(RICH_CONVO)
            PaneScreenDesktop(
                pane = pane,
                info = demoInfo(),
                view = PaneView.Conversation,
                surfaces = ConversationSurfaces(),
                readOnly = false,
                onView = {},
                onAnswer = {},
            )
        }
    }

    // A second theme is the only thing that catches a colour that never went through the tokens.
    @Test
    fun portraitRendersInASecondTheme() {
        renderArtboard(
            PORTRAIT.first, PORTRAIT.second, PhosphorTheme, TypeScale.Phone,
            File(OUT, "conversation-portrait-phosphor.png"), content = mobile(landscape = false),
        )
    }

    @Test
    fun codexTranscriptRenders() {
        renderArtboard(PORTRAIT.first, PORTRAIT.second, SoftTheme, TypeScale.Phone, File(OUT, "conversation-codex.png")) {
            val (_, pane) = demoPane(CODEX_CONVO.replace("01JNODE.../w4:p1", PANE_ID))
            PaneScreenMobile(
                pane = pane,
                info = demoInfo(agent = "codex"),
                view = PaneView.Conversation,
                surfaces = ConversationSurfaces(),
                landscape = false,
                readOnly = false,
                onBack = {},
                onView = {},
                onAnswer = {},
            )
        }
    }

    // Visual evidence of a turn mid-flight: the preview and the mark that says it is one. The
    // assertion that the mark is *there* is a semantics one, in AccessibilityTest.
    @Test
    fun aLiveTurnRenders() {
        renderArtboard(PORTRAIT.first, 300.dp, SoftTheme, TypeScale.Phone, File(OUT, "conversation-live.png")) {
            val (_, pane) = demoPane(RICH_CONVO, LIVE_TURN)
            Box(Modifier.fillMaxSize().background(Kampr.tokens.color.bg).padding(16.dp)) {
                TurnView(
                    turn = pane.turns.first { it.id == LIVE_TURN_ID },
                    query = "",
                    expanded = emptyList(),
                    onToggle = {},
                    modifier = Modifier.fillMaxWidth(),
                )
            }
        }
    }

    @Test
    fun absentConversationRenders() {
        renderArtboard(PORTRAIT.first, PORTRAIT.second, SoftTheme, TypeScale.Phone, File(OUT, "conversation-absent.png")) {
            val (_, pane) = demoPane()
            PaneScreenMobile(
                pane = pane,
                info = demoInfo(agent = "aider", conversation = false),
                view = PaneView.Conversation,
                surfaces = ConversationSurfaces(),
                landscape = false,
                readOnly = false,
                onBack = {},
                onView = {},
                onAnswer = {},
            )
        }
    }
}
