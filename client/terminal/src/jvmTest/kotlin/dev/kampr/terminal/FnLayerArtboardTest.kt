package dev.kampr.terminal

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.terminal.input.InputSink
import dev.kampr.terminal.input.Latch
import dev.kampr.terminal.input.PaneKeyRow
import java.io.File
import kotlin.test.Test

private object HushKeyIo : PaneIo {
    override fun send(msg: ClientMsg) = Unit
    override fun prefs(paneId: String) = PanePrefs()
}

// The row with the layer up, at the width of the phone that reported it. The layer is what an
// operator sees for as long as they are pressing function keys, and it is a row taller than the
// one under it — so it is drawn here rather than described, on both orientations.
class FnLayerArtboardTest {
    @Test
    fun theFnLayerIsDrawnAtPhoneWidth() {
        for ((name, width, compact) in listOf(
            Triple("portrait", 411.dp, false),
            Triple("landscape", 914.dp, true),
        )) {
            renderArtboard(
                width = width,
                height = 200.dp,
                spec = SoftTheme,
                scale = TypeScale.Phone,
                file = File(OUT, "fn-layer-$name.png"),
            ) {
                val session = PaneSession("n/w1:p1")
                session.latches.lock(Latch.Fn)
                Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.BottomStart) {
                    PaneKeyRow(
                        session,
                        InputSink("n/w1:p1", HushKeyIo, session.latches),
                        compact = compact,
                        enabled = true,
                        modifier = Modifier.fillMaxWidth(),
                    )
                }
            }
        }
    }
}
