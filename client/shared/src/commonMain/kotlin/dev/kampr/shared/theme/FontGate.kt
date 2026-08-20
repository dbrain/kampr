package dev.kampr.shared.theme

import androidx.compose.runtime.Composable
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import org.jetbrains.compose.resources.FontResource

data class FontFace(
    val id: String,
    val resource: FontResource,
    val weight: FontWeight,
    val style: FontStyle = FontStyle.Normal,
)

// Probe #65: a resource font resolves asynchronously and can beat first layout, so the
// family stays null until every face has bytes rather than letting a fallback be measured.
@Composable
expect fun rememberFamily(faces: List<FontFace>): FontFamily?
