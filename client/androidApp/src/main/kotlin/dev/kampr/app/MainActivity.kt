package dev.kampr.app

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import dev.kampr.shared.platform.KamprAndroid
import dev.kampr.shared.ui.KamprApp

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        KamprAndroid.attach(applicationContext)
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        setContent { KamprApp() }
    }
}
