package dev.kampr.shared.net

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonPrimitive

const val ASSET_LINKS_PATH = "/.well-known/assetlinks.json"

// The relation Credential Manager reads. A file that delegates only `common.handle_all_urls` is an
// app-links file and buys a passkey nothing at all.
private const val LOGIN_CREDS = "delegate_permission/common.get_login_creds"

private val json = Json { ignoreUnknownKeys = true; isLenient = true }

// Why Android refused, when the reason is the one thing the phone can see and the node cannot.
//
// A node names the app it will accept a passkey from, by package and by signing certificate. The
// shipped release APK is named by default, so this is silent for anyone who installed Kampr. A
// debug build carries the machine's own debug keystore and a build from source carries whoever
// built it, and for those the node has to be told — which is impossible to guess at and trivial to
// paste, provided something says which fingerprint to paste.
//
// `null` means the file is fine and the failure was something else. Nothing here is a security
// check: it explains a refusal that has already happened.
fun assetLinkComplaint(document: String?, identity: AppIdentity?): String? {
    if (identity == null) return null
    val statements = document?.let { runCatching { json.parseToJsonElement(it).jsonArray }.getOrNull() }
        ?: return "This node publishes no $ASSET_LINKS_PATH, so Android will not create a passkey " +
            "for it. Add this to its config.toml:\n\n[android]\npackage = \"${identity.packageName}\"\n" +
            "fingerprints = [\"${identity.fingerprint}\"]"
    val targets = statements.filterIsInstance<JsonObject>()
        .filter { statement ->
            (statement["relation"] as? JsonArray)
                ?.any { it.jsonPrimitive.content == LOGIN_CREDS } == true
        }
        .mapNotNull { it["target"] as? JsonObject }
        .filter { it["namespace"]?.jsonPrimitive?.content == "android_app" }
    if (targets.isEmpty()) {
        return "This node's $ASSET_LINKS_PATH delegates nothing to an Android app, so Android " +
            "will not create a passkey for it. Its config.toml needs [android] fingerprints."
    }
    val ours = targets.filter { it["package_name"]?.jsonPrimitive?.content == identity.packageName }
    if (ours.isEmpty()) {
        val named = targets.mapNotNull { it["package_name"]?.jsonPrimitive?.content }.joinToString(", ")
        return "This node delegates passkeys to $named, and this app is ${identity.packageName}."
    }
    val fingerprints = ours.flatMap { target ->
        (target["sha256_cert_fingerprints"] as? JsonArray).orEmpty().map { it.jsonPrimitive.content.uppercase() }
    }
    if (fingerprints.none { it == identity.fingerprint.uppercase() }) {
        return "This node names ${identity.packageName} but not the certificate this build is " +
            "signed with. Add it to [android] fingerprints in its config.toml:\n\n" +
            "\"${identity.fingerprint}\""
    }
    return null
}
