package dev.kampr.conversation.files

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.pointer.PointerIcon
import androidx.compose.ui.platform.LocalClipboard
import androidx.compose.ui.semantics.SemanticsNode
import androidx.compose.ui.unit.dp
import dev.kampr.conversation.Decoder
import dev.kampr.conversation.Films
import dev.kampr.conversation.HeldClipboard
import dev.kampr.conversation.LocalFilms
import dev.kampr.conversation.LocalVoices
import dev.kampr.conversation.PORTRAIT
import dev.kampr.conversation.Projector
import dev.kampr.conversation.Voices
import dev.kampr.conversation.tokensFor
import dev.kampr.shared.net.AttachmentBytes
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.FilesNav
import dev.kampr.shared.ui.FilesPlace
import dev.kampr.shared.ui.FilesStack
import dev.kampr.shared.ui.LocalOpenPlace
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.ui.SafeArea
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.PanePrefs

internal val PHONE_BARS = SafeArea(top = 44.dp, bottom = 46.dp)

internal class Stack(vararg first: FilesPlace) : FilesNav {
    val places = mutableStateListOf(*first)
    override fun open(place: FilesPlace) {
        places += place
    }
    override fun replace(place: FilesPlace) {
        places[places.lastIndex] = place
    }
    override fun back() {
        places.removeAt(places.lastIndex)
    }
    override fun close() = places.clear()
}

internal class FileNode(private val answers: Map<String, AttachmentBytes>) : PaneIo {
    val asked = mutableListOf<Pair<String, String>>()
    override fun send(msg: ClientMsg) = Unit
    override fun prefs(paneId: String): PanePrefs = PanePrefs()
    override suspend fun attachment(paneId: String, id: String): AttachmentBytes {
        asked += paneId to id
        return answers[id] ?: AttachmentBytes.Failed("no such attachment")
    }
    override suspend fun streamUrl(paneId: String, id: String): String = streamOf(id)
}

internal fun streamOf(id: String) = "https://node.test/api/stream/$id"

internal fun text(body: String) = AttachmentBytes.Ok(body.encodeToByteArray(), "application/octet-stream")

// What `KamprApp` does with the stack, without the app around it: the top place drawn over the
// window through the app's own `FilesStack`, and every surface under it opening places onto it.
@Composable
internal fun Stage(io: PaneIo, nav: Stack, held: HeldBytes = HeldBytes(), voices: Voices = Decoder(), films: Films = Projector(), content: @Composable () -> Unit = {}) {
    CompositionLocalProvider(
        LocalVoices provides voices,
        LocalFilms provides films,
        LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
        LocalPaneIo provides io,
        LocalSafeArea provides PHONE_BARS,
        LocalHeldBytes provides held,
        LocalOpenPlace provides { nav.open(it) },
    ) {
        Box(Modifier.size(PORTRAIT.first, PORTRAIT.second)) {
            content()
            FilesStack(nav.places) { FilesHost(it, nav, Modifier.fillMaxSize()) }
        }
    }
}

// A copy of `dev.kampr.terminal.HoverChain`'s reader, for the reason given there: a module may not
// depend on another's test sources.
private val HOVER = Regex("""PointerHoverIconModifierElement\(icon=(.+), overrideDescendants=(\w+)\)""")

private fun SemanticsNode.hoverChain(): List<Pair<String, Boolean>> = layoutInfo.getModifierInfo()
    .mapNotNull { HOVER.find(it.modifier.toString()) }
    .map { it.groupValues[1] to it.groupValues[2].toBoolean() } +
    children.flatMap { it.hoverChain() }

internal fun SemanticsNode.cursor(): String? {
    val chain = hoverChain()
    return chain.firstOrNull { it.second }?.first ?: chain.lastOrNull()?.first
}

internal val TEXT_CURSOR = PointerIcon.Text.toString()

@Composable
internal fun WithClipboard(board: HeldClipboard, content: @Composable () -> Unit) {
    CompositionLocalProvider(LocalClipboard provides board, content = content)
}
