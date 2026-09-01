package dev.kampr.app

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import dev.kampr.shared.push.BLOCKED_CHANNEL
import dev.kampr.shared.push.BLOCKED_NOTIFICATION_ID
import dev.kampr.shared.push.DONE_CHANNEL
import dev.kampr.shared.push.DONE_NOTIFICATION_ID

// The two kinds the node sends, and the two slots they land in. `wire` is the word in the payload
// (`kampr_push::note::Kind`); a payload with no `kind` at all is from a node that only ever sent
// one, so it is the blocked one.
//
// The importances differ because the events do. A question is waiting on a person and is worth
// the screen; an agent that finished is worth knowing about without taking it over, which is what
// IMPORTANCE_DEFAULT buys — it makes a sound and does not heads-up. A user who wants even less
// than that can turn the channel down without touching the other.
enum class NoteKind(
    val wire: String,
    val channel: String,
    val id: Int,
    val label: String,
    val importance: Int,
) {
    Blocked("blocked", BLOCKED_CHANNEL, BLOCKED_NOTIFICATION_ID, "Blocked agents", NotificationManager.IMPORTANCE_HIGH),
    Done("done", DONE_CHANNEL, DONE_NOTIFICATION_ID, "Finished agents", NotificationManager.IMPORTANCE_DEFAULT),
    ;

    companion object {
        fun of(wire: String?): NoteKind = entries.firstOrNull { it.wire == wire } ?: Blocked
    }
}

// A summary that says there is *less* outstanding than there was: the pane that was dealt with
// somewhere else leaves the shade, the rest stay named, and nothing buzzes. A phone that vibrates
// to report work being taken away is a phone that gets muted.
fun postAgentResync(context: Context, kind: NoteKind, title: String, body: String, pane: String?) {
    val manager = context.getSystemService(NotificationManager::class.java) ?: return
    // A resync corrects a notification; it never conjures one. Somebody who swiped it away has
    // already dealt with it, and posting a quieter copy of what they dismissed is the app arguing
    // with them.
    if (manager.activeNotifications.none { it.id == kind.id }) return
    post(context, manager, kind, title, body, pane, alert = false)
}

fun clearAgentNotification(context: Context, kind: NoteKind) {
    context.getSystemService(NotificationManager::class.java)?.cancel(kind.id)
}

fun postAgentNotification(context: Context, kind: NoteKind, title: String, body: String, pane: String?) {
    val manager = context.getSystemService(NotificationManager::class.java) ?: return
    post(context, manager, kind, title, body, pane, alert = true)
}

private fun post(
    context: Context,
    manager: NotificationManager,
    kind: NoteKind,
    title: String,
    body: String,
    pane: String?,
    alert: Boolean,
) {
    manager.createNotificationChannel(NotificationChannel(kind.channel, kind.label, kind.importance))
    // One pane opens that pane in its conversation view — the view a blocked agent can be
    // answered from without leasing a terminal, and the view a finished one's work is read in. No
    // pane opens the triage list, because picking one of three for the user would be picking
    // wrong two times in three.
    val intent = Intent(context, MainActivity::class.java)
        .setFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP)
    if (pane != null) {
        intent.putExtra("pane", pane).putExtra("view", "conversation")
    } else {
        intent.putExtra("screen", "herd")
    }
    manager.notify(
        kind.id,
        Notification.Builder(context, kind.channel)
            .setContentTitle(title)
            .setContentText(body)
            .setStyle(Notification.BigTextStyle().bigText(body))
            .setSmallIcon(R.drawable.ic_kampr_notification)
            .setAutoCancel(true)
            // This is what stops an update to a notification already in the shade alerting a
            // second time.
            .setOnlyAlertOnce(!alert)
            .setContentIntent(
                PendingIntent.getActivity(
                    context,
                    // Per kind: two PendingIntents that differ only in their extras and share a
                    // request code are the *same* intent to the system, so the second `notify`
                    // would deep-link to the first one's pane.
                    kind.id,
                    intent,
                    PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
                ),
            )
            .build(),
    )
}
