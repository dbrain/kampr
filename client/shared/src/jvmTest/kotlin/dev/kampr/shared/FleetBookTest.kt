package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.Herd
import dev.kampr.shared.theme.Ground
import dev.kampr.shared.theme.KamprFonts
import dev.kampr.shared.theme.KamprTokens
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.theme.on
import dev.kampr.shared.theme.themeOf
import dev.kampr.shared.theme.typography
import dev.kampr.shared.ui.Breakpoint
import dev.kampr.shared.ui.FleetScreen
import dev.kampr.shared.ui.LocalConnectionStatus
import dev.kampr.shared.wire.FleetCommand
import dev.kampr.shared.wire.ManageOp
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.ServerMsg
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull
import kotlin.test.assertTrue

private val HERD = Herd(
    nodes = listOf(
        NodeInfo(id = "n1", name = "one", kind = "primary", online = true),
        NodeInfo(id = "n2", name = "two", kind = "peer", online = true),
    ),
    known = true,
)

private val SAVED = FleetCommand(id = "b1", args = listOf("kampr", "update"), label = "update everything")
private val RECENT = FleetCommand(id = "b2", args = listOf("pacman", "-Syu"))
private val LEAKY = FleetCommand(id = "b3", args = listOf("env", "TOKEN=hunter2", "./deploy"))

// How a node writes down a command the operator typed: one element, and that element is the whole
// line. An older client renders an entry by joining `args` with spaces, so this reads back byte
// for byte — quotes, pipe and all — on a client that has never heard of the change.
private val TYPED = FleetCommand(id = "b4", args = listOf("""find . -name "*.rs" | wc -l"""))

private fun tokens() = themeOf("soft").on(Ground.Dark).let { spec ->
    val fonts = KamprFonts(FontFamily.Default, FontFamily.Monospace, FontFamily.Monospace)
    KamprTokens(spec, fonts, typography(fonts, spec.label, TypeScale.Phone))
}

private class Pressed {
    var ran: String? = null
    var op: ManageOp? = null
}

@OptIn(ExperimentalTestApi::class)
class FleetBookTest {
    // **The guard question.** A saved command is a one-press fan-out across every machine in the
    // herd, and pressing one must not become a way to skip the confirmation a typed command gets.
    // It stages; the sheet's own button is still the only thing that fires.
    @Test
    fun pressingASavedCommandStagesItAndRunsNothing() = runComposeUiTest {
        val board = open(book(saved = listOf(SAVED)))
        onNodeWithContentDescription("Put kampr update in the box").performClick()
        assertNull(board.ran, "a saved command fanned out across the herd on one press")

        onNodeWithContentDescription("Run everywhere").performClick()
        assertEquals("kampr update", board.ran)
    }

    // **The line reaches the run unsplit, and the sheet shows it before it fans out.** What used to
    // be here was a note telling the operator that `;` and `&&` were arguments and to write
    // `sh -c '…'` if they meant a pipeline. The note is gone because the limitation is; what
    // replaced it is the command itself, on screen, beside the number of machines.
    @Test
    fun aTypedPipelineIsPreviewedWholeAndFansOutExactlyAsItWasWritten() = runComposeUiTest {
        val board = open(book(recent = listOf(TYPED)))
        onNodeWithContentDescription("""Put find . -name "*.rs" | wc -l in the box""").performClick()
        assertTrue(
            onAllNodesWithText("Will run on 2 machines").fetchSemanticsNodes().isNotEmpty(),
            "the sheet did not say what it was about to run, or on how many machines",
        )
        assertTrue(
            onAllNodesWithText("""find . -name "*.rs" | wc -l""", substring = true)
                .fetchSemanticsNodes()
                .isNotEmpty(),
            "the line about to reach every machine in the herd was not on screen",
        )
        onNodeWithContentDescription("Run everywhere").performClick()
        assertEquals("""find . -name "*.rs" | wc -l""", board.ran)
    }

    // A label that hid what was about to run on every machine would be a trap, so it never
    // replaces the command — it sits above it.
    @Test
    fun aLabelledCommandStillShowsTheCommandItWillRun() = runComposeUiTest {
        open(book(saved = listOf(SAVED)))
        assertTrue(
            onAllNodesWithText("kampr update").fetchSemanticsNodes().isNotEmpty(),
            "a label hid the command it was about to run on every machine in the herd",
        )
        assertTrue(onAllNodesWithText("update everything").fetchSemanticsNodes().isNotEmpty())
    }

    // The one thing that actually holds about credentials in the book: whatever got written down
    // can be taken out again.
    @Test
    fun anythingInTheBookCanBeForgotten() = runComposeUiTest {
        val board = open(book(recent = listOf(LEAKY)))
        onNodeWithContentDescription("Forget env TOKEN=hunter2 ./deploy").performClick()
        assertEquals(ManageOp.FleetDrop("b3"), board.op)
    }

    @Test
    fun aRecentCommandCanBeKeptAndKeepingItSendsTheEntryRatherThanTheArgv() = runComposeUiTest {
        val board = open(book(recent = listOf(RECENT)))
        onNodeWithContentDescription("Keep pacman -Syu").performClick()
        assertEquals(ManageOp.FleetSave(entry = "b2"), board.op)
    }

    // Staged from the book rather than typed, because the node applies the same rule to decide
    // what it writes down by itself — and the operator has to be told which of the two is
    // happening before they press Save.
    @Test
    fun aCommandThatLooksLikeItCarriesASecretSaysSoBeforeItIsSaved() = runComposeUiTest {
        open(book(recent = listOf(LEAKY)))
        onNodeWithContentDescription("Put env TOKEN=hunter2 ./deploy in the box").performClick()
        assertTrue(
            onAllNodesWithText("carries TOKEN", substring = true).fetchSemanticsNodes().isNotEmpty(),
            "the sheet offered to write a credential to the node's disk and said nothing",
        )
    }

    // A fresh node has nothing, and the panel has to read as deliberate rather than broken.
    @Test
    fun anEmptyBookSaysWhatWouldFillItRatherThanShowingNothing() = runComposeUiTest {
        open(book())
        assertTrue(onAllNodesWithText("Saved").fetchSemanticsNodes().isNotEmpty())
        assertTrue(
            onAllNodesWithText("Nothing kept yet", substring = true).fetchSemanticsNodes().isNotEmpty(),
        )
        assertTrue(
            onAllNodesWithText("What you run here shows up", substring = true).fetchSemanticsNodes().isNotEmpty(),
        )
    }

    private fun book(
        recent: List<FleetCommand> = emptyList(),
        saved: List<FleetCommand> = emptyList(),
    ) = ServerMsg.FleetBook(recent = recent, saved = saved)

    // The sheet is behind **Run**, which is where the operator goes to run something — so the
    // memory lives beside the box rather than on a screen of its own.
    private fun ComposeUiTest.open(book: ServerMsg.FleetBook): Pressed {
        val board = Pressed()
        setContent {
            CompositionLocalProvider(
                LocalTokens provides tokens(),
                LocalConnectionStatus provides ConnectionStatus.Live("full"),
            ) {
                Box(Modifier.size(411.dp, 891.dp)) {
                    FleetScreen(
                        herd = HERD,
                        breakpoint = Breakpoint.Portrait,
                        onOpenPane = {},
                        onAnswer = { _, _ -> },
                        onStop = {},
                        onRun = { board.ran = it },
                        canRun = true,
                        book = book,
                        onBook = { board.op = it },
                        modifier = Modifier.fillMaxSize(),
                    )
                }
            }
        }
        waitForIdle()
        onNodeWithContentDescription("Run").performClick()
        waitForIdle()
        return board
    }
}
