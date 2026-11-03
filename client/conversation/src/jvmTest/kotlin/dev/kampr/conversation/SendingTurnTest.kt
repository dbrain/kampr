package dev.kampr.conversation

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.v2.runComposeUiTest
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.model.UNSEEN_AFTER_MS
import dev.kampr.shared.net.AttachmentBytes
import dev.kampr.shared.platform.PickedFile
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.LocalConnectionStatus
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.wire.Attachment
import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.Facets
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.shared.wire.Queued
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Turn
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private class Wire : PaneIo {
    var pane: dev.kampr.shared.model.PaneState? = null
    var down = false
    val sent = mutableListOf<ClientMsg>()
    override fun send(msg: ClientMsg) {
        if (down) pane?.noteUndelivered() else sent += msg
    }
    override fun prefs(paneId: String): PanePrefs = PanePrefs()

    val fetched = mutableListOf<String>()
    override suspend fun attachment(paneId: String, id: String): AttachmentBytes {
        fetched += paneId
        return AttachmentBytes.Ok(byteArrayOf(1, 2, 3), "image/png")
    }

    val typed get() = sent.filterIsInstance<ClientMsg.InputText>().map { it.text }
}

private class Session {
    val store = KamprStore()
    val pane = store.pane(PANE_ID)
    val io = Wire().also { it.pane = pane }
    val showing = mutableStateOf(true)
    private var n = 0

    fun recorded(text: String, vararg more: Block, picture: Boolean = false) {
        val first = Block.Md(text, att = if (picture) Attachment("p", "image", "image/png") else null)
        store.accept(ServerMsg.ConvoTurn(PANE_ID, listOf(Turn("u${n++}", "user", blocks = listOf(first) + more))))
    }

    fun answered(text: String) {
        store.accept(ServerMsg.ConvoTurn(PANE_ID, listOf(Turn("a${n++}", "assistant", blocks = listOf(Block.Md(text))))))
    }

    fun queued(vararg texts: String) {
        store.accept(ServerMsg.ConvoFacets(PANE_ID, Facets(queued = texts.map { Queued(it) })))
    }
}

@Composable
private fun Screen(session: Session) {
    CompositionLocalProvider(
        LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
        LocalConnectionStatus provides ConnectionStatus.Live("full"),
        LocalPaneIo provides session.io,
    ) {
        Box(Modifier.fillMaxSize()) {
            if (session.showing.value) ConversationView(session.pane, demoInfo(), Modifier.fillMaxSize())
        }
    }
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.send(words: String) {
    onNodeWithContentDescription("Reply to claude").performTextInput(words)
    onNodeWithContentDescription("Send this reply to claude").performClick()
    waitForIdle()
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.count(text: String, substring: Boolean = false) = onAllNodesWithText(text, substring = substring, ignoreCase = true).fetchSemanticsNodes().size

// A reply used to appear only once the harness had written it down, which on a session still
// starting is many seconds of a send button that seemed to have done nothing. It stands at the foot
// of the transcript from the press, marked as not yet a record, until the record replaces it.
@OptIn(ExperimentalTestApi::class)
class SendingTurnTest {
    @Test
    fun aReplyStandsAtTheFootFromThePressBeforeAnyRecord() = runComposeUiTest {
        val session = Session()
        setContent { Screen(session) }

        send("hello there")

        assertEquals(listOf("hello there", "\r"), session.io.typed)
        onNodeWithText("hello there").assertExists()
        assertEquals(1, count("sending…"))
    }

    @Test
    fun theMatchingRecordRetiresItAndAnOlderIdenticalOneDoesNot() = runComposeUiTest {
        val session = Session()
        setContent { Screen(session) }
        runOnIdle { session.recorded("hello there") }
        waitForIdle()

        send("hello there")
        runOnIdle { session.answered("looking") }
        waitForIdle()
        assertEquals(1, count("sending…"), "a record from before the press was taken for this one")

        runOnIdle { session.recorded("hello there\n") }
        waitForIdle()
        assertEquals(0, count("sending…"))
        assertEquals(2, count("hello there"), "the reply is drawn twice, or not at all")
    }

    @Test
    fun aQueuedEntryRetiresIt() = runComposeUiTest {
        val session = Session()
        setContent { Screen(session) }

        send("and copy the config across")
        runOnIdle { session.queued("and copy the config across") }
        waitForIdle()

        assertEquals(0, count("sending…"))
        assertEquals(1, count("queued"))
        assertEquals(1, count("and copy the config across"))

        runOnIdle {
            session.queued()
            session.recorded("and copy the config across")
        }
        waitForIdle()
        assertEquals(1, count("and copy the config across"))
    }

    // The queued entry already stood for the first reply, so the record it becomes is that reply's
    // and not the second one's.
    @Test
    fun aQueuedReplysRecordIsNotTakenForALaterIdenticalOne() = runComposeUiTest {
        val session = Session()
        setContent { Screen(session) }

        send("keep going")
        runOnIdle { session.queued("keep going") }
        waitForIdle()
        send("keep going")
        assertEquals(1, count("sending…"))

        runOnIdle {
            session.queued()
            session.recorded("keep going")
        }
        waitForIdle()
        assertEquals(1, count("sending…"), "the first reply's record retired the second")

        runOnIdle { session.recorded("keep going") }
        waitForIdle()
        assertEquals(0, count("sending…"))
    }

    @Test
    fun twoIdenticalRepliesNeedTwoRecords() = runComposeUiTest {
        val session = Session()
        setContent { Screen(session) }

        send("again")
        send("again")
        assertEquals(2, count("sending…"))

        runOnIdle {
            session.recorded("again")
            session.answered("on it")
        }
        waitForIdle()
        assertEquals(1, count("sending…"), "one record retired both replies")

        runOnIdle { session.recorded("again") }
        waitForIdle()
        assertEquals(0, count("sending…"))
    }

    // What muffit records once it has lifted the picture out, and what the node shows for Claude's
    // record once it has dressed the path as the picture: neither carries the path that was sent.
    // The picture a reply carries is drawn on its card before the record comes, and it is fetched
    // for this pane: a card that asked for no pane at all was told the node no longer had it.
    @Test
    fun aPictureOnTheCardIsFetchedForThisPane() = runComposeUiTest {
        val session = Session()
        setContent { Screen(session) }
        val path = "/home/u/.local/state/kampr/pastes/shot-1762061518000.png"

        runOnIdle {
            attach(session.pane, session.io, PickedFile("shot.png", "image/png", byteArrayOf(1, 2, 3)))
            val id = session.pane.files.all.single().id
            session.store.accept(ServerMsg.Pasted(PANE_ID, id, path = path, text = "$path "))
        }
        waitForIdle()
        send("what is this")
        waitUntil { session.io.fetched.isNotEmpty() }
        assertEquals(listOf(PANE_ID), session.io.fetched.distinct())
    }

    @Test
    fun theFilesARideCarriedDoNotStopItsRecordMatching() = runComposeUiTest {
        val session = Session()
        setContent { Screen(session) }
        val path = "/home/u/.local/state/kampr/pastes/shot-1762061518000.png"

        runOnIdle {
            attach(session.pane, session.io, PickedFile("shot.png", "image/png", byteArrayOf(1, 2, 3)))
            val id = session.pane.files.all.single().id
            session.store.accept(ServerMsg.Pasted(PANE_ID, id, path = path, text = "$path "))
        }
        waitForIdle()
        send("what is this")
        assertEquals(1, count("sending…"))

        runOnIdle { session.recorded("what is this") }
        waitForIdle()
        assertEquals(0, count("sending…"))

        runOnIdle {
            attach(session.pane, session.io, PickedFile("shot.png", "image/png", byteArrayOf(1, 2, 3)))
            val id = session.pane.files.all.single().id
            session.store.accept(ServerMsg.Pasted(PANE_ID, id, path = path, text = "$path "))
        }
        waitForIdle()
        send("and this")
        runOnIdle { session.recorded("[image · png]", Block.Md("and this"), picture = true) }
        waitForIdle()
        assertEquals(0, count("sending…"))
    }

    @Test
    fun aRecordWithMuffitsLabelWhereThePictureStoodStillMatches() = runComposeUiTest {
        val session = Session()
        setContent { Screen(session) }
        val path = "/home/u/.local/state/kampr/pastes/shot-1762061518000.png"

        runOnIdle {
            attach(session.pane, session.io, PickedFile("shot.png", "image/png", byteArrayOf(1, 2, 3)))
            val id = session.pane.files.all.single().id
            session.store.accept(ServerMsg.Pasted(PANE_ID, id, path = path, text = "$path "))
        }
        waitForIdle()
        send("what is this")
        runOnIdle { session.recorded("[image 1: shot-1762061518000.png] what is this") }
        waitForIdle()
        assertEquals(0, count("sending…"))
    }

    @Test
    fun keysThatNeverLeftAreFailedAtOnceAndResendTypesThemAgain() = runComposeUiTest {
        val session = Session()
        setContent { Screen(session) }
        session.io.down = true

        send("are you there")
        assertEquals(1, count("not delivered"))
        assertEquals(0, count("sending…"))

        session.io.down = false
        onNodeWithContentDescription("Resend this reply").performClick()
        waitForIdle()
        assertEquals(listOf("are you there", "\r"), session.io.typed)
        assertEquals(1, count("sending…"))
        assertEquals(0, count("not delivered"))

        runOnIdle { session.recorded("are you there") }
        waitForIdle()
        assertEquals(0, count("sending…"))
    }

    // Not "failed": the keys left, and a harness still starting may simply not have written the
    // record yet. A record that turns up after the flag is still the reply.
    @Test
    fun noRecordInTheWindowIsNotSeenYetAndALateRecordStillRetiresIt() = runComposeUiTest {
        val session = Session()
        setContent { Screen(session) }

        send("start with the tests")
        mainClock.advanceTimeBy(UNSEEN_AFTER_MS + 1_000)
        waitForIdle()
        assertEquals(1, count("not seen yet"))
        onNodeWithContentDescription("Resend this reply").assertExists()
        onNodeWithContentDescription("Dismiss this reply").assertExists()

        runOnIdle { session.recorded("start with the tests") }
        waitForIdle()
        assertEquals(0, count("not seen yet"))
        assertEquals(1, count("start with the tests"))
    }

    @Test
    fun aFlaggedReplyIsDismissed() = runComposeUiTest {
        val session = Session()
        setContent { Screen(session) }
        session.io.down = true

        send("never mind")
        onNodeWithContentDescription("Dismiss this reply").performClick()
        waitForIdle()
        assertEquals(0, count("never mind"))
    }

    @Test
    fun itSurvivesTheConversationLeavingAndComingBack() = runComposeUiTest {
        val session = Session()
        setContent { Screen(session) }

        send("hold on to this")
        runOnIdle { session.showing.value = false }
        waitForIdle()
        runOnIdle { session.showing.value = true }
        waitForIdle()
        assertEquals(1, count("sending…"))
        onNodeWithText("hold on to this").assertExists()
    }

    @Test
    fun theRecordTakesThePendingCardsPlaceWithoutAJump() = runComposeUiTest {
        val session = Session()
        setContent { Screen(session) }
        runOnIdle { repeat(40) { session.answered("an earlier answer, line $it, long enough to fill some of the screen") } }
        waitForIdle()

        send("where does this land")
        val before = onNodeWithText("where does this land").getUnclippedBoundsInRoot()
        runOnIdle { session.recorded("where does this land") }
        waitForIdle()
        val after = onNodeWithText("where does this land").getUnclippedBoundsInRoot()

        assertEquals(0, count("sending…"))
        assertEquals(before.top, after.top, "the reply moved when its record replaced it")
        assertTrue(before.bottom == after.bottom)
    }
}
