package dev.kampr.mosaic

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.asSkiaBitmap
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.v2.runSkikoComposeUiTest
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Ground
import dev.kampr.shared.theme.LocalGround
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.theme.themeOf
import dev.kampr.shared.ui.Breakpoint
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.SafeArea
import org.jetbrains.skia.EncodedImageFormat
import org.jetbrains.skia.Image
import java.io.File
import kotlin.test.Test

private val GALLERY_OUT = File("build/artboards/gallery")

private enum class Frame(val width: Int, val height: Int, val density: Float, val scale: TypeScale, val bars: SafeArea, val breakpoint: Breakpoint) {
    Portrait(390, 844, 2.625f, TypeScale.Phone, SafeArea(top = 44.dp, bottom = 46.dp), Breakpoint.Portrait),
    Desktop(1280, 800, 1.5f, TypeScale.Desk, SafeArea(0.dp, 0.dp), Breakpoint.Desktop),
}

@OptIn(ExperimentalTestApi::class)
private fun shoot(name: String, frame: Frame, content: @Composable () -> Unit) {
    val file = File(GALLERY_OUT, "$name-${frame.name.lowercase()}.png")
    runSkikoComposeUiTest(Size(frame.width * frame.density, frame.height * frame.density), Density(frame.density)) {
        mainClock.autoAdvance = false
        setContent {
            CompositionLocalProvider(
                LocalTokens provides tokensFor(themeOf("soft"), frame.scale, Ground.Dark),
                LocalGround provides Ground.Dark,
                LocalSafeArea provides frame.bars,
                LocalPaneIo provides ArtboardIo,
            ) {
                Box(Modifier.fillMaxSize()) { content() }
            }
        }
        mainClock.advanceTimeBy(800)
        file.parentFile.mkdirs()
        file.writeBytes(requireNotNull(Image.makeFromBitmap(captureToImage().asSkiaBitmap()).encodeToData(EncodedImageFormat.PNG)).bytes)
    }
}

class ModalGalleryArtboardTest {
    @Test
    fun thePanePickerRenders() {
        val fixture = Fixture().apply { store.accept(herdMessage()) }
        val herd = fixture.store.herd.value
        for (frame in Frame.entries) {
            shoot("pane-picker", frame) {
                PanePicker(herd, frame.breakpoint, listOf(CLAUDE, CODEX), full = false, onPick = {}, onDismiss = {})
            }
            shoot("pane-picker-full", frame) {
                PanePicker(herd, frame.breakpoint, listOf(CLAUDE, CODEX, SUNGROW, DOGE), full = true, onPick = {}, onDismiss = {})
            }
        }
    }
}
