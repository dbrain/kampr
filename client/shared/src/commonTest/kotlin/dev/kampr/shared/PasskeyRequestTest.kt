package dev.kampr.shared

import dev.kampr.shared.net.AppIdentity
import dev.kampr.shared.net.Endpoint
import dev.kampr.shared.net.assetLinkComplaint
import dev.kampr.shared.net.assetLinksUrl
import dev.kampr.shared.net.credentialManagerRequest
import dev.kampr.shared.net.Enrolment
import dev.kampr.shared.net.PasskeyOutcome
import dev.kampr.shared.net.passkeyRefusal
import dev.kampr.shared.net.relyingParty
import dev.kampr.shared.ui.passkeyNoteOf
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

// The same route's answer, captured the same way: `POST /auth/webauthn/authenticate/start`.
// `webauthn-rs` spells the RP ID `rpId` here and `rp.id` in the registration above, and both are
// the one field that says which host Google will be asked about.
private const val AUTHENTICATE_START = """
{"challenge_id":"3f1a90c2f0d84e1b8e6c2a55b7d31c04","options":{"publicKey":{
"allowCredentials":[],"challenge":"Yk5wS0dRb0hzZXJfdGVzdF9jaGFsbGVuZ2VfMDE",
"rpId":"example.net","timeout":300000,"userVerification":"required"}}}
"""

private const val ASSET_LINKS = """
[{"relation":["delegate_permission/common.get_login_creds"],"target":{"namespace":"android_app",
"package_name":"dev.kampr.app","sha256_cert_fingerprints":
["A0:8A:21:84:46:AA:2B:99:08:5C:67:0B:5A:9B:70:32:5E:05:F9:27:CC:DD:12:17:E7:94:63:13:C7:7F:C6:18"]}}]
"""

// What Credential Manager actually says when Google's validator cannot fetch the file.
private const val RAW = "RP ID cannot be validated"

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

    // The node's file names this exact build and the ceremony failed anyway. Answering "nothing
    // wrong here" to somebody whose passkey just failed is a diagnosis that is confidently wrong,
    // and so is naming only one of the two causes that are left. Both are outside the phone: the
    // app's own half of the association, compiled into its manifest (#288), and Google's reach to
    // the node, which probe #170 measured and which only `kampr doctor` can answer.
    @Test
    fun aFileThatIsRightIsNotReportedAsAShrug() {
        val told = passkeyRefusal(ASSET_LINKS, RELEASE, "kampr.example.net", RAW)
        assertTrue("kampr.example.net" in told, told)
        assertTrue(RAW in told, "the authenticator's own words are the one thing to search for: $told")
        assertTrue("public internet" in told, told)
        assertTrue("doctor" in told, told)
        assertTrue(
            "manifest" in told,
            "naming only Google's reach sends the operator to a check that will say ok: $told",
        )
    }

    // The order the three answers are tried in. A file that is wrong is still the likelier cause
    // and still the one the phone can prove, so it keeps precedence over the one it cannot.
    @Test
    fun theFileIsBlamedOnlyWhenTheFileIsWrong() {
        val debug = AppIdentity("dev.kampr.app", "AB".repeat(32).chunked(2).joinToString(":"))
        assertTrue("fingerprints" in passkeyRefusal(ASSET_LINKS, debug, "node", RAW))
        assertTrue("assetlinks.json" in passkeyRefusal(null, RELEASE, "node", RAW))
        assertEquals(RAW, passkeyRefusal(ASSET_LINKS, null, "node", RAW), "a browser has no asset links")
    }

    // One field held the success and the refusal, one strip painted both, and the operator got a
    // green strip telling them a ceremony had failed. Both directions, or a rule that answered
    // "not a refusal" to everything would pass.
    @Test
    fun aRefusalIsNotToldInTheVoiceOfASuccess() {
        val refused = requireNotNull(passkeyNoteOf(PasskeyOutcome.Refused(RAW)))
        assertEquals(RAW, refused.message)
        assertTrue(refused.refused, "a refusal painted as a success is the worst version of this")

        val enrolled = requireNotNull(passkeyNoteOf(PasskeyOutcome.Enrolled(Enrolment("t", null, null, null))))
        assertTrue(!enrolled.refused, "an enrolment is not a failure")

        assertNull(passkeyNoteOf(PasskeyOutcome.Cancelled), "backing out of the sheet says nothing")
    }

    // Probe #170's sequel. The operator is moving the RP ID up to the registrable domain while the
    // node stays where it is, and from that moment `endpoint.host` is a hostname that decides
    // nothing: Google reads the RP ID's own well-known location and no other. A refusal that names
    // the host the client dials sends somebody to inspect a file nobody reads.
    @Test
    fun theHostBlamedIsTheOneTheCeremonyNamedAndNotTheOneTheClientDialled() {
        val register = requireNotNull(elementOf(REGISTER_START, "options"))
        assertEquals("localhost", relyingParty(register))

        val authenticate = requireNotNull(elementOf(AUTHENTICATE_START, "options"))
        assertEquals("example.net", relyingParty(authenticate))

        assertNull(relyingParty("""{"publicKey":{}}"""), "no rp id stated is no host to blame")
        assertNull(relyingParty("not json"))

        val told = passkeyRefusal(ASSET_LINKS, RELEASE, "example.net", RAW)
        assertTrue("https://example.net/.well-known/assetlinks.json" in told, told)
    }

    // And the file is read from the same place, or the client proves the wrong host correct and
    // then complains about it.
    @Test
    fun theFileIsReadFromTheHostThatDecidesRatherThanTheOneBeingDialled() {
        val node = Endpoint("https://kampr.example.net")
        assertEquals(
            "https://example.net/.well-known/assetlinks.json",
            assetLinksUrl(node, "example.net"),
        )
        assertEquals(
            "https://kampr.example.net/.well-known/assetlinks.json",
            assetLinksUrl(node, "kampr.example.net"),
        )
        // A dev node is plain http on a port of its own and has nothing at all on 443, so when the
        // RP ID *is* the host being dialled its own scheme and port are the right ones.
        val dev = Endpoint("http://localhost:8793")
        assertEquals("http://localhost:8793/.well-known/assetlinks.json", assetLinksUrl(dev, "localhost"))
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
