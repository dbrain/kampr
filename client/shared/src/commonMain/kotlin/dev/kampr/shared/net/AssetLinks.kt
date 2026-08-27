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

// Why the ceremony failed, in the order the phone can actually prove things.
//
// A file that is wrong is the cause the phone can read off the node, so it keeps precedence. What
// is left over are two causes the app cannot see, and both are named because neither can be ruled
// out from here (#288).
//
// The first is the app's own half of the association: Credential Manager checks the agreement in
// both directions, and the app's direction is a manifest entry fixed when its APK was compiled. A
// build reaching this line declares *some* site — the control is hidden otherwise — but not
// necessarily this one, and nothing at runtime can change which.
//
// The second is the one probe #170 measured: the node's file is right and Google, which fetches it
// server-side from the public internet, cannot reach the host. The client does not test that and
// must not — asking Google whether Google can see the operator's node is the app phoning a third
// party about the operator's network. `kampr doctor` answers that half and only that half.
//
// The authenticator's own words are kept either way, so there is something to search for.
fun passkeyRefusal(document: String?, identity: AppIdentity?, host: String, reason: String): String {
    if (identity == null) return reason
    assetLinkComplaint(document, identity)?.let { return it }
    return "This node's own setup is right: it names this app and this build's certificate. Two " +
        "things it cannot see decide the rest. This build lists the sites it may hold passkeys " +
        "for in its own manifest, fixed when it was compiled, and $host has to be one of them. " +
        "And Android never reads the node's file itself — Google's Digital Asset Links service " +
        "fetches https://$host$ASSET_LINKS_PATH over the public internet, so a node that only " +
        "resolves on your own network is one it cannot reach. `kampr doctor` there answers the " +
        "second of those and not the first.\n\nAndroid said: $reason"
}
