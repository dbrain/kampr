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
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.theme.themeOf
import dev.kampr.shared.ui.LocalManage
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.PaneScreenMobile
import dev.kampr.shared.ui.PaneView
import dev.kampr.shared.wire.Cursor
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.Run
import dev.kampr.shared.wire.RowDiff
import dev.kampr.shared.wire.ServerMsg
import java.awt.image.BufferedImage
import java.io.ByteArrayInputStream
import java.io.File
import javax.imageio.ImageIO
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private const val PANE = "01JNODE/w1:p1"
private val OUT = File("build/artboards")

// The words the node actually puts in `herd.panes[].detail`.
private const val DETAIL =
    "No pane on this node can show a screen: Kampr cannot run herdr — the herdr binary is " +
        "configured as /usr/bin/herdr and that is not an executable file. Put herdr on the node's " +
        "PATH, or set herdr.binary in its config to the full path; kampr doctor on that machine " +
        "says where it looked. Kampr keeps retrying, and the panes come back on their own."

private fun info(detail: String? = null) = PaneInfo(
    id = PANE,
    nodeId = "01JNODE",
    workspace = "kampr",
    cwd = "/home/dbrain/dev/kampr",
    agentStatus = "idle",
    cols = 94,
    rows = 40,
    detail = detail,
)

// A pane that has painted, so "blank" is a claim about the fault rather than about the fixture.
private fun painted(): PaneState = PaneState(PANE, StyleTable()).apply {
    applyReset(
        ServerMsg.GridReset(
            pane = PANE,
            cols = 94,
            rows = 40,
            rowsData = listOf(RowDiff(0, listOf(Run(0, "$ ")))),
            cursor = Cursor(2, 0, true),
            links = emptyList(),
        ),
    )
}

@Composable
private fun Screen(pane: PaneState, info: PaneInfo?) {
    CompositionLocalProvider(LocalSafeArea provides BARS, LocalManage provides AllowManage) {
        PaneScreenMobile(
            pane = pane,
            info = info,
            view = PaneView.Terminal,
            surfaces = BlankSurfaces,
            landscape = false,
            readOnly = false,
            onBack = {},
            onView = {},
            modifier = Modifier.fillMaxSize(),
        )
    }
}

@OptIn(ExperimentalTestApi::class)
class StreamNoticeTest {
    // The whole defect, from the operator's side: a pane that can never paint used to look exactly
    // like a pane that had not painted yet, and the only difference was in a journal on a machine
    // they were not sitting at.
    @Test
    fun anUnstreamablePaneSaysWhatIsWrongAndWhatToDo() = runComposeUiTest {
        var detail by mutableStateOf<String?>(null)
        setContent {
            CompositionLocalProvider(LocalTokens provides phoneTokens()) {
                Box(Modifier.size(360.dp, 800.dp)) { Screen(PaneState(PANE, StyleTable()), info(detail)) }
            }
        }
        waitForIdle()
        assertEquals(
            0,
            onAllNodesWithContentDescription(DETAIL, substring = true).fetchSemanticsNodes().size,
            "an empty pane is an ordinary thing and must not be dressed as a fault (probe #212)",
        )

        detail = DETAIL
        waitForIdle()
        // It arrives without the operator having done anything, so it is a live region — the same
        // convention the role notice and the watch notice already use.
        onNodeWithContentDescription(DETAIL, substring = true).assertExists()
        onNodeWithContentDescription(NO_PICTURE).assertExists()

        detail = null
        waitForIdle()
        assertEquals(
            0,
            onAllNodesWithContentDescription(DETAIL, substring = true).fetchSemanticsNodes().size,
            "the notice outlived the fault it was about",
        )
        assertEquals(0, onAllNodesWithContentDescription(NO_PICTURE).fetchSemanticsNodes().size)
    }

    // Once a pane has a grid, the grid is the truth on that surface and the mark in the header is
    // the whole of the news. Covering a last-known screen with a card is the stale badge's job
    // done worse.
    @Test
    fun aPaneThatHasAlreadyPaintedKeepsItsGridAndWearsTheMark() = runComposeUiTest {
        setContent {
            CompositionLocalProvider(LocalTokens provides phoneTokens()) {
                Box(Modifier.size(360.dp, 800.dp)) { Screen(painted(), info(DETAIL)) }
            }
        }
        waitForIdle()
        onNodeWithContentDescription(NO_PICTURE).assertExists()
        assertEquals(
            0,
            onAllNodesWithContentDescription(DETAIL, substring = true).fetchSemanticsNodes().size,
            "the full notice belongs where a grid would have been, not over one",
        )
    }

    // Ink, not pixels: an earlier attempt at this compared images and failed on 212 antialiasing
    // pixels. The claim is that the surface below the header stops being empty.
    @Test
    fun theNoticeFillsTheSpaceTheGridNeverWill() {
        val blank = ink(board("pane-no-stream-empty", null))
        val notice = ink(board("pane-no-stream", DETAIL))
        assertTrue(blank.isEmpty(), "the fixture's own surface has to start empty: $blank")
        assertTrue(notice.isNotEmpty(), "a pane that can never paint showed the operator nothing")
        assertTrue(
            notice.first() > 0.25f && notice.last() < 0.85f,
            "the notice sits in the middle of the surface, not under the chrome: $notice",
        )
    }

    private fun board(name: String, detail: String?): BufferedImage {
        val image = render(360.dp, 800.dp, themeOf("soft"), TypeScale.Phone, File(OUT, "$name.png"), density = Density(3f)) {
            Screen(PaneState(PANE, StyleTable()), info(detail))
        }
        return ImageIO.read(ByteArrayInputStream(requireNotNull(image.encodeToData()).bytes))
    }

    // Which fraction of the way down the surface holds anything but its own ground, below the
    // chrome the header measured and above the key row.
    private fun ink(image: BufferedImage): List<Float> {
        // The left margin, which the notice never reaches: sampling the middle would have taken
        // the ground *from inside the card* and called the rest of the screen ink.
        val ground = image.getRGB(2, (image.height * 0.55f).toInt())
        val from = (image.height * 0.45f).toInt()
        val to = (image.height * 0.92f).toInt()
        return (from until to)
            .filter { y -> (0 until image.width).any { image.getRGB(it, y) != ground } }
            .map { it.toFloat() / image.height }
    }
}

private const val NO_PICTURE = "This pane has no picture"
