package dev.kampr.app

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.net.Uri
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performSemanticsAction
import androidx.compose.ui.test.runComposeUiTest
import androidx.test.platform.app.InstrumentationRegistry
import dev.kampr.conversation.Composer
import dev.kampr.shared.platform.pastedFile
import dev.kampr.shared.theme.KamprTheme
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeoutOrNull
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Test

private const val REPLY = "Reply to claude"

// **The whole of what a host test cannot answer.** Every route a pasted image takes on Android
// ends at a text field — there is no page-wide paste event the way a browser has one — and the
// field it ends at has to be one built on a `TextFieldState`, because the older `BasicTextField`
// declares no content types and refuses `commitContent` outright (#368). Both of those are
// readings taken off the compiled framework; this is the one that puts a real clipboard, a real
// paste and a real `contentReceiver` together on a real device and watches what arrives.
//
// The clipboard carries a URI rather than a bitmap, because that is what a screenshot tool, a
// gallery and Gboard's own clipboard tray all put there. An `android.resource://` URI belonging to
// the test package is used rather than a `FileProvider` one: it needs no provider declared in the
// app under test, and it is read through the same `ContentResolver` call a `content://` URI is.
@OptIn(ExperimentalTestApi::class)
class PastingAFileTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val app: Context = instrumentation.targetContext
    private val picture: Uri =
        Uri.parse("android.resource://${instrumentation.context.packageName}/raw/clipped")

    private fun putThePictureOnTheClipboard() {
        instrumentation.runOnMainSync {
            val clipboard = app.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
            clipboard.setPrimaryClip(ClipData.newUri(app.contentResolver, "screenshot", picture))
        }
    }

    @Test
    fun anImageOnTheClipboardPastedIntoTheReplyBoxArrivesAsBytesRatherThanAsALineOfUrl() =
        runComposeUiTest {
            val expected = app.contentResolver.openInputStream(picture)!!.use { it.readBytes() }
            putThePictureOnTheClipboard()
            var sent: String? = null
            setContent {
                KamprTheme(SoftTheme, TypeScale.Phone) {
                    Box(Modifier.fillMaxSize()) {
                        Composer("claude", enabled = true, onSend = { sent = it })
                    }
                }
            }
            onNodeWithContentDescription(REPLY).performSemanticsAction(SemanticsActions.PasteText)
            waitForIdle()

            val arrived = runBlocking { withTimeoutOrNull(10_000) { pastedFile() } }
            assertNotNull("the paste never reached the receiver", arrived)
            assertArrayEquals("the bytes are not the bytes on the clipboard", expected, arrived!!.bytes)
            // **Measured, and it costs nothing.** `ContentResolver.getType` answers null for an
            // `android.resource://` URI where it answers `image/png` for a gallery's `content://`
            // one — but `ClientMsg.Paste` carries no media type at all and the node derives the
            // extension by sniffing the body (`paste::write`), so neither the type nor the display
            // name is on the path a pasted picture takes. Asserted rather than skipped, so a
            // platform that starts answering sends somebody back to this row.
            assertEquals(null, arrived.mime)
            assertEquals(null, arrived.name)

            // The other half, and the one that would be the silent defect: the receiver consumed
            // the item, so nothing of the paste was left for the text field to insert. A URL typed
            // into somebody's reply is what not consuming it looks like.
            assertEquals("", editableOf(REPLY))
            assertEquals(null, sent)
        }

    private fun ComposeUiTest.editableOf(label: String): String =
        onNodeWithContentDescription(label).fetchSemanticsNode()
            .config.getOrElse(SemanticsProperties.EditableText) { AnnotatedString("") }.text
}
