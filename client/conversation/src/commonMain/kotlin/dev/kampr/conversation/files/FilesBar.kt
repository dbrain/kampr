package dev.kampr.conversation.files

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.absolutePadding
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.text.selection.DisableSelection
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.BackAction
import dev.kampr.shared.ui.CloseAction
import dev.kampr.shared.ui.FilesNav
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.asHeading
import dev.kampr.shared.ui.edgeBottom

@Composable
fun FilesBar(title: String, subtitle: String?, nav: FilesNav, actions: @Composable RowScope.() -> Unit = {}) {
    val tokens = Kampr.tokens
    val safe = LocalSafeArea.current
    DisableSelection {
        Row(
            Modifier
                .fillMaxWidth()
                .background(tokens.color.bar)
                .edgeBottom()
                .absolutePadding(left = 4.dp + safe.left, right = 4.dp + safe.right, top = 4.dp + safe.top, bottom = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            BackAction("Back from $title", nav::back)
            Column(Modifier.weight(1f)) {
                KText(title, tokens.type.meta, tokens.color.text, Modifier.asHeading())
                subtitle?.let { KText(it, tokens.type.micro, tokens.color.mute) }
            }
            actions()
            CloseAction("Close $title", nav::close)
        }
    }
}
