package dev.kampr.mosaic

import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.theme.Ground
import dev.kampr.shared.theme.PhosphorTheme
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.SafeArea
import dev.kampr.terminal.TerminalSurfaces
import java.io.File
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private val LIVE = ConnectionStatus.Live("full")

private data class Paired(val name: String, val size: Pair<Dp, Dp>, val landscape: Boolean, val focus: String)

class MosaicArtboardTest {
    @Test
    fun theDesktopMosaicRendersFourPanesFromThreeNodes() {
        val fixture = Fixture()
        fixture.fourPanes()
        val file = File(OUT, "desktop-mosaic.png")
        renderArtboard(DESKTOP.first, DESKTOP.second, SoftTheme, TypeScale.Desk, file) {
            MosaicScreen(
                store = fixture.store,
                mosaic = fixture.mosaic,
                herd = fixture.store.herd.value,
                connectionStatus = LIVE,
                build = "0.1.0",
                surfaces = TerminalSurfaces(),
                onHerd = {},
                onAdd = {},
            )
        }
        assertTrue(file.length() > 0, "the mosaic rendered nothing")
        assertEquals(4, fixture.mosaic.observers, "four cells is four observe streams and nothing else")
    }

    // A peer going down must degrade only its own cells; the other three keep painting.
    @Test
    fun aDeadPeerDegradesOnlyItsOwnCell() {
        val fixture = Fixture()
        fixture.fourPanes()
        fixture.store.accept(herdMessage(sunOnline = false))
        val herd = fixture.store.herd.value
        assertEquals(4, herd.panes.size, "a dropped peer keeps its panes listed")
        fixture.mosaic.reconcile(herd)
        assertEquals(4, fixture.mosaic.panes.size)

        val file = File(OUT, "desktop-mosaic-peer-down.png")
        renderArtboard(DESKTOP.first, DESKTOP.second, SoftTheme, TypeScale.Desk, file) {
            MosaicScreen(
                store = fixture.store,
                mosaic = fixture.mosaic,
                herd = herd,
                connectionStatus = LIVE,
                build = "0.1.0",
                surfaces = TerminalSurfaces(),
                onHerd = {},
                onAdd = {},
            )
        }
        assertTrue(file.length() > 0)
    }

    @Test
    fun theSwitcherRendersOnAPhoneInBothOrientations() {
        for ((name, size, landscape) in listOf(
            Triple("switcher-portrait", PORTRAIT, false),
            Triple("switcher-landscape", LANDSCAPE, true),
        )) {
            val fixture = Fixture()
            fixture.fourPanes()
            fixture.mosaic.focus(SUNGROW)
            val file = File(OUT, "$name.png")
            renderArtboard(size.first, size.second, SoftTheme, TypeScale.Phone, file) {
                MosaicSwitcher(
                    store = fixture.store,
                    mosaic = fixture.mosaic,
                    herd = fixture.store.herd.value,
                    surfaces = TerminalSurfaces(),
                    landscape = landscape,
                    onHerd = {},
                    onAdd = {},
                )
            }
            assertTrue(file.length() > 0, "$name rendered nothing")
        }
    }

    // With the system bars a real phone has, so the key row can be seen to stand off the handle and
    // the right-hand cell off a navigation bar on its side.
    @Test
    fun theSwitcherShowsTwoPanesInBothOrientations() {
        for ((name, size, landscape, focus) in listOf(
            Paired("switcher-pair-portrait", PORTRAIT, false, CODEX),
            Paired("switcher-pair-portrait-top", PORTRAIT, false, CLAUDE),
            Paired("switcher-pair-landscape", LANDSCAPE, true, CODEX),
        )) {
            val fixture = Fixture()
            fixture.fourPanes()
            fixture.mosaic.pair(true)
            fixture.mosaic.focus(focus)
            val file = File(OUT, "$name.png")
            val bars = if (landscape) SafeArea(top = 24.dp, bottom = 0.dp, right = 48.dp) else SafeArea(top = 32.dp, bottom = 24.dp)
            renderArtboard(size.first, size.second, SoftTheme, TypeScale.Phone, file) {
                CompositionLocalProvider(LocalSafeArea provides bars) {
                    MosaicSwitcher(
                        store = fixture.store,
                        mosaic = fixture.mosaic,
                        herd = fixture.store.herd.value,
                        surfaces = TerminalSurfaces(),
                        landscape = landscape,
                        onHerd = {},
                        onAdd = {},
                    )
                }
            }
            assertTrue(file.length() > 0, "$name rendered nothing")
        }
    }

    @Test
    fun thePickerGroupsTheHerdByNodeThenSession() {
        val fixture = Fixture()
        fixture.fourPanes()
        fixture.mosaic.remove(DOGE)
        val file = File(OUT, "mosaic-picker.png")
        renderArtboard(DESKTOP.first, DESKTOP.second, SoftTheme, TypeScale.Desk, file) {
            PanePicker(
                herd = fixture.store.herd.value,
                breakpoint = dev.kampr.shared.ui.Breakpoint.Desktop,
                chosen = fixture.mosaic.panes,
                full = fixture.mosaic.full,
                onPick = {},
                onDismiss = {},
            )
        }
        assertTrue(file.length() > 0)
    }

    // The token layer is only proved by a second theme and a second ground.
    @Test
    fun theMosaicSurvivesASecondThemeAndALightGround() {
        val fixture = Fixture()
        fixture.fourPanes()
        for ((name, spec, ground) in listOf(
            Triple("desktop-mosaic-phosphor", PhosphorTheme, Ground.Dark),
            Triple("desktop-mosaic-light", SoftTheme, Ground.Light),
        )) {
            val file = File(OUT, "$name.png")
            renderArtboard(DESKTOP.first, DESKTOP.second, spec, TypeScale.Desk, file, ground) {
                MosaicScreen(
                    store = fixture.store,
                    mosaic = fixture.mosaic,
                    herd = fixture.store.herd.value,
                    connectionStatus = LIVE,
                    build = "0.1.0",
                    surfaces = TerminalSurfaces(),
                    onHerd = {},
                    onAdd = {},
                )
            }
            assertTrue(file.length() > 0, "$name rendered nothing")
        }
    }
}
