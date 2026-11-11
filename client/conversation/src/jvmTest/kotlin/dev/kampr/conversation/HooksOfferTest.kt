package dev.kampr.conversation

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.v2.runComposeUiTest
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.LocalConnectionStatus
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.shared.wire.PendingOption
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Wire
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private const val ALL = "Install for all projects"
private const val PROJECT = "Install for kampr"
private const val UPDATE = "Update hooks"
private const val NOT_NOW = "Not now"
private const val NEVER = "Don't ask again"

private class HooksIo(override val readOnly: Boolean = false) : PaneIo {
    val sent = mutableListOf<ClientMsg>()
    override fun send(msg: ClientMsg) {
        sent += msg
    }
    override fun prefs(paneId: String): PanePrefs = PanePrefs()

    val manage get() = sent.filterIsInstance<ClientMsg.Manage>()
}

private class Hooked(frame: String?, readOnly: Boolean = false) {
    val store = KamprStore()
    val pane = store.pane(PANE_ID)
    val io = HooksIo(readOnly)
    val showing = mutableStateOf(true)

    init {
        frame?.let { store.accept(requireNotNull(Wire.decode(it))) }
    }
}

private fun hooks(state: String, ask: Boolean = true, scope: String? = null, project: String? = "/home/dbrain/dev/kampr"): String = buildString {
    append("""{"t":"claude.hooks","pane":"$PANE_ID","state":"$state","ask":$ask""")
    scope?.let { append(""","scope":"$it"""") }
    project?.let { append(""","project":"$it"""") }
    append("}")
}

@Composable
private fun Screen(hooked: Hooked) {
    CompositionLocalProvider(
        LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
        LocalConnectionStatus provides ConnectionStatus.Live("full"),
        LocalPaneIo provides hooked.io,
    ) {
        Box(Modifier.size(PORTRAIT.first, PORTRAIT.second)) {
            if (hooked.showing.value) ConversationView(hooked.pane, demoInfo(status = "idle"), Modifier.fillMaxSize())
        }
    }
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.shown(label: String): Boolean = onAllNodes(androidx.compose.ui.test.hasContentDescription(label)).fetchSemanticsNodes().isNotEmpty()

// docs/12 W11, "Asking to install".
@OptIn(ExperimentalTestApi::class)
class HooksOfferTest {
    @Test
    fun absentHooksOfferBothScopesAndSayWhatTheyGive() = runComposeUiTest {
        val hooked = Hooked(hooks("absent"))
        setContent { Screen(hooked) }
        waitForIdle()
        for (label in listOf(ALL, PROJECT, NOT_NOW, NEVER)) assertTrue(shown(label), "$label is not offered")
        assertTrue(!shown(UPDATE))
        onNodeWithText("arrive live", substring = true).assertExists()
        onNodeWithText("every Claude on this machine", substring = true).assertExists()
    }

    @Test
    fun withNoProjectOnlyTheUserScopeIsOffered() = runComposeUiTest {
        val hooked = Hooked(hooks("absent", project = null))
        setContent { Screen(hooked) }
        waitForIdle()
        assertTrue(shown(ALL))
        assertTrue(!shown(PROJECT))
    }

    @Test
    fun itIsNotOfferedWhereThereIsNothingToAskOrNobodyMayAnswer() {
        val cases = listOf(
            "current" to Hooked(hooks("current", scope = "user")),
            "declined" to Hooked(hooks("absent", ask = false)),
            "read-only" to Hooked(hooks("absent"), readOnly = true),
            "an older node" to Hooked(null),
        )
        for ((name, hooked) in cases) {
            runComposeUiTest {
                setContent { Screen(hooked) }
                waitForIdle()
                assertTrue(!shown(NOT_NOW) && !shown(ALL) && !shown(UPDATE), "$name: offered")
            }
        }
    }

    @Test
    fun eachPressSendsExactlyItsOp() {
        val cases = listOf(
            Triple(hooks("absent"), ALL, """{"t":"manage","op":"claude.hooks.install","at":"$PANE_ID","mode":"user"}"""),
            Triple(hooks("absent"), PROJECT, """{"t":"manage","op":"claude.hooks.install","at":"$PANE_ID","mode":"project"}"""),
            Triple(hooks("absent"), NEVER, """{"t":"manage","op":"claude.hooks.decline","at":"$PANE_ID"}"""),
            Triple(hooks("stale", scope = "project"), UPDATE, """{"t":"manage","op":"claude.hooks.install","at":"$PANE_ID","mode":"project"}"""),
            Triple(hooks("stale", scope = "user"), UPDATE, """{"t":"manage","op":"claude.hooks.install","at":"$PANE_ID","mode":"user"}"""),
            Triple(hooks("stale", scope = "user"), NEVER, """{"t":"manage","op":"claude.hooks.decline","at":"$PANE_ID"}"""),
        )
        for ((frame, press, want) in cases) {
            runComposeUiTest {
                val hooked = Hooked(frame)
                setContent { Screen(hooked) }
                waitForIdle()
                onNodeWithContentDescription(press).performClick()
                waitForIdle()
                val sent = hooked.io.manage.single()
                val got = Wire.encode(ClientMsg.Manage(sent.request))
                assertEquals(want, got, "$press on $frame")
                assertTrue(sent.rid != null, "$press carries no rid to hear its answer by")
            }
        }
    }

    @Test
    fun staleHooksOfferAnUpdateAndNotBothScopes() = runComposeUiTest {
        val hooked = Hooked(hooks("stale", scope = "project"))
        setContent { Screen(hooked) }
        waitForIdle()
        assertTrue(shown(UPDATE) && shown(NOT_NOW) && shown(NEVER))
        assertTrue(!shown(ALL) && !shown(PROJECT))
    }

    @Test
    fun notNowHidesItUntilTheConversationIsOpenedAgain() = runComposeUiTest {
        val hooked = Hooked(hooks("absent"))
        setContent { Screen(hooked) }
        waitForIdle()
        onNodeWithContentDescription(NOT_NOW).performClick()
        waitForIdle()
        assertTrue(!shown(ALL))
        assertTrue(hooked.io.manage.isEmpty(), "not now told the node something")

        runOnIdle { hooked.store.accept(Wire.decode(hooks("absent"))!!) }
        waitForIdle()
        assertTrue(!shown(ALL), "the next claude.hooks frame brought it back")

        runOnIdle { hooked.showing.value = false }
        waitForIdle()
        runOnIdle { hooked.showing.value = true }
        waitForIdle()
        assertTrue(shown(ALL))
    }

    @Test
    fun anAskInFlightDisablesThePressesAndARefusalIsSaidInline() = runComposeUiTest {
        val hooked = Hooked(hooks("absent"))
        setContent { Screen(hooked) }
        waitForIdle()
        onNodeWithContentDescription(ALL).performClick()
        waitForIdle()
        for (label in listOf(ALL, PROJECT, NEVER)) onNodeWithContentDescription(label).assertIsNotEnabled()

        val rid = hooked.io.manage.single().rid
        runOnIdle {
            hooked.store.accept(
                ServerMsg.Managed("claude.hooks.install", ok = false, id = null, rid = rid, message = "~/.claude/settings.json is not valid JSON"),
            )
        }
        waitForIdle()
        onNodeWithText("~/.claude/settings.json is not valid JSON", substring = true).assertExists()
        onNodeWithContentDescription(ALL).assertIsEnabled()
    }

    @Test
    fun theNodeSayingItIsCurrentTakesItDown() = runComposeUiTest {
        val hooked = Hooked(hooks("absent"))
        setContent { Screen(hooked) }
        waitForIdle()
        onNodeWithContentDescription(ALL).performClick()
        waitForIdle()
        runOnIdle {
            hooked.store.accept(Wire.decode(hooks("current", scope = "user"))!!)
            hooked.store.accept(ServerMsg.Managed("claude.hooks.install", ok = true, id = null, rid = hooked.io.manage.single().rid))
        }
        waitForIdle()
        assertTrue(!shown(ALL) && !shown(NOT_NOW))
    }

    // The offer must never cost the operator the question or the reply box on a phone.
    @Test
    fun itCoversNeitherTheQuestionNorTheReplyBox() = runComposeUiTest {
        val hooked = Hooked(hooks("absent"))
        hooked.store.accept(
            ServerMsg.Pending(
                pane = PANE_ID,
                question = "Run the workspace tests",
                header = "Bash",
                source = "hook",
                context = "cargo test --workspace",
                options = listOf(PendingOption("1", "Yes"), PendingOption("3", "No")),
            ),
        )
        setContent { Screen(hooked) }
        waitForIdle()
        val offer = onNodeWithContentDescription(ALL).fetchSemanticsNode().boundsInRoot
        val no = onNodeWithContentDescription("Answer 3, No").fetchSemanticsNode().boundsInRoot
        val reply = onNodeWithContentDescription("Reply to claude").fetchSemanticsNode().boundsInRoot
        assertTrue(!offer.overlaps(no), "the offer at $offer covers the question's answer at $no")
        assertTrue(!offer.overlaps(reply), "the offer at $offer covers the reply box at $reply")
        onNodeWithContentDescription("Answer 3, No").performClick()
        waitForIdle()
        assertTrue(hooked.io.sent.any { it is ClientMsg.Answer && it.key == "3" })
    }
}
