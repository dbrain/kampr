package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performSemanticsAction
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.platform.MemoryPrefs
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.ui.AppState
import dev.kampr.shared.ui.Breakpoint
import dev.kampr.shared.ui.ManageLayer
import dev.kampr.shared.ui.NewSheet
import dev.kampr.shared.ui.Screen
import dev.kampr.shared.ui.Sheet
import dev.kampr.shared.wire.ManageOp
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Wire
import dev.kampr.shared.wire.fields
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private val NODE = NodeInfo(id = "01JNODE", name = "comingclean", kind = "local", native = true)
private const val CARD = "Muffit session,"
private const val START = "Start muffit session"

private fun hello(caps: String) = """{"t":"hello","protocol":1,"node_id":"01JNODE","node_name":"comingclean","build":"test",""" +
    """"role":"full","caps":{"manage":true$caps},"security":{"tier":0,"passkeys":false}}"""

private const val HERE = "01JNODE"
private const val PEER = "01JPEER"

private fun herd(here: String, peer: String) = """{"t":"herd","nodes":[""" +
    """{"id":"$HERE","name":"comingclean","kind":"local","native":true$here},""" +
    """{"id":"$PEER","name":"loft","kind":"peer","native":true$peer}],"panes":[]}"""

private class MuffitSheet {
    val sent = mutableListOf<ManageOp>()
    val created = mutableListOf<String>()
    var dismissed = false
    val outcome = mutableStateOf<ServerMsg.Managed?>(null)
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.open(sheet: MuffitSheet) {
    setContent {
        CompositionLocalProvider(LocalTokens provides phoneTokens()) {
            Box(Modifier.size(420.dp, 1200.dp)) {
                NewSheet(
                    breakpoint = Breakpoint.Portrait,
                    node = NODE,
                    pane = null,
                    nodes = listOf(NODE),
                    caps = null,
                    outcome = sheet.outcome.value,
                    onManage = { sheet.sent += it },
                    onNode = {},
                    onNodePicker = {},
                    onDismiss = { sheet.dismissed = true },
                    onCreated = { sheet.created += it },
                    muffit = true,
                )
            }
        }
    }
    onNodeWithContentDescription(CARD, substring = true).performSemanticsAction(SemanticsActions.OnClick)
    waitForIdle()
}

// docs/18 "Making and ending one": a directory and a first prompt, sent as `muffit.create`, and
// the pane the ack names is the one opened.
@OptIn(ExperimentalTestApi::class)
class NewSheetMuffitTest {
    // `nodes[].muffit` speaks for each node, a peer behind a hub included; `hello.caps.muffit`
    // stands in only for the connected node, and only on a node too old to send the flag.
    @Test
    fun theCardIsOfferedOnlyByANodeThatSaysItCanMakeOne() {
        data class Case(val caps: String, val here: String, val peer: String, val target: String, val offered: Boolean)
        val cases = listOf(
            Case("", "", "", HERE, false),
            Case(""","muffit":false""", "", "", HERE, false),
            Case(""","muffit":true""", "", "", HERE, true),
            Case(""","muffit":true""", "", "", PEER, false),
            Case("", "", ""","muffit":true""", PEER, true),
            Case("", ""","muffit":true""", "", HERE, true),
            Case(""","muffit":true""", ""","muffit":false""", "", HERE, false),
        )
        for (case in cases) {
            runComposeUiTest {
                val scope = CoroutineScope(SupervisorJob() + Dispatchers.Unconfined)
                val store = KamprStore()
                store.accept(Wire.decode(hello(case.caps))!!)
                store.accept(Wire.decode(herd(case.here, case.peer))!!)
                val state = AppState(scope, store, MemoryPrefs(), null).apply { go(Screen.Herd) }
                try {
                    state.openSheet(Sheet.New(case.target, null))
                    setContent {
                        CompositionLocalProvider(LocalTokens provides phoneTokens()) {
                            Box(Modifier.size(420.dp, 1200.dp)) { ManageLayer(state, store.herd.value, Breakpoint.Portrait) }
                        }
                    }
                    waitForIdle()
                    val shown = onAllNodesWithContentDescription(CARD, substring = true).fetchSemanticsNodes().isNotEmpty()
                    assertEquals(case.offered, shown, "$case")
                } finally {
                    scope.cancel()
                }
            }
        }
    }

    @Test
    fun theDirectoryAndPromptAreSentAndTheAckOpensWhatItMade() = runComposeUiTest {
        val sheet = MuffitSheet()
        open(sheet)
        onNodeWithContentDescription("directory").performTextInput(" /home/dbrain/dev/kampr ")
        onNodeWithContentDescription("first prompt").performTextInput("tidy the parser")
        waitForIdle()
        onNodeWithContentDescription(START).performClick()
        waitForIdle()
        val op = sheet.sent.single()
        assertEquals("muffit.create", op.op)
        assertEquals(
            buildJsonObject {
                put("node", JsonPrimitive(NODE.id))
                put("cwd", JsonPrimitive("/home/dbrain/dev/kampr"))
                put("prompt", JsonPrimitive("tidy the parser"))
            },
            op.fields(),
        )

        sheet.outcome.value = ServerMsg.Managed(op = "muffit.create", ok = true, id = "01JNODE/m:s9")
        waitForIdle()
        assertEquals(listOf("01JNODE/m:s9"), sheet.created)
        assertTrue(sheet.dismissed)
    }

    @Test
    fun aSessionNeedsAPromptToStart() = runComposeUiTest {
        val sheet = MuffitSheet()
        open(sheet)
        onNodeWithContentDescription(START).assertIsNotEnabled()
    }
}
