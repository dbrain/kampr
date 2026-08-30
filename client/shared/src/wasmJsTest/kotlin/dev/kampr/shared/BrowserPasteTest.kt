package dev.kampr.shared

import dev.kampr.shared.platform.pastedFile
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.async
import kotlinx.coroutines.await
import kotlinx.coroutines.test.runTest
import kotlin.js.ExperimentalWasmJsInterop
import kotlin.js.Promise
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNotNull
import kotlin.test.assertTrue
import kotlin.time.Duration.Companion.seconds

// A clipboard the harness owns. `DataTransfer` and `ClipboardEvent` are both constructible in
// Chromium, so a paste can be put in front of the listener without a real clipboard, a real
// permission or a real screenshot — which is the only way any of this is testable at all, because
// no headless browser has a clipboard to read.
//
// It is dispatched on `document.body` with `bubbles`, not on `window`, so what is measured is the
// listener seeing an event that started somewhere else on the page — which is every real paste,
// wherever the focus happens to be.
@OptIn(ExperimentalWasmJsInterop::class)
private fun pasteFiles(names: String, mime: String, body: String): Boolean = js(
    """
    (function () {
      var data = new DataTransfer();
      names.split(',').forEach(function (name) {
        data.items.add(new File([body + ':' + name], name, { type: mime }));
      });
      var event = new ClipboardEvent('paste', { clipboardData: data, bubbles: true, cancelable: true });
      document.body.dispatchEvent(event);
      return event.defaultPrevented;
    })()
    """
)

// Real milliseconds, not the virtual ones `runTest` runs on: what has to elapse here is a
// browser finishing a `FileReader`, and no test scheduler can advance that.
@OptIn(ExperimentalWasmJsInterop::class)
private fun jsSettle(ms: Int): Promise<JsString?> = js(
    "new Promise(function (resolve) { setTimeout(function () { resolve(null); }, ms); })"
)

private suspend fun settle(ms: Int) {
    jsSettle(ms).await<JsString?>()
}

@OptIn(ExperimentalWasmJsInterop::class)
private fun pasteText(text: String): Boolean = js(
    """
    (function () {
      var data = new DataTransfer();
      data.setData('text/plain', text);
      var event = new ClipboardEvent('paste', { clipboardData: data, bubbles: true, cancelable: true });
      document.body.dispatchEvent(event);
      return event.defaultPrevented;
    })()
    """
)

// There was no way to put a screenshot in front of an agent from a browser: the attach button
// raises a file picker, and a screenshot taken with the system's own tool is on the clipboard and
// nowhere else — so it had to be saved to disk first and then picked. These assert the seam that
// closes that, at the only level that can see it: a real Chromium, a real `paste` event, and the
// same `pastedFile()` the pane surfaces wait on.
class BrowserPasteTest {
    @Test
    fun aFileOnTheClipboardArrivesWithItsNameItsTypeAndItsBytes() = runTest {
        val waiting = async(start = CoroutineStart.UNDISPATCHED) { pastedFile() }
        assertTrue(
            pasteFiles("shot.png", "image/png", "one"),
            "a paste carrying a file was left for the text field to insert its name",
        )
        val file = assertNotNull(waiting.await(), "the paste never arrived")
        assertEquals("shot.png", file.name)
        assertEquals("image/png", file.mime)
        assertEquals("one:shot.png", file.bytes.decodeToString())
    }

    // The half that would be a regression nobody notices: a clipboard with words on it is Compose's
    // and must stay Compose's. Taking every paste would mean an operator could no longer paste a
    // command into the reply box, which is the thing this surface is most used for.
    @Test
    fun aClipboardWithNoFileOnItIsLeftAloneAndTheNextOneWithAFileIsNot() = runTest {
        val waiting = async(start = CoroutineStart.UNDISPATCHED) { pastedFile() }
        assertFalse(
            pasteText("cargo test --workspace"),
            "a paste of ordinary text was taken away from the reply box",
        )
        assertTrue(pasteFiles("later.txt", "text/plain", "two"))
        val file = assertNotNull(waiting.await(), "the file paste never arrived")
        assertEquals("later.txt", file.name)
    }

    // **Nobody is waiting for most of a paste's life.** Two files are two independent `FileReader`s
    // and the browser owes nothing about which finishes first; between them the caller is off
    // encoding the first one and sending it to the node, and after them the pane may have been
    // switched away from entirely. So the second read lands with no waiter parked, and it has to be
    // kept rather than delivered to nobody.
    //
    // The settle is what makes this true rather than lucky: without it the second read finishes
    // *after* the next `pastedFile()` has already parked a waiter, and the test passes with the
    // queue deleted — which it did, the first time it was written. Order is still not asserted,
    // because the browser does not owe it.
    @Test
    fun aFileThatArrivesWithNobodyWaitingIsKeptRatherThanDropped() = runTest(timeout = 15.seconds) {
        val waiting = async(start = CoroutineStart.UNDISPATCHED) { pastedFile() }
        assertTrue(pasteFiles("a.png,b.png", "image/png", "three"))
        val first = assertNotNull(waiting.await(), "the first of two never arrived")
        settle(200)
        val second = assertNotNull(pastedFile(), "the second of two was dropped")
        assertEquals(setOf("a.png", "b.png"), setOf(first.name, second.name))
        assertEquals("three:${second.name}", second.bytes.decodeToString())
    }
}
