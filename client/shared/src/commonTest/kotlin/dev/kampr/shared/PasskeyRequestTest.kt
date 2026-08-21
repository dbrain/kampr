package dev.kampr.shared

import dev.kampr.shared.net.AppIdentity
import dev.kampr.shared.net.assetLinkComplaint
import dev.kampr.shared.net.credentialManagerRequest
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull
import kotlin.test.assertTrue

// Captured from a running node, not written here: `POST /auth/webauthn/register/start` with
// `{"device_name":"Pixel","platform":"android"}` against `kampr serve` at http://localhost:8793.
// `webauthn-rs` chose every field; this client's job is to hand Credential Manager the half of it
// that is the request, unaltered.
private const val REGISTER_START = """
{"challenge_id":"9c044ecb31675a26a97a60f853ea618c","options":{"publicKey":{"attestation":"none",
"authenticatorSelection":{"authenticatorAttachment":"platform","requireResidentKey":true,
"residentKey":"required","userVerification":"required"},
"challenge":"yK8d6nF-dDyy7dUuOunSLmdEbSVtHA7w10bWuw8ltdY","excludeCredentials":[],
"extensions":{"credProps":true,"uvm":true},"hints":["client-device"],
"pubKeyCredParams":[{"alg":-7,"type":"public-key"},{"alg":-257,"type":"public-key"}],
"rp":{"id":"localhost","name":"Kampr"},"timeout":300000,
"user":{"displayName":"Pixel","id":"CvLnrLs4SiKe6wnRmiKEsw","name":"Pixel"}}}}
"""

private const val ASSET_LINKS = """
[{"relation":["delegate_permission/common.get_login_creds"],"target":{"namespace":"android_app",
"package_name":"dev.kampr.app","sha256_cert_fingerprints":
["A0:8A:21:84:46:AA:2B:99:08:5C:67:0B:5A:9B:70:32:5E:05:F9:27:CC:DD:12:17:E7:94:63:13:C7:7F:C6:18"]}}]
"""

private val RELEASE = AppIdentity(
    "dev.kampr.app",
    "A0:8A:21:84:46:AA:2B:99:08:5C:67:0B:5A:9B:70:32:5E:05:F9:27:CC:DD:12:17:E7:94:63:13:C7:7F:C6:18",
)

class PasskeyRequestTest {
    // Credential Manager takes the *contents* of `publicKey`, where a browser takes the wrapper.
    // Handing it the wrapper is a request with no challenge in it.
    @Test
    fun credentialManagerIsHandedTheRequestAndNotTheEnvelope() {
        val options = requireNotNull(elementOf(REGISTER_START, "options"))
        val request = requireNotNull(credentialManagerRequest(options))
        assertTrue("\"challenge\":\"yK8d6nF-dDyy7dUuOunSLmdEbSVtHA7w10bWuw8ltdY\"" in request, request)
        assertTrue("\"publicKey\"" !in request, "the envelope is not the request: $request")
        for (required in listOf("rp", "user", "pubKeyCredParams", "authenticatorSelection")) {
            assertTrue("\"$required\"" in request, "$required is missing: $request")
        }
    }

    @Test
    fun somethingThatIsNotAChallengeIsNoRequest() {
        assertNull(credentialManagerRequest("""{"publicKey":{}}"""))
        assertNull(credentialManagerRequest("""{"challenge":"abc"}"""))
        assertNull(credentialManagerRequest("not json"))
        assertNull(credentialManagerRequest(""))
    }

    // The one failure that is otherwise a shrug. A node that does not name *this* build of the app
    // refuses every ceremony, and the only place the app's own fingerprint can be read is the app.
    @Test
    fun aNodeThatDoesNotNameThisBuildSaysWhichBuildItNames() {
        assertNull(assetLinkComplaint(ASSET_LINKS, RELEASE), "the shipped app is named: no complaint")
        assertNull(assetLinkComplaint(ASSET_LINKS, null), "off Android there is nothing to complain about")

        val debug = AppIdentity("dev.kampr.app", "AB".repeat(32).chunked(2).joinToString(":"))
        val complaint = requireNotNull(assetLinkComplaint(ASSET_LINKS, debug))
        assertTrue(debug.fingerprint in complaint, complaint)
        assertTrue("fingerprints" in complaint, complaint)

        val other = AppIdentity("com.example.other", RELEASE.fingerprint)
        assertTrue("com.example.other" in requireNotNull(assetLinkComplaint(ASSET_LINKS, other)))

        val missing = requireNotNull(assetLinkComplaint(null, RELEASE))
        assertTrue("assetlinks.json" in missing, missing)
    }

    // A node whose file lists this app under some *other* relation delegates nothing a passkey
    // can use, and that reads exactly like a file that is simply absent.
    @Test
    fun onlyTheLoginRelationCounts() {
        val urlsOnly = ASSET_LINKS.replace("common.get_login_creds", "common.handle_all_urls")
        assertTrue(assetLinkComplaint(urlsOnly, RELEASE) != null)
    }
}

private fun elementOf(json: String, key: String): String? =
    kotlinx.serialization.json.Json.parseToJsonElement(json)
        .let { it as? kotlinx.serialization.json.JsonObject }
        ?.get(key)
        ?.toString()
