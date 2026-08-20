package dev.kampr.shared.theme

import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.produceState
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.platform.Font
import org.jetbrains.compose.resources.getFontResourceBytes
import org.jetbrains.compose.resources.rememberResourceEnvironment

@Composable
actual fun rememberFamily(faces: List<FontFace>): FontFamily? {
    val environment = rememberResourceEnvironment()
    val family by produceState<FontFamily?>(null, faces, environment) {
        value = FontFamily(
            faces.map { face ->
                Font(face.id, getFontResourceBytes(environment, face.resource), face.weight, face.style)
            }
        )
    }
    return family
}
