package dev.kampr.app

import android.Manifest
import android.content.pm.PackageManager
import androidx.test.core.app.ActivityScenario
import androidx.test.platform.app.InstrumentationRegistry
import dev.kampr.shared.net.assetLinkComplaint
import dev.kampr.shared.net.createPasskeys
import dev.kampr.shared.net.decodeQrLuminance
import dev.kampr.shared.net.pairingScanAvailable
import dev.kampr.shared.util.joinLink
import dev.kampr.shared.util.qrEncode
import java.net.HttpURLConnection
import java.net.URL
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Test

// The two things a phone can answer and a host cannot: whether there is an authenticator to ask,
// and what this build is signed with. Both decide whether a passkey is offered at all.
class PasskeyAndScannerTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val node: String? = InstrumentationRegistry.getArguments().getString("kamprNode")

    @Test
    fun anAuthenticatorIsOnlyOfferedOnceThereIsSomethingToRaiseItFrom() {
        assertTrue(
            "with no Activity attached there is nothing for Credential Manager to draw on, so " +
                "the passkey control must be absent rather than present and failing",
            !createPasskeys().available,
        )
        ActivityScenario.launch(MainActivity::class.java).use {
            assertTrue("MainActivity attaches the host in onCreate", createPasskeys().available)
        }
    }

    // The fingerprint the node has to name in `[android] fingerprints`. A debug build is signed
    // with this machine's own debug keystore, so it is never the release one, and the only way an
    // operator can learn it is for the app to read it off itself.
    @Test
    fun theAppKnowsWhichCertificateItIsSignedWith() {
        ActivityScenario.launch(MainActivity::class.java).use {
            val identity = requireNotNull(createPasskeys().identity) { "no app identity on a device" }
            assertEquals(context.packageName, identity.packageName)
            assertTrue(
                "not a SHA-256 an assetlinks.json would accept: ${identity.fingerprint}",
                Regex("([0-9A-F]{2}:){31}[0-9A-F]{2}").matches(identity.fingerprint),
            )
        }
    }

    // With `-e kamprNode <url>`: whether *this* build could enrol against *that* node, which is
    // the question a mysterious refusal is otherwise hiding. A complaint is a legitimate answer —
    // a debug build against a stock node is exactly that — but it has to name what to paste.
    @Test
    fun aNodeThatWillNotTakeThisBuildSaysWhatToPasteIntoItsConfig() {
        assumeTrue("no -e kamprNode <url> given", node != null)
        val document = fetch("$node/.well-known/assetlinks.json")
        ActivityScenario.launch(MainActivity::class.java).use {
            val identity = requireNotNull(createPasskeys().identity) { "no app identity on a device" }
            val complaint = assetLinkComplaint(document, identity)
            // Two-sided, against the bytes the node served rather than against the same reasoning
            // twice: if this build is named in that file it must enrol, and if it is not the
            // refusal must carry the line to paste.
            val named = document != null &&
                identity.packageName in document &&
                identity.fingerprint in document.uppercase()
            if (named) {
                assertNull("$node names this build, so nothing should complain: $complaint", complaint)
            } else {
                assertTrue(
                    "the complaint has to carry the fingerprint to add: $complaint",
                    complaint != null &&
                        (identity.fingerprint in complaint || identity.packageName in complaint),
                )
            }
        }
    }

    @Test
    fun theAppAsksForTheCameraItScansWith() {
        assertTrue(pairingScanAvailable)
        val requested = context.packageManager
            .getPackageInfo(context.packageName, PackageManager.GET_PERMISSIONS)
            .requestedPermissions
            .orEmpty()
        assertTrue(
            "CAMERA is not declared, so the in-app scanner can never be granted and a scanned " +
                "pairing code can only ever reach the browser",
            Manifest.permission.CAMERA in requested,
        )
    }

    // The decoder on the real VM, against the encoder that draws the desktop's symbol.
    @Test
    fun theSymbolTheDesktopDrawsDecodesOnThisDevice() {
        val link = joinLink("http://192.168.1.24:8790", "2KQK-RB5Y")
        val code = requireNotNull(qrEncode(link)) { "nothing encoded for $link" }
        val scale = 5
        val quiet = 4
        val side = (code.size + quiet * 2) * scale
        // A camera plane, padded the way a real one is.
        val stride = side + 16
        val luma = ByteArray(stride * side) { 0xFF.toByte() }
        for (y in 0 until code.size) {
            for (x in 0 until code.size) {
                if (!code.dark(x, y)) continue
                for (dy in 0 until scale) {
                    for (dx in 0 until scale) {
                        luma[((y + quiet) * scale + dy) * stride + (x + quiet) * scale + dx] = 0
                    }
                }
            }
        }
        assertEquals(link, decodeQrLuminance(luma, stride, side, side))
    }

    private fun fetch(url: String): String? {
        val connection = (URL(url).openConnection() as HttpURLConnection).apply {
            connectTimeout = 10_000
            readTimeout = 10_000
        }
        return try {
            if (connection.responseCode == 200) {
                connection.inputStream.bufferedReader().use { it.readText() }
            } else {
                null
            }
        } finally {
            connection.disconnect()
        }
    }
}
