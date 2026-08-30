package dev.kampr.shared.platform

import java.awt.GraphicsEnvironment
import java.awt.Toolkit
import java.awt.datatransfer.DataFlavor

// AWT throws for both of the ways there is nothing to paste — `UnsupportedFlavorException` when the
// clipboard holds something that is not text, `IllegalStateException` when another process has it
// open — and both of those are this function's null. A headless JVM has no clipboard at all.
actual suspend fun clipboardText(): String? {
    if (GraphicsEnvironment.isHeadless()) return null
    val text = runCatching {
        Toolkit.getDefaultToolkit().systemClipboard.getData(DataFlavor.stringFlavor) as? String
    }.getOrNull()
    return text?.takeIf { it.isNotEmpty() }
}
