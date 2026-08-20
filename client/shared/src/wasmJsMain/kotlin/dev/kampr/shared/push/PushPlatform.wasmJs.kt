package dev.kampr.shared.push

import kotlinx.coroutines.await
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlin.js.ExperimentalWasmJsInterop
import kotlin.js.JsString
import kotlin.js.Promise

// Every branch a browser can take, resolved in JS because that is where the answers live —
// `navigator.standalone`, `isSecureContext`, `Notification.permission` — and returned as one word
// so the Kotlin side has a single thing to match on.
@OptIn(ExperimentalWasmJsInterop::class)
private fun jsCapability(): String = js(
    """
    (function () {
      if (!self.isSecureContext) return 'insecure';
      var ios = /iP(hone|ad|od)/.test(navigator.userAgent) ||
                (navigator.platform === 'MacIntel' && navigator.maxTouchPoints > 1);
      var standalone = navigator.standalone === true ||
                       (window.matchMedia && window.matchMedia('(display-mode: standalone)').matches);
      if (!('serviceWorker' in navigator) || !('PushManager' in window) || !('Notification' in window)) {
        return ios && !standalone ? 'homescreen' : 'unsupported';
      }
      if (ios && !standalone) return 'homescreen';
      return Notification.permission;
    })()
    """
)

// Registration and the token hand-off are one call: the worker is useless without the token and
// the token is useless without the worker.
@OptIn(ExperimentalWasmJsInterop::class)
private fun jsPrepare(token: String): Unit = js(
    """
    (function () {
      if (!('serviceWorker' in navigator) || !self.isSecureContext) return;
      navigator.serviceWorker.register('/sw.js', { scope: '/' }).then(function (registration) {
        var post = function () {
          var worker = registration.active || navigator.serviceWorker.controller;
          if (worker) worker.postMessage({ type: 'kampr-token', token: token || null });
        };
        post();
        navigator.serviceWorker.ready.then(post);
      }).catch(function () {});
    })()
    """
)

// The permission prompt and the subscription in one round trip, because a browser only honours
// `requestPermission` inside the user gesture that started it — splitting them across two suspend
// points loses the gesture and the call silently resolves to 'default'.
@OptIn(ExperimentalWasmJsInterop::class)
private fun jsSubscribe(key: String): Promise<JsString?> = js(
    """
    (async function () {
      if (!('serviceWorker' in navigator) || !('PushManager' in window)) return null;
      var permission = await Notification.requestPermission();
      if (permission !== 'granted') return null;
      var registration = await navigator.serviceWorker.register('/sw.js', { scope: '/' });
      await navigator.serviceWorker.ready;
      var existing = await registration.pushManager.getSubscription();
      // A subscription made against a different application server key is dead to this node, and
      // a browser refuses to re-subscribe over it. Dropping it is the only way forward.
      if (existing) {
        var same = existing.options && existing.options.applicationServerKey
          ? btoa(String.fromCharCode.apply(null, new Uint8Array(existing.options.applicationServerKey)))
              .replace(/\+/g, '-').replace(/\//g, '_').replace(/=+${'$'}/, '') === key
          : false;
        if (!same) { try { await existing.unsubscribe(); } catch (e) {} existing = null; }
      }
      var subscription = existing || await registration.pushManager.subscribe({
        userVisibleOnly: true,
        applicationServerKey: key,
      });
      return JSON.stringify(subscription.toJSON());
    })()
    """
)

@OptIn(ExperimentalWasmJsInterop::class)
private fun jsUnsubscribe(): Promise<JsString?> = js(
    """
    (async function () {
      if (!('serviceWorker' in navigator)) return null;
      var registration = await navigator.serviceWorker.getRegistration('/');
      if (!registration) return null;
      var subscription = await registration.pushManager.getSubscription();
      if (!subscription) return null;
      var endpoint = subscription.endpoint;
      try { await subscription.unsubscribe(); } catch (e) {}
      return endpoint;
    })()
    """
)

@OptIn(ExperimentalWasmJsInterop::class)
private fun jsEndpoint(): Promise<JsString?> = js(
    """
    (async function () {
      if (!('serviceWorker' in navigator)) return null;
      var registration = await navigator.serviceWorker.getRegistration('/');
      if (!registration) return null;
      var subscription = await registration.pushManager.getSubscription();
      return subscription ? subscription.endpoint : null;
    })()
    """
)

private class BrowserPush : PushPlatform {
    private val json = Json { ignoreUnknownKeys = true }

    override fun capability(): PushCapability = when (jsCapability()) {
        "insecure" -> PushCapability.InsecureContext
        "homescreen" -> PushCapability.NeedsHomeScreen
        "granted" -> PushCapability.Ready(PushPermission.Granted)
        "denied" -> PushCapability.Ready(PushPermission.Denied)
        "default" -> PushCapability.Ready(PushPermission.Default)
        else -> PushCapability.Unsupported
    }

    override fun prepare(token: String?) {
        jsPrepare(token.orEmpty())
    }

    override suspend fun subscribe(vapidKey: String): PushEnrolment? {
        val raw = runCatching { jsSubscribe(vapidKey).await() }.getOrNull()?.toString() ?: return null
        val body = runCatching { json.parseToJsonElement(raw).jsonObject }.getOrNull() ?: return null
        val endpoint = body["endpoint"]?.jsonPrimitive?.contentOrNull() ?: return null
        val keys = (body["keys"] as? JsonObject) ?: return null
        return PushEnrolment(
            endpoint = endpoint,
            p256dh = keys["p256dh"]?.jsonPrimitive?.contentOrNull() ?: return null,
            auth = keys["auth"]?.jsonPrimitive?.contentOrNull() ?: return null,
        )
    }

    override suspend fun unsubscribe(): String? =
        runCatching { jsUnsubscribe().await() }.getOrNull()?.toString()

    override suspend fun currentEndpoint(): String? =
        runCatching { jsEndpoint().await() }.getOrNull()?.toString()
}

private fun kotlinx.serialization.json.JsonPrimitive.contentOrNull(): String? =
    content.takeIf { it.isNotEmpty() && it != "null" }

actual fun createPushPlatform(): PushPlatform = BrowserPush()
