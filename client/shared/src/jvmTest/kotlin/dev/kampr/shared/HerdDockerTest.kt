package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.SemanticsNodeInteraction
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.v2.runComposeUiTest
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.Fold
import dev.kampr.shared.model.Folds
import dev.kampr.shared.model.Herd
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.model.groups
import dev.kampr.shared.model.tree
import dev.kampr.shared.platform.MemoryPrefs
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.ui.AppState
import dev.kampr.shared.ui.HerdLandscape
import dev.kampr.shared.ui.HerdPortrait
import dev.kampr.shared.ui.HerdRail
import dev.kampr.shared.ui.HerdSidebar
import dev.kampr.shared.ui.LocalManage
import dev.kampr.shared.ui.ManageIo
import dev.kampr.shared.ui.MenuAnchor
import dev.kampr.shared.ui.RAIL_WIDTH
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Wire
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

private const val HOST = "01JHOST"
private const val CHILD = "01JHOST-c-f49c60fac115"
private const val STRAY = "01JHOST-c-0000000000aa"
private const val NOW_SEC = 1_794_220_000L
private const val NOW = NOW_SEC * 1000.0

private val HERD_JSON = """
{"t":"herd",
 "nodes":[
  {"id":"$HOST","name":"front","kind":"local","online":true,"native":true,"docker":true,
   "containers":[
    {"id":"f49c60fac115","name":"kobbler-postgres-test-1","image":"postgres:18.6","state":"running",
     "health":"healthy","started_at":${NOW_SEC - 3600},"restarts":4,"project":"kobbler","child":"$CHILD"},
    {"id":"c2","name":"kobbler-cache-1","image":"redis","state":"running","health":"unhealthy",
     "started_at":${NOW_SEC - 120},"project":"kobbler"},
    {"id":"c3","name":"worker","image":"w","state":"restarting"},
    {"id":"c4","name":"frozen","image":"f","state":"paused","started_at":${NOW_SEC - 7200}},
    {"id":"c5","name":"flaky","image":"f","state":"exited","exit_code":137,"oom":true,
     "finished_at":${NOW_SEC - 180},"last_crash":{"at":${NOW_SEC - 180},"exit_code":137,"oom":true}},
    {"id":"c6","name":"old-job","image":"o","state":"exited","exit_code":0,"finished_at":${NOW_SEC - 900_000}},
    {"id":"c7","name":"fresh","image":"o","state":"created"}
   ]},
  {"id":"$CHILD","name":"pg-inside","kind":"peer","online":false,"native":true,"reachable":true,"parent":"$HOST"},
  {"id":"$STRAY","name":"stray-inside","kind":"peer","online":false,"native":true,"reachable":true,"parent":"$HOST"}
 ],
 "panes":[
  {"id":"$HOST/n:t1","node_id":"$HOST","provider":"native","label":"build","agent_status":"idle"},
  {"id":"$HOST/n:sh","node_id":"$HOST","provider":"native","label":"pg shell","agent_status":"idle",
   "container":"kobbler-postgres-test-1"},
  {"id":"$HOST/m:wake","node_id":"$HOST","provider":"muffit","agent":"muffit","label":"wake","agent_status":"working"},
  {"id":"$CHILD/n:in","node_id":"$CHILD","provider":"native","label":"inside","agent_status":"idle"}
 ]}
"""

private val HERD: Herd = (Wire.decode(HERD_JSON) as ServerMsg.Herd).let { Herd(it.nodes, it.panes, known = true) }

private val OPEN = Folds().with(HOST, Fold.Containers, true)

private class Opened : ManageIo {
    val containers = mutableListOf<Pair<String, String>>()
    override val enabled = true
    override fun openNew(paneId: String?) = Unit
    override fun openActions(paneId: String) = Unit
    override fun openMenu(paneId: String, at: MenuAnchor?) = Unit
    override fun openContainer(nodeId: String, containerId: String) {
        containers += nodeId to containerId
    }
}

private data class Folded(val node: String, val fold: Fold, val open: Boolean)

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.count(label: String): Int = onAllNodesWithContentDescription(label, substring = true).fetchSemanticsNodes().size

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.find(label: String): SemanticsNodeInteraction = onNodeWithContentDescription(label, substring = true)

private fun SemanticsNodeInteraction.left(): Dp = getUnclippedBoundsInRoot().left

private fun SemanticsNodeInteraction.top(): Dp = getUnclippedBoundsInRoot().top

private fun SemanticsNodeInteraction.state(): String? = fetchSemanticsNode().config.getOrNull(SemanticsProperties.StateDescription)

@OptIn(ExperimentalTestApi::class)
class HerdDockerTest {
    @Composable
    private fun Portrait(folds: Folds, manage: ManageIo = Opened(), onFold: (String, Fold, Boolean) -> Unit = { _, _, _ -> }) {
        CompositionLocalProvider(LocalTokens provides phoneTokens(), LocalManage provides manage) {
            Box(Modifier.size(420.dp, 3000.dp)) {
                HerdPortrait(HERD, ConnectionStatus.Live("full"), NOW, 12.0, emptyList(), {}, null, folds = folds, onFold = onFold)
            }
        }
    }

    @Test
    fun aChildNodeIsDrawnUnderItsHostAndNeverAsAHostOfItsOwn() {
        val top = HERD.tree()
        assertEquals(listOf(HOST), top.map { it.node.id })
        assertEquals(setOf(CHILD, STRAY), top.single().children.map { it.node.id }.toSet())
        assertEquals(3, HERD.groups().size, "the flat list still holds every node, for the palette and the picker")
    }

    @Test
    fun theDefaultShowsWhatIsAliveOrRecentlyCrashedAndCountsTheRest() = runComposeUiTest {
        val folded = mutableListOf<Folded>()
        setContent { Portrait(OPEN, onFold = { n, f, o -> folded += Folded(n, f, o) }) }
        for (name in listOf("kobbler-postgres-test-1", "kobbler-cache-1", "worker", "frozen", "flaky")) {
            assertEquals(1, count("Container $name,"), name)
        }
        assertEquals(0, count("Container old-job,"))
        assertEquals(0, count("Container fresh,"))
        assertTrue(count("exited 137 · OOM 3m ago") >= 1, "a crash says how and when")
        val stopped = find("2 stopped containers")
        assertEquals("collapsed", stopped.state())
        stopped.performScrollTo().performClick()
        assertEquals(listOf(Folded(HOST, Fold.Stopped, true)), folded)
    }

    @Test
    fun aFailingAdoptionIsShownWhateverItsStateAndSaysSo() = runComposeUiTest {
        val failing = (
            Wire.decode(
                """{"t":"herd","nodes":[{"id":"$HOST","name":"front","kind":"local","online":true,"docker":true,"containers":[
                 {"id":"c9","name":"stuck","image":"s","state":"exited","exit_code":0,"finished_at":${NOW_SEC - 900_000},
                  "adopt_error":"stuck is not running"},
                 {"id":"gone","name":"gone","image":"","state":"missing","adopt_error":"gone no longer exists"}]}],"panes":[]}""",
            ) as ServerMsg.Herd
            ).let { Herd(it.nodes, it.panes, known = true) }
        setContent {
            CompositionLocalProvider(LocalTokens provides phoneTokens(), LocalManage provides Opened()) {
                Box(Modifier.size(420.dp, 3000.dp)) {
                    HerdPortrait(failing, ConnectionStatus.Live("full"), NOW, 12.0, emptyList(), {}, null, folds = OPEN, onFold = { _, _, _ -> })
                }
            }
        }
        assertEquals(1, count("Container stuck,"))
        assertEquals(1, count("Container gone,"))
        assertTrue(count("gone no longer exists") == 0 && count(", gone") >= 1, "a missing container reads as gone")
        assertEquals(2, count("adoption failing"))
    }

    @Test
    fun theStoppedOnesAppearOnceUnfolded() = runComposeUiTest {
        setContent { Portrait(OPEN.with(HOST, Fold.Stopped, true)) }
        assertEquals(1, count("Container old-job,"))
        assertEquals(1, count("Container fresh,"))
        assertEquals("expanded", find("2 stopped containers").state())
    }

    @Test
    fun aComposeProjectIsACaptionAboveItsContainersNotALevel() = runComposeUiTest {
        setContent { Portrait(OPEN) }
        val caption = find("Project kobbler")
        val pg = find("Container kobbler-postgres-test-1,")
        val worker = find("Container worker,")
        assertTrue(caption.top() < pg.top())
        assertEquals(worker.left(), pg.left(), "a project is not a nesting level")
    }

    @Test
    fun aShellAndAnAdoptedNodeSitUnderTheirContainerIndentedATightEight() = runComposeUiTest {
        setContent { Portrait(OPEN) }
        val build = find("Open build")
        val pg = find("Container kobbler-postgres-test-1,")
        val shell = find("Open pg shell")
        val inside = find("pg-inside, ")
        assertEquals(1, count("pg-inside, "), "an adopted node is drawn once")
        assertEquals(1, count("Open pg shell"), "a shell is not also among the host's terminals")
        assertTrue(shell.top() > pg.top() && inside.top() > pg.top())
        assertTrue(shell.top() > build.top())
        val indent = shell.left() - build.left()
        assertTrue(indent > 0.dp && indent <= 8.dp, "indent was $indent")
        assertEquals(1, count("Open inside"), "the adopted node's own sessions are listed")
    }

    @Test
    fun anAdoptedNodeWithNoRowStillSitsUnderItsHost() = runComposeUiTest {
        setContent { Portrait(OPEN) }
        assertEquals(1, count("stray-inside, "))
        assertTrue(find("stray-inside, ").left() > find("front, ").left())
    }

    @Test
    fun containersStartFoldedAndTheHeaderSaysSoAndUnfolds() = runComposeUiTest {
        val folded = mutableListOf<Folded>()
        setContent { Portrait(Folds(), onFold = { n, f, o -> folded += Folded(n, f, o) }) }
        assertEquals(0, count("Container worker,"))
        val header = find("Containers · docker")
        assertEquals("collapsed", header.state())
        assertEquals("expanded", find("Background · muffit").state())
        assertEquals(1, count("Open wake"))
        header.performClick()
        assertEquals(listOf(Folded(HOST, Fold.Containers, true)), folded)
    }

    @Test
    fun aMuffitFoldHidesTheBackgroundSessions() = runComposeUiTest {
        setContent { Portrait(Folds().with(HOST, Fold.Muffit, false)) }
        assertEquals(0, count("Open wake"))
        assertEquals("collapsed", find("Background · muffit").state())
    }

    @Test
    fun tappingAContainerOpensItsSheet() = runComposeUiTest {
        val manage = Opened()
        setContent { Portrait(OPEN, manage) }
        find("Container worker,").performClick()
        assertEquals(listOf(HOST to "c3"), manage.containers)
    }

    @Test
    fun theWideLayoutsDrawTheSameTree() = runComposeUiTest {
        var wide by mutableStateOf(true)
        setContent {
            CompositionLocalProvider(LocalTokens provides phoneTokens(), LocalManage provides Opened()) {
                Box(Modifier.size(1200.dp, 3000.dp)) {
                    if (wide) {
                        HerdLandscape(HERD, ConnectionStatus.Live("full"), NOW, 12.0, emptyList(), {}, null, folds = OPEN)
                    } else {
                        HerdSidebar(HERD, ConnectionStatus.Live("full"), NOW, 12.0, emptyList(), null, "d", "full", {}, {}, folds = OPEN)
                    }
                }
            }
        }
        for (pass in 0..1) {
            assertEquals(1, count("pg-inside, "), "pass $pass")
            assertEquals(1, count("Container flaky,"), "pass $pass")
            assertEquals(1, count("Open pg shell"), "pass $pass")
            wide = false
            waitForIdle()
        }
    }

    @Test
    fun theRailFollowsTheSameFolds() = runComposeUiTest {
        var folds by mutableStateOf(Folds())
        setContent {
            CompositionLocalProvider(LocalTokens provides phoneTokens(), LocalManage provides Opened()) {
                Box(Modifier.size(RAIL_WIDTH, 3000.dp)) {
                    HerdRail(HERD, ConnectionStatus.Live("full"), NOW, null, "d", "full", {}, {}, {}, folds = folds)
                }
            }
        }
        assertEquals(1, count("Open wake"))
        assertEquals(0, count("Container worker,"))
        folds = Folds().with(HOST, Fold.Muffit, false).with(HOST, Fold.Containers, true)
        waitForIdle()
        assertEquals(0, count("Open wake"))
        assertEquals(1, count("Container worker,"))
        assertEquals(1, count("Open pg shell"))
        assertEquals(1, count("Open inside"))
    }

    @Test
    fun foldsAreRememberedOnThisDeviceAcrossLaunches() {
        val prefs = MemoryPrefs()
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Unconfined)
        try {
            val first = AppState(scope, KamprStore(), prefs, null)
            assertFalse(first.folds.open(HOST, Fold.Containers))
            assertTrue(first.folds.open(HOST, Fold.Muffit))
            assertFalse(first.folds.open(HOST, Fold.Stopped))
            first.fold(HOST, Fold.Containers, true)
            first.fold(HOST, Fold.Muffit, false)
            first.fold(CHILD, Fold.Stopped, true)
            assertTrue(first.folds.open(HOST, Fold.Containers))

            val again = AppState(scope, KamprStore(), prefs, null)
            assertTrue(again.folds.open(HOST, Fold.Containers))
            assertFalse(again.folds.open(HOST, Fold.Muffit))
            assertTrue(again.folds.open(CHILD, Fold.Stopped))
            assertFalse(again.folds.open(CHILD, Fold.Containers))
        } finally {
            scope.cancel()
        }
    }
}
