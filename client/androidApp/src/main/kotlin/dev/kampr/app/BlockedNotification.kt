package dev.kampr.app

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent

// Must match `kampr_push::note::TAG` and the service worker's, so the newest replaces the last
// rather than stacking a column of stale prompts on a phone that was away.
const val BLOCKED_CHANNEL = "kampr.blocked"
const val BLOCKED_NOTIFICATION_ID = 1

fun postBlockedNotification(context: Context, title: String, body: String, pane: String?) {
    val manager = context.getSystemService(NotificationManager::class.java) ?: return
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
