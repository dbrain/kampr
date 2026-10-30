package dev.kampr.shared.net

import android.app.Activity
import android.app.Application
import android.os.Bundle
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleOwner
import dev.kampr.shared.platform.KamprAndroid

// The application object rather than androidx.lifecycle: `ProcessLifecycleOwner` needs
// lifecycle-process on the classpath and this needs nothing that is not already there.
actual fun watchForeground(onChange: (shown: Boolean) -> Unit): ForegroundWatch {
    val app = KamprAndroid.context as? Application ?: return ForegroundWatch {}
    val callbacks = object : Application.ActivityLifecycleCallbacks {
        // Usually registered from inside a started activity, which reports no start of its own;
        // counted from zero, the first background would go unheard.
        private var started = if ((KamprHost.activity as? LifecycleOwner)?.lifecycle?.currentState?.isAtLeast(Lifecycle.State.STARTED) == true) 1 else 0

        override fun onActivityStarted(activity: Activity) {
            if (started++ == 0) onChange(true)
        }

        override fun onActivityStopped(activity: Activity) {
            if (started > 0 && --started == 0) onChange(false)
        }

        override fun onActivityCreated(activity: Activity, savedInstanceState: Bundle?) = Unit
        override fun onActivityResumed(activity: Activity) = Unit
        override fun onActivityPaused(activity: Activity) = Unit
        override fun onActivitySaveInstanceState(activity: Activity, outState: Bundle) = Unit
        override fun onActivityDestroyed(activity: Activity) = Unit
    }
    app.registerActivityLifecycleCallbacks(callbacks)
    return ForegroundWatch { app.unregisterActivityLifecycleCallbacks(callbacks) }
}
