package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Ground
import dev.kampr.shared.theme.KamprFonts
import dev.kampr.shared.theme.KamprTokens
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.theme.on
import dev.kampr.shared.theme.themeOf
import dev.kampr.shared.theme.typography
import dev.kampr.shared.ui.AgentArgs
import dev.kampr.shared.ui.Breakpoint
import dev.kampr.shared.ui.NewSheet
import dev.kampr.shared.wire.ManageOp
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.ServerMsg
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

private val NODE = NodeInfo(id = "01JNODE", name = "comingclean", kind = "local")

private val PANE = PaneInfo(
    id = "01JNODE/w3:p2",
    nodeId = "01JNODE",
    workspaceId = "01JNODE/w3",
    workspace = "kampr",
    cwd = "/home/dbrain/dev/kampr",
    cols = 74,
    rows = 30,
)

private val CAPS = ServerMsg.NodeCaps(node = "01JNODE", agentKinds = listOf("claude", "codex"), sessions = emptyList())

private class FakeArgs(initial: Map<String, String> = emptyMap()) : AgentArgs {
    val stored = initial.toMutableMap()
    override fun get(kind: String): String = stored[kind].orEmpty()
    override fun remember(kind: String, text: String?) {
        if (text == null) stored.remove(kind) else stored[kind] = text
    }
}

private fun tokens() = themeOf("soft").on(Ground.Dark).let { spec ->
    val fonts = KamprFonts(FontFamily.Default, FontFamily.Monospace, FontFamily.Monospace)
    KamprTokens(spec, fonts, typography(fonts, spec.label, TypeScale.Phone))
}

@Composable
private fun Themed(content: @Composable () -> Unit) {
    CompositionLocalProvider(LocalTokens provides tokens(), content = content)
}

// The habit this exists for is a shell alias — `dangerclaude`, which is `claude
// --dangerously-skip-permissions`. An alias only exists inside an interactive shell, so it is not
// something `agent.start` can invoke; the argv is. The node has always forwarded it and nothing
// here ever set it.
@OptIn(ExperimentalTestApi::class)
class AgentArgsTest {
    @Test
    fun theFlagsTypedForAnAgentReachTheOp() = runComposeUiTest {
        val sent = mutableListOf<ManageOp>()
        val args = FakeArgs()
        setContent { Themed { Box(Modifier.size(420.dp, 900.dp)) { sheet(sent, args) } } }

        onNodeWithContentDescription("Start a claude agent").performClick()
        onNodeWithContentDescription("Arguments for claude")
            .performTextInput("--dangerously-skip-permissions")
        // Not hidden behind a remembered setting: the sheet prints the launch it is about to make.
        onNodeWithContentDescription("Starts claude --dangerously-skip-permissions").assertExists()
        onNodeWithContentDescription("Start claude").performClick()
        waitForIdle()

        assertEquals(
            listOf(
                ManageOp.AgentStart(
                    PANE.id,
                    "claude",
                    null,
                    listOf("--dangerously-skip-permissions"),
                ) as ManageOp,
            ),
            sent,
        )
        assertEquals("--dangerously-skip-permissions", args.stored["claude"])
    }

    // Somebody who wants that flag wants it every time. Coming back prefilled is the feature; the
    // printed launch line is what stops it from being a setting they forget is on.
    @Test
    fun rememberedFlagsComeBackAndAreVisibleBeforeAnythingIsTyped() = runComposeUiTest {
        val sent = mutableListOf<ManageOp>()
        val args = FakeArgs(mapOf("claude" to "--dangerously-skip-permissions"))
        setContent { Themed { Box(Modifier.size(420.dp, 900.dp)) { sheet(sent, args) } } }

        onNodeWithContentDescription("Start a claude agent").performClick()
        onNodeWithContentDescription("Starts claude --dangerously-skip-permissions").assertExists()
        onNodeWithContentDescription(
            "--dangerously-skip-permissions removes a confirmation step — this agent will act without asking",
        ).assertExists()
        onNodeWithContentDescription("Start claude").performClick()
        waitForIdle()

        assertEquals(listOf("--dangerously-skip-permissions"), (sent.single() as ManageOp.AgentStart).args)
    }

    // Per kind, not per sheet: picking the other harness must not launch it with the first one's
    // flags, which is exactly the mistake a single remembered box would make.
    @Test
    fun eachHarnessKeepsItsOwnFlags() = runComposeUiTest {
        val sent = mutableListOf<ManageOp>()
        val args = FakeArgs(mapOf("claude" to "--dangerously-skip-permissions"))
        setContent { Themed { Box(Modifier.size(420.dp, 900.dp)) { sheet(sent, args) } } }

        onNodeWithContentDescription("Start a codex agent").performClick()
        onNodeWithContentDescription("Starts codex").assertExists()
        onNodeWithContentDescription("Start codex").performClick()
        waitForIdle()

        assertEquals(emptyList(), (sent.single() as ManageOp.AgentStart).args)
    }

    @Test
    fun turningTheMemoryOffForgetsTheFlagRatherThanKeepingIt() = runComposeUiTest {
        val sent = mutableListOf<ManageOp>()
        val args = FakeArgs(mapOf("claude" to "--dangerously-skip-permissions"))
        setContent { Themed { Box(Modifier.size(420.dp, 900.dp)) { sheet(sent, args) } } }

        onNodeWithContentDescription("Start a claude agent").performClick()
        onNodeWithContentDescription("Remember these arguments for claude").performClick()
        onNodeWithContentDescription("Start claude").performClick()
        waitForIdle()

        assertNull(args.stored["claude"], "the flag was still remembered after being turned off")
        assertEquals(
            listOf("--dangerously-skip-permissions"),
            (sent.single() as ManageOp.AgentStart).args,
            "this launch still gets what was in the box",
        )
    }
}

@Composable
private fun sheet(sent: MutableList<ManageOp>, args: AgentArgs) {
    NewSheet(
        breakpoint = Breakpoint.Portrait,
        node = NODE,
        pane = PANE,
        nodes = listOf(NODE),
        caps = CAPS,
        outcome = null,
        onManage = { sent += it },
        onNode = {},
        onNodePicker = {},
        onDismiss = {},
        agentArgs = args,
    )
}
