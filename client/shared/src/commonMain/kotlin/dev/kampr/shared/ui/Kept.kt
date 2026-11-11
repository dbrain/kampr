package dev.kampr.shared.ui

import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.ProvidableCompositionLocal
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.SaveableStateHolder
import androidx.compose.runtime.saveable.rememberSaveableStateHolder
import androidx.compose.runtime.staticCompositionLocalOf

// What a place had in hand that is not worth a `Bundle`: the bytes it was showing, so a place
// that is uncovered draws them at once rather than a skeleton while it asks again.
class PlaceMemory internal constructor() {
    private val values = mutableMapOf<String, Any>()

    @Suppress("UNCHECKED_CAST")
    fun <T : Any> recall(key: String): T? = values[key] as? T

    fun keep(key: String, value: Any) {
        values[key] = value
    }
}

val LocalPlaceMemory: ProvidableCompositionLocal<PlaceMemory?> = staticCompositionLocalOf { null }

// Only the top of a stack is ever composed, so whatever a place below it holds — a filter, a
// scroll, an open section — would be gone by the time back uncovers it. Each place's saveable
// state is held under its key instead, for as long as the key is still somewhere on the stack: a
// place that has been left for good starts fresh when it is opened again.
class Kept internal constructor(private val holder: SaveableStateHolder) {
    private val memories = mutableMapOf<String, PlaceMemory>()

    // A null key composes with no slot at all, so nothing it holds outlives it.
    @Composable
    fun Keep(key: String?, content: @Composable () -> Unit) {
        if (key == null) return content()
        val memory = memories.getOrPut(key) { PlaceMemory() }
        holder.SaveableStateProvider(key) {
            CompositionLocalProvider(LocalPlaceMemory provides memory, content = content)
        }
    }

    internal fun keepOnly(keys: Set<String>) {
        (memories.keys - keys).forEach {
            memories.remove(it)
            holder.removeState(it)
        }
    }
}

// After the frame, so a place popped this frame has already been disposed and saved, and the
// removal is what wins.
@Composable
fun rememberKept(stack: List<String>): Kept {
    val holder = rememberSaveableStateHolder()
    val kept = remember(holder) { Kept(holder) }
    SideEffect { kept.keepOnly(stack.toSet()) }
    return kept
}
