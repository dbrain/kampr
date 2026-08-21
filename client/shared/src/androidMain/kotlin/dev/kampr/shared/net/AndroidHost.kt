package dev.kampr.shared.net

import android.app.Activity
import java.lang.ref.WeakReference

// Credential Manager raises system UI, so it needs the Activity rather than the Application: given
// an application context it has no task to draw into and refuses. `AppState` is built before any
// composition, so the Activity cannot come from a composition local either — it is handed over by
// the one class that has it.
object KamprHost {
    private var current: WeakReference<Activity> = WeakReference(null)

    fun attach(activity: Activity) {
        current = WeakReference(activity)
    }

    fun detach(activity: Activity) {
        if (current.get() === activity) current = WeakReference(null)
    }

    val activity: Activity?
        get() = current.get()?.takeIf { !it.isFinishing && !it.isDestroyed }
}
