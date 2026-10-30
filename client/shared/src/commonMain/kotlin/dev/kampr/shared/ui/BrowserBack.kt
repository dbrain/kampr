package dev.kampr.shared.ui

import androidx.compose.runtime.Composable

@Composable
internal expect fun BrowserBack(wanted: () -> Boolean, back: () -> Unit)
