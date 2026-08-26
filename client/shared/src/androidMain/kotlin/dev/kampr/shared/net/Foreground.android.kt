package dev.kampr.shared.net

import android.app.Activity
import android.app.Application
import android.os.Bundle
import dev.kampr.shared.platform.KamprAndroid

// The application object rather than androidx.lifecycle: `ProcessLifecycleOwner` needs
// lifecycle-process on the classpath and this needs nothing that is not already there.
actual fun watchForeground(onForeground: () -> Unit): ForegroundWatch {
    val app = KamprAndroid.context as? Application ?: return ForegroundWatch {}
    val callbacks = object : Application.ActivityLifecycleCallbacks {
        private var started = 0

        override fun onActivityStarted(activity: Activity) {
            if (started++ == 0) onForeground()
        }

        override fun onActivityStopped(activity: Activity) {
            if (started > 0) started--
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
