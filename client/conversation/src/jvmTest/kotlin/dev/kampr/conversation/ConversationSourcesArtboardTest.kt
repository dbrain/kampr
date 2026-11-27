package dev.kampr.conversation

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.asSkiaBitmap
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.v2.runSkikoComposeUiTest
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.theme.Ground
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.LocalConnectionStatus
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.SafeArea
import dev.kampr.shared.wire.Abilities
import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.Control
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.SessionFacts
import dev.kampr.shared.wire.Turn
import org.jetbrains.skia.EncodedImageFormat
import org.jetbrains.skia.Image
import java.io.File
import kotlin.test.Test

private val OUT = File("build/artboards/sources")

private enum class SourceFrame(val width: Int, val height: Int, val density: Float, val scale: TypeScale, val bars: SafeArea) {
    Portrait(390, 844, 2.625f, TypeScale.Phone, SafeArea(top = 44.dp, bottom = 46.dp)),
    Desktop(1280, 800, 1.5f, TypeScale.Desk, SafeArea(0.dp, 0.dp)),
}

// Headless Claude: the full control surface.
private val HEADLESS_ABILITIES = Abilities(
    input = listOf("slash", "mention", "queue_next", "send_now", "unqueue", "files", "images"),
    steer = listOf("interrupt"),
    asks = listOf("permission", "always", "edit_input", "question", "free_text", "plan"),
    streams = listOf("text", "task_output", "sub_live"),
    facts = listOf("account", "billing", "limits", "context"),
    lifecycle = listOf("end"),
)

private val HEADLESS_CONTROLS = listOf(
    Control(
        id = "model",
        label = "Model",
        kind = "select",
        options = listOf(
            dev.kampr.shared.wire.ControlOption("sonnet", "Claude Sonnet"),
            dev.kampr.shared.wire.ControlOption("haiku", "Claude Haiku"),
            dev.kampr.shared.wire.ControlOption("opus", "Claude Opus"),
        ),
    ),
    Control(
        id = "permission",
        label = "Permissions",
        kind = "select",
        options = listOf(
            dev.kampr.shared.wire.ControlOption("default", "Default"),
            dev.kampr.shared.wire.ControlOption("bypassPermissions", "Bypass"),
        ),
    ),
)

// Codex: minimal surface — text stream, interrupt, queue.
private val CODEX_ABILITIES = Abilities(
    input = listOf("queue_next"),
    steer = listOf("interrupt"),
    asks = listOf("permission"),
    streams = listOf("text"),
    facts = listOf("limits"),
    lifecycle = listOf("end"),
)

// Muffit: medium surface — text, interrupt, permission asks.
private val MUFFIT_ABILITIES = Abilities(
    input = listOf("slash"),
    steer = listOf("interrupt"),
    asks = listOf("permission", "free_text"),
    streams = listOf("text"),
    facts = emptyList(),
    lifecycle = listOf("end"),
)

// Terminal Claude: the TUI's surface — no input (the terminal handles it), full asks.
private val TERMINAL_ABILITIES = Abilities(
    input = emptyList(),
    steer = emptyList(),
    asks = listOf("permission", "always", "edit_input", "question", "free_text", "plan"),
    streams = listOf("text", "task_output", "sub_live"),
    facts = listOf("trouble"),
    lifecycle = emptyList(),
)

private fun sourceStore(
    abilities: Abilities,
    controls: List<Control> = emptyList(),
    agent: String = "claude",
    turns: List<Turn> = listOf(
        Turn("h-1", "user", null, listOf(Block.Md("What can you do?"))),
        Turn("h-2", "assistant", null, listOf(Block.Md("I'm ready. Ask me anything."))),
    ),
): KamprStore = KamprStore().also { store ->
    store.accept(
        ServerMsg.Convo(PANE_ID, "h-2", false, turns),
    )
    store.accept(ServerMsg.ConvoAbilities(PANE_ID, abilities))
    if (controls.isNotEmpty()) {
        store.accept(ServerMsg.ConvoControls(PANE_ID, controls))
    }
    store.accept(
        ServerMsg.ConvoSession(
            PANE_ID,
            SessionFacts(
                id = "s1",
                cwd = "/tmp",
                model = "sonnet",
            ),
        ),
    )
    // A herd frame so the pane info is present.
    store.accept(
        ServerMsg.Herd(
            nodes = emptyList(),
            panes = listOf(
                dev.kampr.shared.wire.PaneInfo(
                    id = PANE_ID,
                    nodeId = "01JNODE",
                    workspace = "demo",
                    tab = "1",
                    cwd = "/tmp",
                    agent = agent,
                    agentStatus = "idle",
                    cols = 80,
                    rows = 24,
                    hasConversation = true,
                    converses = true,
                ),
            ),
        ),
    )
}

@OptIn(ExperimentalTestApi::class)
private fun shoot(name: String, frame: SourceFrame, content: @Composable () -> Unit) {
    val file = File(OUT, "$name-${frame.name.lowercase()}.png")
    runSkikoComposeUiTest(Size(frame.width * frame.density, frame.height * frame.density), Density(frame.density)) {
        mainClock.autoAdvance = false
        setContent {
            CompositionLocalProvider(
                LocalTokens provides tokensFor(SoftTheme, frame.scale, Ground.Dark),
                LocalSafeArea provides frame.bars,
                LocalPaneIo provides RecordingIo,
                LocalConnectionStatus provides ConnectionStatus.Live("full"),
            ) {
                Box(Modifier.fillMaxSize()) { content() }
            }
        }
        mainClock.advanceTimeBy(800)
        file.parentFile.mkdirs()
        file.writeBytes(requireNotNull(Image.makeFromBitmap(captureToImage().asSkiaBitmap()).encodeToData(EncodedImageFormat.PNG)).bytes)
    }
}

class ConversationSourcesArtboardTest {
    @Test
    fun theFourSourcesRenderTheirDeclaredSurface() {
        val sources = listOf(
            "headless-claude" to { sourceStore(HEADLESS_ABILITIES, HEADLESS_CONTROLS, agent = "claude") },
            "codex" to { sourceStore(CODEX_ABILITIES, agent = "codex") },
            "muffit" to { sourceStore(MUFFIT_ABILITIES, agent = "muffit") },
            "terminal-claude" to { sourceStore(TERMINAL_ABILITIES, agent = "claude") },
        )
        for ((name, make) in sources) {
            for (frame in SourceFrame.entries) {
                val store = make()
                val info = demoInfo(
                    agent = if (name == "codex") {
                        "codex"
                    } else if (name == "muffit") {
                        "muffit"
                    } else {
                        "claude"
                    },
                    status = "idle",
                    converses = true,
                )
                shoot(name, frame) { ConversationView(store.pane(PANE_ID), info, Modifier.fillMaxSize()) }
            }
        }
    }
}
