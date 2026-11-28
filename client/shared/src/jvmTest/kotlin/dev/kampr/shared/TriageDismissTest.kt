package dev.kampr.shared

import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.v2.runComposeUiTest
import androidx.compose.ui.text.font.FontFamily
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.platform.MemoryPrefs
import dev.kampr.shared.theme.Ground
import dev.kampr.shared.theme.KamprFonts
import dev.kampr.shared.theme.KamprTokens
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.theme.on
import dev.kampr.shared.theme.themeOf
import dev.kampr.shared.theme.typography
import dev.kampr.shared.ui.AppState
import dev.kampr.shared.ui.HerdLandscape
import dev.kampr.shared.ui.HerdPortrait
import dev.kampr.shared.ui.HerdSidebar
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.ServerMsg
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private const val PANE = "n1/w1:p1"

private fun herd(status: String) = ServerMsg.Herd(
    listOf(NodeInfo("n1", "box", kind = "local", online = true)),
    listOf(PaneInfo(id = PANE, nodeId = "n1", workspace = "plan", agent = "claude", agentStatus = status, cols = 80, rows = 24)),
)

private fun question(id: String?, text: String = "Approve this plan?") = ServerMsg.Pending(PANE, text, emptyList(), "hook", id = id, kind = "plan")

private fun withApp(prefs: MemoryPrefs = MemoryPrefs(), body: (AppState, KamprStore) -> Unit) {
    val scope = CoroutineScope(SupervisorJob() + Dispatchers.Unconfined)
    try {
        val store = KamprStore()
        body(AppState(scope, store, prefs, null), store)
    } finally {
        scope.cancel()
    }
}

// A plan the operator means to read slowly is still a blocked agent, so the pane keeps its blocked
// mark; what goes is the "Needs you" card, and only until the agent asks something else.
@OptIn(ExperimentalTestApi::class)
class TriageDismissTest {
    @Test
    fun aDismissedQuestionLeavesTheTriageButNotTheBlockedPane() = withApp { app, store ->
        store.accept(herd("blocked"))
        store.accept(question("plan_1"))
        app.dismiss(app.triage().single())
        assertEquals(emptyList(), app.triage())
        assertEquals(listOf(PANE), store.blocked().map { it.id }, "the pane itself stopped reading as blocked")
    }

    @Test
    fun aDismissalOutlivesTheAppAndAReconnect() {
        val prefs = MemoryPrefs()
        withApp(prefs) { app, store ->
            store.accept(herd("blocked"))
            store.accept(question("plan_1"))
            app.dismiss(app.triage().single())
        }
        withApp(prefs) { app, store ->
            store.accept(herd("blocked"))
            assertEquals(emptyList(), app.triage(), "came back before the node republished the question")
            store.accept(question("plan_1"))
            assertEquals(emptyList(), app.triage(), "came back on a reload")
            store.markStale()
            store.accept(herd("blocked"))
            assertEquals(emptyList(), app.triage(), "came back on a reconnect")
        }
    }

    @Test
    fun itComesBackForADifferentQuestionOrTheNextBlock() {
        val cases = listOf<Pair<String, (KamprStore) -> Unit>>(
            "another question id" to { it.accept(question("plan_2")) },
            "another question without an id" to { it.accept(question(null, "Allow Bash?")) },
            "the next time it blocks" to {
                it.accept(herd("working"))
                it.accept(herd("blocked"))
            },
        )
        for ((case, next) in cases) {
            withApp { app, store ->
                store.accept(herd("blocked"))
                store.accept(question("plan_1"))
                app.dismiss(app.triage().single())
                next(store)
                assertEquals(1, app.triage().size, "stayed dismissed after $case")
            }
        }
    }

    @Test
    fun everyTriageListOffersTheDismissal() {
        val layouts = listOf("portrait", "landscape", "sidebar")
        for (layout in layouts) {
            runComposeUiTest {
                withApp { app, store ->
                    store.accept(herd("blocked"))
                    store.accept(question("plan_1"))
                    setContent {
                        CompositionLocalProvider(LocalTokens provides tokens()) {
                            val herd = store.herd.value
                            val live = ConnectionStatus.Live("full")
                            when (layout) {
                                "portrait" -> HerdPortrait(herd, live, 0.0, null, app.triage(), {}, null, onDismiss = app::dismiss)
                                "landscape" -> HerdLandscape(herd, live, 0.0, null, app.triage(), {}, null, onDismiss = app::dismiss)
                                else -> HerdSidebar(herd, live, 0.0, null, app.triage(), null, "d", "d", {}, {}, onDismiss = app::dismiss)
                            }
                        }
                    }
                    onNodeWithContentDescription("Dismiss", substring = true).performClick()
                    waitForIdle()
                    assertTrue(
                        onAllNodesWithContentDescription("Needs you", substring = true).fetchSemanticsNodes().isEmpty(),
                        "$layout still shouts after a dismissal",
                    )
                    assertTrue(
                        onAllNodesWithContentDescription("Blocked", substring = true).fetchSemanticsNodes().isNotEmpty(),
                        "$layout lost the pane's blocked mark",
                    )
                }
            }
        }
    }
}

private fun tokens() = themeOf("soft").on(Ground.Dark).let { spec ->
    val fonts = KamprFonts(FontFamily.Default, FontFamily.Monospace, FontFamily.Monospace)
    KamprTokens(spec, fonts, typography(fonts, spec.label, TypeScale.Phone))
}
