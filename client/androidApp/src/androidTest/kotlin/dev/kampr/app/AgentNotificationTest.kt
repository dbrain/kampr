package dev.kampr.app

import android.Manifest
import android.app.NotificationManager
import android.service.notification.StatusBarNotification
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test

// The notification is the entire reason a native client exists, and it is the app surface most
// exposed to a targetSdk bump. This asserts it against the real NotificationManager on a device.
class AgentNotificationTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val context = instrumentation.targetContext
    private val manager = context.getSystemService(NotificationManager::class.java)

    @Before
    fun grant() {
        instrumentation.uiAutomation
            .grantRuntimePermission(context.packageName, Manifest.permission.POST_NOTIFICATIONS)
        clear()
    }

    @After
    fun clear() = NoteKind.entries.forEach { manager.cancel(it.id) }

    @Test
    fun aBlockedPaneReachesTheShade() {
        postAgentNotification(context, NoteKind.Blocked, "claude is blocked", "wants your approval", "pane-7")

        val posted = shade(NoteKind.Blocked).notification
        assertEquals(NoteKind.Blocked.channel, posted.channelId)
        assertEquals("claude is blocked", posted.extras.getString("android.title"))
        assertEquals("wants your approval", posted.extras.getString("android.text"))
        assertNotNull("no content intent — tapping the shade would do nothing", posted.contentIntent)
    }

    @Test
    fun aFinishedAgentReachesTheShadeTooAndOnItsOwnChannel() {
        postAgentNotification(context, NoteKind.Done, "claude · kampr finished", "~/dev/kampr", "pane-7")

        val posted = shade(NoteKind.Done).notification
        assertEquals(NoteKind.Done.channel, posted.channelId)
        assertEquals("claude · kampr finished", posted.extras.getString("android.title"))
        assertNotNull(posted.contentIntent)
    }

    // **The reason the two kinds are two slots.** One id is one notification: whatever arrives
    // last is the only thing in the shade. An agent finishing while another is asking a question
    // is the ordinary case, and it must not take the question off the phone.
    //
    // Mutation: give NoteKind.Done the blocked id and this drops to one notification, with the
    // question gone.
    @Test
    fun aFinishedAgentDoesNotDisplaceAQuestionThatIsStillWaiting() {
        postAgentNotification(context, NoteKind.Blocked, "claude needs you", "Proceed?", "pane-1")
        shade(NoteKind.Blocked)

        postAgentNotification(context, NoteKind.Done, "codex finished", "~/dev/herdr", "pane-2")
        shade(NoteKind.Done)

        assertEquals(
            "the question is still on the phone",
            "claude needs you",
            shade(NoteKind.Blocked).notification.extras.getString("android.title"),
        )
        assertEquals(2, ours(NoteKind.Blocked).size + ours(NoteKind.Done).size)
    }

    // A question is worth the screen; a finished agent is worth knowing about without taking it
    // over. An Android channel's importance is fixed per channel, so this is the whole reason one
    // channel could not have served both.
    @Test
    fun eachKindsChannelCarriesTheImportanceItsEventIsWorth() {
        postAgentNotification(context, NoteKind.Blocked, "claude is blocked", "wants your approval", null)
        postAgentNotification(context, NoteKind.Done, "claude finished", "~/dev/kampr", null)

        assertEquals(
            NotificationManager.IMPORTANCE_HIGH,
            manager.getNotificationChannel(NoteKind.Blocked.channel).importance,
        )
        assertEquals(
            NotificationManager.IMPORTANCE_DEFAULT,
            manager.getNotificationChannel(NoteKind.Done.channel).importance,
        )
        assertTrue(
            "a finish must not be able to interrupt as hard as a question",
            manager.getNotificationChannel(NoteKind.Done.channel).importance <
                manager.getNotificationChannel(NoteKind.Blocked.channel).importance,
        )
    }

    @Test
    fun theNewestNotificationOfAKindReplacesTheLastRatherThanStacking() {
        for (kind in NoteKind.entries) {
            postAgentNotification(context, kind, "first", "one", "pane-1")
            postAgentNotification(context, kind, "second", "two", "pane-2")

            assertEquals("second", shade(kind, "second").notification.extras.getString("android.title"))
            assertEquals(1, ours(kind).size)
            manager.cancel(kind.id)
        }
    }

    // The defect this guards: a prompt answered at the desk used to sit on the phone until somebody
    // tapped it, because the node only ever sent rising edges and one id means the last notification
    // stands until another replaces it. A finish read at the desk falls the same way (#357, #396).
    @Test
    fun clearingTakesTheNotificationDown() {
        for (kind in NoteKind.entries) {
            postAgentNotification(context, kind, "claude", "something", "pane-7")
            shade(kind)

            clearAgentNotification(context, kind)
            awaitGone(kind)
        }
    }

    private fun awaitGone(kind: NoteKind) {
        repeat(50) {
            if (ours(kind).isEmpty()) return
            Thread.sleep(100)
        }
        throw AssertionError("$kind was still in the shade 5s after it was dealt with")
    }

    // Answering one of two leaves the other one waiting, and its notification has to say so.
    @Test
    fun aResyncRewritesANotificationThatIsStillShowing() {
        postAgentNotification(context, NoteKind.Blocked, "2 agents need you", "one · two", null)
        shade(NoteKind.Blocked, "2 agents need you")

        postAgentResync(context, NoteKind.Blocked, "codex needs you", "Apply the patch?", "pane-2")

        val posted = shade(NoteKind.Blocked, "codex needs you").notification
        assertEquals("Apply the patch?", posted.extras.getString("android.text"))
        assertEquals(
            "a resync replaces the notification rather than stacking beside it",
            1,
            ours(NoteKind.Blocked).size,
        )
    }

    // Somebody who swiped the notification away has already dealt with it. A quieter copy of what
    // they dismissed is the app arguing with them — and it is what a naive "just post the new
    // summary" would do on every pane that left a set.
    @Test
    fun aResyncDoesNotConjureANotificationThatWasDismissed() {
        for (kind in NoteKind.entries) {
            manager.cancel(kind.id)

            postAgentResync(context, kind, "codex", "something", "pane-2")

            Thread.sleep(500)
            assertEquals("nothing was on the screen, so there was nothing to correct", 0, ours(kind).size)
        }
    }

    // A resync of one kind must not correct — or conjure — the other kind's slot.
    @Test
    fun aResyncOfOneKindLeavesTheOtherKindAlone() {
        postAgentNotification(context, NoteKind.Blocked, "claude needs you", "Proceed?", "pane-1")
        shade(NoteKind.Blocked)

        postAgentResync(context, NoteKind.Done, "codex finished", "~/dev/herdr", "pane-2")

        Thread.sleep(500)
        assertEquals("nothing was showing for finished agents", 0, ours(NoteKind.Done).size)
        assertEquals(
            "claude needs you",
            shade(NoteKind.Blocked).notification.extras.getString("android.title"),
        )
    }

    private fun ours(kind: NoteKind): List<StatusBarNotification> =
        manager.activeNotifications.filter { it.id == kind.id }

    // NotificationManager.notify() crosses a binder and lands in the shade asynchronously, so
    // reading activeNotifications on the next line is a race the first assertion always loses.
    private fun shade(kind: NoteKind, title: String? = null): StatusBarNotification {
        repeat(50) {
            ours(kind).singleOrNull { n ->
                title == null || n.notification.extras.getString("android.title") == title
            }?.let { return it }
            Thread.sleep(100)
        }
        throw AssertionError("no $kind notification reached the shade within 5s")
    }
}
