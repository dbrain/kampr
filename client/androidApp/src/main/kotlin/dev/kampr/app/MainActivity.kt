package dev.kampr.app

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import dev.kampr.shared.platform.KamprAndroid
import dev.kampr.shared.ui.DeepLink
import dev.kampr.shared.ui.KamprApp
import dev.kampr.conversation.ConversationSurfaces
import dev.kampr.mosaic.MosaicSurfaces
import dev.kampr.terminal.TerminalSurfaces
import dev.kampr.terminal.bench.TerminalBenchApp

// ConversationSurfaces wraps: it renders the transcript and delegates the terminal and the
// key row to its base, so both halves of the pane are live.
private val surfaces = ConversationSurfaces(TerminalSurfaces())
private val mosaic = MosaicSurfaces()

class MainActivity : ComponentActivity() {
    private var link by mutableStateOf<DeepLink?>(null)

    override fun onCreate(savedInstanceState: Bundle?) {
        KamprAndroid.attach(applicationContext)
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        val bench = intent?.getBooleanExtra("bench", false) == true
        link = linkOf(intent)
        askForPermissions()
        setContent { if (bench) TerminalBenchApp() else KamprApp(surfaces, link, mosaic) }
    }

    // The activity is `singleTop` from the notification, so a tap on a second blocked agent
    // arrives here rather than through `onCreate` — without this it would focus the app and show
    // whatever was already on screen.
    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        linkOf(intent)?.let { link = it }
    }

    private fun linkOf(intent: Intent?): DeepLink? {
        val pane = intent?.getStringExtra("pane")
        val view = intent?.getStringExtra("view")
        val screen = intent?.getStringExtra("screen")
        if (pane == null && view == null && screen == null) return null
        return DeepLink(screen = screen, view = view, pane = pane)
    }

    // Asked for here rather than inside the subscribe call, because a permission Android refuses
    // is one the notifications screen must be able to report as refused.
    private fun askForPermissions() {
        val missing = buildList {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                add(Manifest.permission.POST_NOTIFICATIONS)
            }
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.CINNAMON_BUN) {
                add(Manifest.permission.ACCESS_LOCAL_NETWORK)
            }
        }.filter { checkSelfPermission(it) != PackageManager.PERMISSION_GRANTED }
        if (missing.isNotEmpty()) requestPermissions(missing.toTypedArray(), 1)
    }
}
