package dev.kampr.shared.ui

import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.ime
import androidx.compose.runtime.Composable
import androidx.compose.ui.unit.Dp

@Composable
internal actual fun imeInset(): Dp = WindowInsets.ime.asPaddingValues().calculateBottomPadding()
