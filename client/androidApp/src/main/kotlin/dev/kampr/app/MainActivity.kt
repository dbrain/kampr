package dev.kampr.app

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import dev.kampr.shared.platform.KamprAndroid
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
    override fun onCreate(savedInstanceState: Bundle?) {
        KamprAndroid.attach(applicationContext)
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        val bench = intent?.getBooleanExtra("bench", false) == true
        setContent { if (bench) TerminalBenchApp() else KamprApp(surfaces, mosaic = mosaic) }
    }
}
