package dev.kampr.shared.net

import kotlinx.coroutines.await
import kotlin.js.ExperimentalWasmJsInterop
import kotlin.js.JsString
import kotlin.js.Promise

// The whole base64url⇄ArrayBuffer dance lives in JS because that is the only place the browser's
// own types exist: `webauthn-rs` serialises every buffer as base64url and `navigator.credentials`
// takes and returns `ArrayBuffer`, so something has to translate and it cannot be Kotlin.
@OptIn(ExperimentalWasmJsInterop::class)
private fun jsAvailable(): Boolean = js(
    """
    (function () {
      return !!(self.isSecureContext && window.PublicKeyCredential && navigator.credentials &&
                navigator.credentials.create && navigator.credentials.get);
    })()
    """
)

@OptIn(ExperimentalWasmJsInterop::class)
private fun jsCreate(optionsJson: String): Promise<JsString?> = js(
    """
    (async function () {
      var b64u = function (buffer) {
        var bytes = new Uint8Array(buffer), s = '';
        for (var i = 0; i < bytes.length; i++) s += String.fromCharCode(bytes[i]);
        return btoa(s).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+${'$'}/, '');
      };
      var buf = function (value) {
        var s = atob(String(value).replace(/-/g, '+').replace(/_/g, '/'));
        var bytes = new Uint8Array(s.length);
        for (var i = 0; i < s.length; i++) bytes[i] = s.charCodeAt(i);
        return bytes.buffer;
      };
      var options = JSON.parse(optionsJson).publicKey;
      options.challenge = buf(options.challenge);
      options.user.id = buf(options.user.id);
      (options.excludeCredentials || []).forEach(function (c) { c.id = buf(c.id); });
      var credential = await navigator.credentials.create({ publicKey: options });
      if (!credential) return null;
      return JSON.stringify({
        id: credential.id,
        rawId: b64u(credential.rawId),
        type: credential.type,
        extensions: credential.getClientExtensionResults(),
        response: {
          attestationObject: b64u(credential.response.attestationObject),
          clientDataJSON: b64u(credential.response.clientDataJSON),
        },
      });
    })()
    """
)

@OptIn(ExperimentalWasmJsInterop::class)
private fun jsGet(optionsJson: String): Promise<JsString?> = js(
    """
    (async function () {
      var b64u = function (buffer) {
        var bytes = new Uint8Array(buffer), s = '';
        for (var i = 0; i < bytes.length; i++) s += String.fromCharCode(bytes[i]);
        return btoa(s).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+${'$'}/, '');
      };
      var buf = function (value) {
        var s = atob(String(value).replace(/-/g, '+').replace(/_/g, '/'));
        var bytes = new Uint8Array(s.length);
        for (var i = 0; i < s.length; i++) bytes[i] = s.charCodeAt(i);
        return bytes.buffer;
      };
      var options = JSON.parse(optionsJson).publicKey;
      options.challenge = buf(options.challenge);
      (options.allowCredentials || []).forEach(function (c) { c.id = buf(c.id); });
      var credential = await navigator.credentials.get({ publicKey: options });
      if (!credential) return null;
      var response = {
        authenticatorData: b64u(credential.response.authenticatorData),
        clientDataJSON: b64u(credential.response.clientDataJSON),
        signature: b64u(credential.response.signature),
      };
      if (credential.response.userHandle) response.userHandle = b64u(credential.response.userHandle);
      return JSON.stringify({
        id: credential.id,
        rawId: b64u(credential.rawId),
        type: credential.type,
        extensions: credential.getClientExtensionResults(),
        response: response,
      });
    })()
    """
)

private class BrowserPasskeys : Passkeys {
    override val available: Boolean get() = runCatching { jsAvailable() }.getOrDefault(false)

    override suspend fun create(optionsJson: String): String? =
        runCatching { jsCreate(optionsJson).await() }.getOrNull()?.toString()

    override suspend fun get(optionsJson: String): String? =
        runCatching { jsGet(optionsJson).await() }.getOrNull()?.toString()
}

actual fun createPasskeys(): Passkeys = BrowserPasskeys()
