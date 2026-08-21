package dev.kampr.app

import android.Manifest
import android.app.NotificationManager
import android.service.notification.StatusBarNotification
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Before
import org.junit.Test

// The notification is the entire reason a native client exists, and it is the app surface most
// exposed to a targetSdk bump. This asserts it against the real NotificationManager on a device.
class BlockedNotificationTest {
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
    fun clear() = manager.cancel(BLOCKED_NOTIFICATION_ID)

    @Test
    fun aBlockedPaneReachesTheShade() {
        postBlockedNotification(context, "claude is blocked", "wants your approval", "pane-7")

        val posted = shade().notification
        assertEquals(BLOCKED_CHANNEL, posted.channelId)
        assertEquals("claude is blocked", posted.extras.getString("android.title"))
        assertEquals("wants your approval", posted.extras.getString("android.text"))
        assertNotNull("no content intent — tapping the shade would do nothing", posted.contentIntent)
    }

    @Test
    fun theChannelIsHighImportanceSoItIsNotSilent() {
        postBlockedNotification(context, "claude is blocked", "wants your approval", null)

        assertEquals(
            NotificationManager.IMPORTANCE_HIGH,
            manager.getNotificationChannel(BLOCKED_CHANNEL).importance,
        )
    }

    @Test
    fun theNewestPromptReplacesTheLastRatherThanStacking() {
        postBlockedNotification(context, "first", "one", "pane-1")
        postBlockedNotification(context, "second", "two", "pane-2")

        assertEquals("second", shade("second").notification.extras.getString("android.title"))
        assertEquals(1, ours().size)
    }

    private fun ours(): List<StatusBarNotification> =
        manager.activeNotifications.filter { it.id == BLOCKED_NOTIFICATION_ID }

    // NotificationManager.notify() crosses a binder and lands in the shade asynchronously, so
    // reading activeNotifications on the next line is a race the first assertion always loses.
    private fun shade(title: String? = null): StatusBarNotification {
        repeat(50) {
            ours().singleOrNull { n ->
                title == null || n.notification.extras.getString("android.title") == title
            }?.let { return it }
            Thread.sleep(100)
        }
        throw AssertionError("no Kampr notification reached the shade within 5s")
    }
}
