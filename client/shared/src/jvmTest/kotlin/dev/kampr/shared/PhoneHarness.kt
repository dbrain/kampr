package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.navigationevent.NavigationEventDispatcher
import androidx.navigationevent.NavigationEventDispatcherOwner
import androidx.navigationevent.NavigationEventInput
import dev.kampr.shared.theme.Ground
import dev.kampr.shared.theme.KamprFonts
import dev.kampr.shared.theme.KamprTokens
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.theme.on
import dev.kampr.shared.theme.themeOf
import dev.kampr.shared.theme.typography
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.ManageIo
import dev.kampr.shared.ui.PaneSurfaces
import dev.kampr.shared.ui.SafeArea
import dev.kampr.shared.wire.PaneInfo

// What a pixel_6 reports: a status bar with a punch-hole in it, and a gesture handle. 1080×2400
// at 480 dpi, which is the profile every one of these defects has been found on.
val BARS = SafeArea(top = 44.dp, bottom = 46.dp)

// Rotated with three-button navigation, which is the posture that moves the bar to a side and
// takes the other with a cutout. Zero under gestures, which is why the emulator hid this.
val SIDE_BARS = listOf(
    SafeArea(top = 24.dp, bottom = 0.dp, left = 48.dp, right = 0.dp),
    SafeArea(top = 24.dp, bottom = 0.dp, left = 0.dp, right = 48.dp),
)

fun phoneTokens(): KamprTokens = themeOf("soft").on(Ground.Dark).let { spec ->
    val fonts = KamprFonts(FontFamily.Default, FontFamily.Monospace, FontFamily.Monospace)
    KamprTokens(spec, fonts, typography(fonts, spec.label, TypeScale.Phone))
}

// Every layout assertion in this suite measures Compose's own semantics tree, which knows nothing
// about what SystemUI paints on top of it — so a full suite passed while the gesture handle sat on
// the word "Pane". `LocalSafeArea` exists so a test can put the bars back.
@Composable
fun Bars(bars: SafeArea = BARS, content: @Composable () -> Unit) {
    CompositionLocalProvider(LocalTokens provides phoneTokens(), LocalSafeArea provides bars, content = content)
}

// A pane screen without the three surfaces it hosts, for the tests that are about the chrome
// around them. The zoom control is a real 40 dp box because the header measures what it holds.
object BlankSurfaces : PaneSurfaces {
    @Composable override fun Terminal(pane: PaneState, info: PaneInfo?, modifier: Modifier) = Box(modifier)
    @Composable override fun Conversation(pane: PaneState, info: PaneInfo?, modifier: Modifier) = Box(modifier)
    @Composable override fun KeyRow(pane: PaneState, compact: Boolean, modifier: Modifier) = Box(modifier)
    @Composable override fun Zoom(pane: PaneState, modifier: Modifier) = Box(modifier.size(40.dp))
}

// `LocalManage` defaults to refusing, and a header without the + and the ... is not the header the
// report came off.
object AllowManage : ManageIo {
    override val enabled = true
    override fun openNew(paneId: String?) = Unit
    override fun openActions(paneId: String) = Unit
}

// The window's own back button, as Android delivers it: a tap with no predictive drag in front of
// it. `claimed` is the signal the platform itself acts on — with nothing in the composition
// claiming back, the gesture goes straight past the app and finishes the activity, which is the
// whole of the report this harness exists for.
private class BackButton : NavigationEventInput() {
    var claimed = false
        private set

    override fun onHasEnabledHandlersChanged(hasEnabledHandlers: Boolean) {
        claimed = hasEnabledHandlers
    }

    fun press() = dispatchOnBackCompleted()
}

class SystemBackWindow : NavigationEventDispatcherOwner {
    override val navigationEventDispatcher = NavigationEventDispatcher()
    private val button = BackButton().also { navigationEventDispatcher.addInput(it) }

    val claimed: Boolean get() = button.claimed

    fun press() = button.press()
}
