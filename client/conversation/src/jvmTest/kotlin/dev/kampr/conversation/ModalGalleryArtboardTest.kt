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
import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.PendingOption
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Turn
import dev.kampr.shared.wire.Wire
import org.jetbrains.skia.EncodedImageFormat
import org.jetbrains.skia.Image
import java.io.File
import kotlin.test.Test

private val GALLERY_OUT = File("build/artboards/gallery")

private enum class Frame(val width: Int, val height: Int, val density: Float, val scale: TypeScale, val bars: SafeArea) {
    Portrait(390, 844, 2.625f, TypeScale.Phone, SafeArea(top = 44.dp, bottom = 46.dp)),
    Desktop(1280, 800, 1.5f, TypeScale.Desk, SafeArea(0.dp, 0.dp)),
}

private val PERMISSION = ServerMsg.Pending(
    pane = PANE_ID,
    id = "q1",
    header = "Bash",
    question = "Run the workspace tests",
    source = "hook",
    context = "cargo test --workspace --all-targets",
    options = listOf(PendingOption("1", "Yes"), PendingOption("2", "Yes, and don't ask again for cargo test"), PendingOption("3", "No")),
)

private val QUESTION = ServerMsg.Pending(
    pane = PANE_ID,
    id = "q2",
    header = "Indentation",
    question = "Which indentation do you prefer?",
    source = "hook",
    options = listOf(
        PendingOption("1", "Tabs", "Indent with tab characters, one per level, as the formatter writes them."),
        PendingOption("2", "Two spaces", "Indent with two spaces per level, which keeps deep nesting on a phone."),
        PendingOption("3", "Four spaces", "Indent with four spaces per level, as most of the codebase already does."),
        PendingOption("4", "Type something", text = true),
    ),
)

private val MULTI = ServerMsg.Pending(
    pane = PANE_ID,
    id = "q3",
    header = "Test suites",
    question = "Which test suites should I run?",
    source = "hook",
    multi = true,
    options = listOf(
        PendingOption("1", "unit", chosen = true),
        PendingOption("2", "integration"),
        PendingOption("3", "live herdr"),
        PendingOption("4", "Type something", text = true),
    ),
)

private val PLAN = ServerMsg.Pending(
    pane = PANE_ID,
    id = "q4",
    header = "Plan",
    question = "Approve Claude's plan?",
    source = "hook",
    context = "## Hooks socket\n\n1. Serve a private socket beside the node's own.\n2. Route each event by process to the pane whose tree holds it.\n3. Release a held hook when the desk answers.\n\n**Tests**: `live.rs` drives a real herdr.",
    options = listOf(
        PendingOption("1", "Yes, auto-accept edits"),
        PendingOption("2", "Yes, manually approve edits"),
        PendingOption("3", "No, tell Claude what to change", text = true),
    ),
)

private val FORM = ServerMsg.Pending(
    pane = PANE_ID,
    id = "q5",
    header = "Deploy",
    question = "Which environment should the deploy go to?",
    source = "hook",
    context = "staging-eu",
    options = emptyList(),
)

private val SCREEN = ServerMsg.Pending(
    pane = PANE_ID,
    question = "Do you want to make this edit to server.ts?",
    options = listOf(PendingOption("1", "Yes"), PendingOption("2", "Yes, allow all edits this session"), PendingOption("3", "No")),
    source = "screen",
)

private const val HOOKS = """{"t":"claude.hooks","pane":"$PANE_ID","state":"absent","ask":true,"project":"/home/dbrain/dev/kampr"}"""
private const val HOOKS_STALE = """{"t":"claude.hooks","pane":"$PANE_ID","state":"stale","ask":true,"scope":"user","project":"/home/dbrain/dev/kampr"}"""

private fun storeWith(vararg msgs: ServerMsg): KamprStore = KamprStore().also { store ->
    store.accept(
        ServerMsg.Convo(
            PANE_ID,
            "h-2",
            false,
            listOf(
                Turn("h-1", "user", null, listOf(Block.Md("wire the hooks socket and run the tests"))),
                Turn("h-2", "assistant", null, listOf(Block.Md("I've added the socket. Next I want to run the whole workspace's tests to check nothing else moved."))),
            ),
        ),
    )
    msgs.forEach(store::accept)
}

@OptIn(ExperimentalTestApi::class)
private fun shoot(name: String, frame: Frame, ground: Ground = Ground.Dark, content: @Composable () -> Unit) {
    val file = File(GALLERY_OUT, "$name-${frame.name.lowercase()}${if (ground == Ground.Light) "-light" else ""}.png")
    runSkikoComposeUiTest(Size(frame.width * frame.density, frame.height * frame.density), Density(frame.density)) {
        mainClock.autoAdvance = false
        setContent {
            CompositionLocalProvider(
                LocalTokens provides tokensFor(SoftTheme, frame.scale, ground),
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

class ModalGalleryArtboardTest {
    @Test
    fun theConversationsDialogLikeCardsRender() {
        val cards = listOf(
            "convo-permission" to { storeWith(PERMISSION) },
            "convo-question" to { storeWith(QUESTION) },
            "convo-question-multi" to { storeWith(MULTI) },
            "convo-plan" to { storeWith(PLAN) },
            "convo-terminal-only" to { storeWith(FORM) },
            "convo-screen-question" to { storeWith(SCREEN) },
            "convo-hooks-offer" to { storeWith(requireNotNull(Wire.decode(HOOKS))) },
            "convo-hooks-stale" to { storeWith(requireNotNull(Wire.decode(HOOKS_STALE))) },
        )
        for (frame in Frame.entries) {
            for ((name, make) in cards) {
                val store = make()
                val status = if (name.startsWith("convo-hooks")) "idle" else "blocked"
                shoot(name, frame) { ConversationView(store.pane(PANE_ID), demoInfo(status = status), Modifier.fillMaxSize()) }
            }
        }
        val store = storeWith(PERMISSION)
        shoot("convo-permission", Frame.Portrait, Ground.Light) { ConversationView(store.pane(PANE_ID), demoInfo(), Modifier.fillMaxSize()) }
    }
}
