package dev.kampr.shared

import dev.kampr.shared.net.Endpoint
import dev.kampr.shared.net.pairingFrom
import dev.kampr.shared.util.joinLink
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

// A camera reads whatever is in front of it. What comes back is a string somebody else wrote, so
// everything here is about refusing text that is not one of ours as firmly as it accepts one.
class PairingScanTest {
    @Test
    fun theLinkTheDesktopPrintsIsAnAddressAndACode() {
        assertEquals(
            Endpoint("http://192.168.1.24:8790", "K7QF2M"),
            pairingFrom(joinLink("http://192.168.1.24:8790", "K7QF2M")),
        )
        assertEquals(
            Endpoint("https://kampr.example.com", "K7QF2M"),
            pairingFrom(joinLink("https://kampr.example.com", "K7QF2M")),
        )
    }

    // The same QR without a pairing code — what the setup screen shows before one is asked for.
    // It is still an address worth having: this device may already hold a token for it.
    @Test
    fun anOriginWithNoCodeIsStillAnAddress() {
        assertEquals(
            Endpoint("http://192.168.1.24:8790"),
            pairingFrom(joinLink("http://192.168.1.24:8790", null)),
        )
    }

    // What the node actually prints is `XXXX-XXXX` — eight characters of its confusable-free
    // alphabet, grouped. The six-character form above is what the golden QR test carries.
    @Test
    fun theCodeTheCliPrintsSurvivesItsDash() {
        assertEquals(
            Endpoint("http://192.168.1.24:8790", "2KQK-RB5Y"),
            pairingFrom("http://192.168.1.24:8790#pair=2KQK-RB5Y"),
        )
    }

    @Test
    fun aBareHostAndPortIsCompletedTheSameWayTypingItIs() {
        assertEquals(
            Endpoint("http://192.168.1.24:8790", "2KQK-RB5Y"),
            pairingFrom("192.168.1.24:8790#pair=2KQK-RB5Y"),
        )
        assertEquals(Endpoint("https://kampr.example.com"), pairingFrom("kampr.example.com"))
    }

    @Test
    fun aTrailingSlashAndSurroundingSpaceSurviveTheCamera() {
        assertEquals(
            Endpoint("http://192.168.1.24:8790", "K7QF2M"),
            pairingFrom("  http://192.168.1.24:8790/#pair=K7QF2M \n"),
        )
    }

    // Everything else a camera will decode if it is pointed at a noticeboard.
    @Test
    fun aQrThatIsNotAKamprLinkIsRefused() {
        for (other in listOf(
            "",
            "   ",
            "WIFI:S:home;T:WPA;P:hunter2;;",
            "otpauth://totp/Example:alice?secret=JBSWY3DPEHPK3PXP",
            "tel:+441234567890",
            "mailto:someone@example.com",
            "just some text",
            "http://192.168.1.24:8790/some/path",
            "ftp://192.168.1.24:8790",
            "javascript:alert(1)",
            "http://",
            "http://192.168.1.24:8790#pair=" + "x".repeat(200),
        )) {
            assertNull(pairingFrom(other), "«$other» is not a node")
        }
    }
}
