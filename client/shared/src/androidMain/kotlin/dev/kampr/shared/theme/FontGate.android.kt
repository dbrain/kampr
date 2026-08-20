package dev.kampr.shared.theme

import androidx.compose.runtime.Composable
import androidx.compose.ui.text.font.FontFamily
import org.jetbrains.compose.resources.Font

@Composable
actual fun rememberFamily(faces: List<FontFace>): FontFamily? =
    FontFamily(faces.map { Font(it.resource, it.weight, it.style) })
