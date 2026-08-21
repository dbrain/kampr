package dev.kampr.shared

import dev.kampr.shared.net.defaultEndpoint
import dev.kampr.shared.net.endpointFrom
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull
import kotlin.test.assertTrue

class ConnectTest {
    // 10.0.2.2 is the emulator's alias for its own host. On a real phone it resolves to nothing,
    // so the app opened, failed, and blamed the user for a guess it had made itself.
    @Test
    fun noPlatformGuessesAnAddressThatCannotExist() {
        val guess = defaultEndpoint()
        assertTrue(
            guess == null || "10.0.2.2" !in guess.baseUrl,
            "defaultEndpoint() guessed ${guess?.baseUrl}",
        )
    }

    // What somebody reads off the node's own output is a host and a port. Making them type the
    // scheme is the difference between one attempt and three.
    @Test
    fun aBareHostGetsTheSchemeItsShapeImplies() {
        val cases = listOf(
            "192.168.1.24:8790" to "http://192.168.1.24:8790",
            "192.168.1.24" to "http://192.168.1.24",
            "  192.168.1.24:8790  " to "http://192.168.1.24:8790",
            "localhost:8790" to "http://localhost:8790",
            "comingclean:8790" to "http://comingclean:8790",
            "nas.local:8790" to "http://nas.local:8790",
            "[fd00::1]:8790" to "http://[fd00::1]:8790",
            // A dotted name that is not .local is a name somebody can get a certificate for, and
            // the deployment this is written for is one: a public hostname behind a proxy.
            "kampr.example.com" to "https://kampr.example.com",
            "kampr.example.com:8443" to "https://kampr.example.com:8443",
            "http://kampr.example.com" to "http://kampr.example.com",
            "https://192.168.1.24:8790" to "https://192.168.1.24:8790",
            "http://192.168.1.24:8790/" to "http://192.168.1.24:8790",
        )
        for ((typed, expected) in cases) {
            assertEquals(expected, endpointFrom(typed)?.baseUrl, "typed: $typed")
        }
    }

    @Test
    fun nothingTypedIsNotAnAddress() {
        assertNull(endpointFrom(""))
        assertNull(endpointFrom("   "))
        assertNull(endpointFrom("http://"))
    }

    @Test
    fun aPairingCodeRidesAlongOnlyWhenThereIsOne() {
        assertEquals("ABC123", endpointFrom("192.168.1.24:8790", "ABC123")?.token)
        assertNull(endpointFrom("192.168.1.24:8790", "  ")?.token)
        assertNull(endpointFrom("192.168.1.24:8790", null)?.token)
    }
}
