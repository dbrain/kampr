package dev.kampr.app

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import dev.kampr.shared.push.BLOCKED_CHANNEL
import dev.kampr.shared.push.BLOCKED_NOTIFICATION_ID

// A summary that says there is *less* waiting than there was: the pane that was answered somewhere
// else leaves the shade, the rest stay named, and nothing buzzes. A phone that vibrates to report
// work being taken away is a phone that gets muted.
fun postBlockedResync(context: Context, title: String, body: String, pane: String?) {
    val manager = context.getSystemService(NotificationManager::class.java) ?: return
    // A resync corrects a prompt; it never conjures one. Somebody who swiped the notification away
    // has already dealt with it, and posting a quieter copy of what they dismissed is the app
    // arguing with them.
    if (manager.activeNotifications.none { it.id == BLOCKED_NOTIFICATION_ID }) return
    post(context, manager, title, body, pane, alert = false)
}

fun clearBlockedNotification(context: Context) {
    context.getSystemService(NotificationManager::class.java)?.cancel(BLOCKED_NOTIFICATION_ID)
}

fun postBlockedNotification(context: Context, title: String, body: String, pane: String?) {
    val manager = context.getSystemService(NotificationManager::class.java) ?: return
    post(context, manager, title, body, pane, alert = true)
}

private fun post(
    context: Context,
    manager: NotificationManager,
    title: String,
    body: String,
    pane: String?,
    alert: Boolean,
) {
    manager.createNotificationChannel(
        NotificationChannel(BLOCKED_CHANNEL, "Blocked agents", NotificationManager.IMPORTANCE_HIGH),
    )
    // One blocked pane opens that pane in its conversation view — the view an answer can be
    // given from without leasing a terminal. No pane opens the triage list, because picking
    // one of three for the user would be picking wrong two times in three.
    val intent = Intent(context, MainActivity::class.java)
        .setFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP)
    if (pane != null) {
        intent.putExtra("pane", pane).putExtra("view", "conversation")
    } else {
        intent.putExtra("screen", "herd")
    }
    manager.notify(
        BLOCKED_NOTIFICATION_ID,
        Notification.Builder(context, BLOCKED_CHANNEL)
            .setContentTitle(title)
            .setContentText(body)
            .setStyle(Notification.BigTextStyle().bigText(body))
            .setSmallIcon(R.drawable.ic_kampr_notification)
            .setAutoCancel(true)
            // The channel is high-importance because a blocked agent is worth a buzz. This is what
            // stops an update to a notification already in the shade buzzing a second time.
            .setOnlyAlertOnce(!alert)
            .setContentIntent(
                PendingIntent.getActivity(
                    context,
                    0,
                    intent,
                    PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
                ),
            )
            .build(),
    )
}
