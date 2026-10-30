package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.platform.MemoryPrefs
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.ui.AppManage
import dev.kampr.shared.ui.AppScaffold
import dev.kampr.shared.ui.AppState
import dev.kampr.shared.ui.AuthSurface
import dev.kampr.shared.ui.Breakpoint
import dev.kampr.shared.ui.FilesPlace
import dev.kampr.shared.ui.LocalManage
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.NoMosaic
import dev.kampr.shared.ui.Sheet
import dev.kampr.shared.wire.Wire
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull
import kotlin.test.assertTrue

private const val WITH_CWD = "01JNODE/w3:p2"
private const val NO_CWD = "01JNODE/w3:p3"

private fun hello(role: String) = """{"t":"hello","protocol":1,"node_id":"01JNODE","node_name":"comingclean","build":"test",""" +
    """"role":"$role","caps":{"manage":true},"security":{"tier":0,"passkeys":false}}"""

private const val HERD_FRAME =
    """{"t":"herd","nodes":[{"id":"01JNODE","name":"comingclean","kind":"local"}],"panes":[""" +
        """{"id":"$WITH_CWD","node_id":"01JNODE","workspace":"kampr","cwd":"/home/u/dev/kampr"},""" +
        """{"id":"$NO_CWD","node_id":"01JNODE","workspace":"kampr"}]}"""

// The pane actions sheet is the one way in from a pane, so it is pressed through the app's own
// scaffold: a chip whose callback is wired nowhere would pass a test of the sheet alone.
@OptIn(ExperimentalTestApi::class)
class FilesEntryTest {
    private fun withApp(role: String, block: ComposeUiTest.(AppState) -> Unit) {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Unconfined)
        val store = KamprStore()
        store.accept(Wire.decode(hello(role)) ?: error("undecodable hello"))
        store.accept(Wire.decode(HERD_FRAME) ?: error("undecodable herd"))
        val prefs = MemoryPrefs()
        prefs.set("endpoint", "http://127.0.0.1:8790")
        prefs.set("token", "tok")
        val app = AppState(scope, store, prefs, null)
        try {
            runComposeUiTest {
                setContent {
                    CompositionLocalProvider(LocalTokens provides phoneTokens(), LocalSafeArea provides BARS, LocalManage provides AppManage(app)) {
                        Box(Modifier.size(411.dp, 914.dp)) {
                            AppScaffold(app, Breakpoint.Portrait, BlankSurfaces, NoMosaic, 0.0, AUTH, ConnectionStatus.Live(role), null)
                        }
                    }
                }
                block(app)
            }
        } finally {
            scope.cancel()
        }
    }

    @Test
    fun theFilesChipOpensThePanesWorkingDirectoryAnchoredAtThePane() {
        val table = listOf(WITH_CWD to "/home/u/dev/kampr", NO_CWD to "~")
        for ((pane, path) in table) {
            withApp("full") { app ->
                app.openSheet(Sheet.Actions(pane))
                waitForIdle()
                onNodeWithText("files").performClick()
                waitForIdle()
                assertEquals(listOf<FilesPlace>(FilesPlace.Dir("01JNODE", pane, path)), app.files, pane)
                assertNull(app.sheet, "the sheet stays over the folder it opened")
            }
        }
    }

    @Test
    fun aReadOnlyDeviceIsNotOfferedFiles() {
        withApp("readonly") { app ->
            app.openSheet(Sheet.Actions(WITH_CWD))
            waitForIdle()
            assertTrue(onAllNodesWithText("files").fetchSemanticsNodes().isEmpty())
            assertTrue(onAllNodesWithContentDescription("Browse files on comingclean").fetchSemanticsNodes().isEmpty())
        }
    }

    @Test
    fun theFolderBesideAMachinesPlusBrowsesItsHome() {
        withApp("full") { app ->
            onNodeWithContentDescription("Browse files on comingclean").performClick()
            waitForIdle()
            assertEquals(listOf<FilesPlace>(FilesPlace.Dir("01JNODE", "01JNODE/_", "~")), app.files)
        }
    }

    // How a machine is reached and how fast it answers are glyphs beside a number, not words; the
    // words are still what a screen reader says.
    @Test
    fun theMachineHeaderSaysReachAndLatencyWithGlyphsRatherThanWords() {
        withApp("full") { _ ->
            val shown = onAllNodes(SemanticsMatcher.keyIsDefined(SemanticsProperties.Text)).fetchSemanticsNodes()
                .flatMap { it.config[SemanticsProperties.Text] }
                .map { it.text }
            assertTrue(shown.none { it.startsWith("local") || it.startsWith("peer") || "ping" in it }, "$shown")
            assertTrue(onAllNodesWithContentDescription("comingclean, local, ping", substring = true).fetchSemanticsNodes().isNotEmpty())
        }
    }
}

private val AUTH = AuthSurface(
    setup = null,
    devices = emptyList(),
    currentDeviceId = null,
    pairingCode = null,
    failure = null,
    onPairingCode = {},
    onRevoke = {},
    onRenew = {},
    onDismissFailure = {},
)
