package dev.kampr.conversation

import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue

// Held by the surfaces rather than the view, because the search is opened from the pane's actions
// sheet, which is composed at the app root a long way from the transcript it searches.
@Stable
class TranscriptSearch {
    var open by mutableStateOf(false)
    internal var onScreen by mutableIntStateOf(0)
}

internal class TranscriptSearches {
    private val byPane = HashMap<String, TranscriptSearch>()

    operator fun get(paneId: String): TranscriptSearch = byPane.getOrPut(paneId) { TranscriptSearch() }
}
