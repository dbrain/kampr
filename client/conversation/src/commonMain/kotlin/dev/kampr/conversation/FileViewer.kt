package dev.kampr.conversation

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.DisableSelection
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.kampr.conversation.syntax.langOf
import dev.kampr.shared.net.pathOfAttachmentId
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.GlyphTarget
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.LANDSCAPE_TOUCH
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.QuietAction
import dev.kampr.shared.ui.announce
import dev.kampr.shared.wire.Attachment
import kotlinx.coroutines.launch

// How much of a file is laid out at once. The route will hand back 8 MiB, and every character of
// it is scanned for syntax and shaped into one text node — so the ceiling here is about what a
// phone can draw, not about what the node will serve, and the reader is told which they got.
private const val LINES_SHOWN = 2_000

// What a text file goes to be read in. The transcript is not it: a file is not a turn, and a
// thousand lines of one inside a reply is a transcript nobody can scroll past.
@Composable
fun FileViewer(
    att: Attachment,
    text: String,
    attachments: AttachmentStore,
    saved: String?,
    onSave: () -> Unit,
    onClose: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val tokens = Kampr.tokens
    val safe = LocalSafeArea.current
    val io = LocalPaneIo.current
    val scope = rememberCoroutineScope()
    val name = att.name ?: "file"
    val path = remember(att.id) { pathOfAttachmentId(att.id) }
    val lines = remember(text) { text.count { it == '\n' } + 1 }
    val shown = remember(text) {
        if (lines <= LINES_SHOWN) text else text.lineSequence().take(LINES_SHOWN).joinToString("\n")
    }
    var asked by remember(att.id) { mutableStateOf(false) }
    val diff = remember(path) { path?.let(::diffTarget) }
    val patch = diff?.let { attachments.state(it.id) }

    Column(modifier.fillMaxSize().background(tokens.color.bg)) {
        DisableSelection {
            Row(
                Modifier
                    .fillMaxWidth()
                    .background(tokens.color.bar)
                    .padding(
                        start = 16.dp + safe.left,
                        end = 16.dp + safe.right,
                        top = 10.dp + safe.top,
                        bottom = 10.dp,
                    ),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                Column(Modifier.weight(1f)) {
                    KText(name, tokens.type.meta, tokens.color.text, maxLines = 1)
                    KText(
                        path ?: "$lines lines",
                        tokens.type.micro,
                        tokens.color.mute,
                        maxLines = 1,
                    )
                }
                GlyphTarget(
                    ConversationIcons.close, "Close $name", tokens.color.mute,
                    onClose, target = LANDSCAPE_TOUCH, glyph = 14.dp,
                )
            }
        }

        SelectionContainer(Modifier.weight(1f)) {
            Column(
                Modifier
                    .fillMaxSize()
                    .verticalScroll(rememberScrollState())
                    .padding(16.dp),
                verticalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                CodeCard(langOf(name), shown, "")
                if (lines > LINES_SHOWN) {
                    KText(
                        "showing the first $LINES_SHOWN of $lines lines",
                        tokens.type.micro,
                        tokens.color.mute,
                        Modifier.announce("Showing the first $LINES_SHOWN of $lines lines"),
                    )
                }
                if (diff != null) {
                    when (patch) {
                        is AttachmentState.Text -> DiffCard(null, patch.text, "")
                        AttachmentState.Fetching -> KText(
                            "asking git",
                            tokens.type.micro,
                            tokens.color.working,
                            Modifier.announce("Asking git what has changed"),
                        )
                        // One uniform refusal covers a file with no changes, a file outside a work
                        // tree and a machine with no git at all, so this says all three rather
                        // than picking one and being wrong about the other two.
                        is AttachmentState.Failed -> KText(
                            "nothing changed since HEAD, or this file is not in a git work tree",
                            tokens.type.micro,
                            tokens.color.mute,
                            Modifier.announce("Nothing changed since HEAD, or this file is not in a git work tree"),
                        )
                        else -> DisableSelection {
                            QuietAction(
                                "Changes since HEAD",
                                {
                                    asked = true
                                    scope.launch { attachments.open(io, diff) }
                                },
                                Modifier.fillMaxWidth(),
                                label = "Show what has changed in $name since HEAD",
                                enabled = !asked,
                            )
                        }
                    }
                }
            }
        }

        DisableSelection {
            Box(
                Modifier
                    .fillMaxWidth()
                    .background(tokens.color.bar)
                    .padding(
                        start = 16.dp + safe.left,
                        end = 16.dp + safe.right,
                        top = 10.dp,
                        bottom = 12.dp + safe.bottom,
                    ),
            ) {
                if (saved == null) {
                    QuietAction(
                        "Save to device",
                        onSave,
                        Modifier.fillMaxWidth(),
                        label = "Save $name to this device",
                    )
                } else {
                    KText(
                        "saved to $saved",
                        tokens.type.micro,
                        tokens.color.done,
                        Modifier.fillMaxWidth().announce("Saved to $saved"),
                        maxLines = 2,
                    )
                }
            }
        }
    }
}
