package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.ImageComposeScene
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.input.pointer.PointerEventType
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.theme.Ground
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.theme.themeOf
import dev.kampr.shared.ui.HINT_HOVER_MS
import dev.kampr.shared.ui.LocalManage
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.PaneScreenMobile
import dev.kampr.shared.ui.PaneView
import dev.kampr.shared.wire.PaneInfo
import java.io.File
import kotlin.test.Test
import kotlin.test.assertTrue

private val INFO = PaneInfo(
    id = "01JNODE/w1:p1",
    nodeId = "01JNODE",
    workspace = "kampr",
    cwd = "/home/dbrain/dev/kampr",
    agent = "claude",
    agentStatus = "working",
    cols = 94,
    rows = 40,
    hasConversation = true,
)

// A mouse resting on the pane header's last glyph, which is where clamping to the window shows.
class HintArtboardTest {
    private fun board(name: String, ground: Ground, at: Offset) = onSceneThread {
        val density = Density(2f)
        val tokens = tokensFor(themeOf("soft"), TypeScale.Phone, ground)
        val scene = ImageComposeScene(720, 360, density) {
            CompositionLocalProvider(LocalTokens provides tokens, LocalSafeArea provides BARS, LocalManage provides AllowManage) {
                Box(Modifier.fillMaxSize()) {
                    PaneScreenMobile(
                        pane = PaneState(INFO.id, StyleTable()),
                        info = INFO,
                        view = PaneView.Terminal,
                        surfaces = BlankSurfaces,
                        landscape = false,
                        readOnly = false,
                        onBack = {},
                        onView = {},
                        modifier = Modifier.fillMaxSize(),
                    )
                }
            }
        }
        try {
            scene.render()
            val px = Offset(at.x * density.density, at.y * density.density)
            scene.sendPointerEvent(PointerEventType.Enter, px)
            scene.sendPointerEvent(PointerEventType.Move, px)
            Thread.sleep(HINT_HOVER_MS + 300)
            scene.render()
            val image = scene.render()
            val file = File("build/artboards/$name.png")
            file.parentFile.mkdirs()
            file.writeBytes(requireNotNull(image.encodeToData()).bytes)
            assertTrue(image.width > 0)
        } finally {
            scene.close()
        }
    }

    @Test
    fun aHintOverThePaneHeaderRendersOnBothGrounds() {
        for (ground in Ground.entries) {
            val tag = ground.name.lowercase()
            board("hint-pane-actions-$tag", ground, Offset(340f, 66f))
            board("hint-back-$tag", ground, Offset(22f, 66f))
        }
    }
}
