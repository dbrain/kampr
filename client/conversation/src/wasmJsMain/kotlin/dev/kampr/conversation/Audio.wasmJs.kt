package dev.kampr.conversation

import kotlin.io.encoding.Base64
import kotlin.js.ExperimentalWasmJsInterop

// The browser is the only honest answer about what the browser will play, and the answer differs
// by engine and by build — Chromium ships no AAC in some Linux packages, Firefox reads FLAC and
// not ALAC. So it is asked, rather than guessed at from a list written here.
@OptIn(ExperimentalWasmJsInterop::class)
private fun jsCanPlay(mime: String): Boolean = js(
    """
    (function () {
      try { return new Audio().canPlayType(mime) !== ''; } catch (e) { return false; }
    })()
    """
)

// The bytes came out of an authorised fetch, so there is no URL an `<audio>` element could be
// pointed at a second time — a media element cannot carry a bearer header. An object URL over the
// bytes already in hand is the only form of playback available here, and it has to be revoked, so
// the element is held on the JS side under a token rather than handed back across the boundary.
@OptIn(ExperimentalWasmJsInterop::class)
private fun jsOpen(mime: String, base64: String): Int = js(
    """
    (function () {
      try {
        var binary = atob(base64);
        var buffer = new Uint8Array(binary.length);
        for (var i = 0; i < binary.length; i++) buffer[i] = binary.charCodeAt(i);
        var url = URL.createObjectURL(new Blob([buffer], { type: mime }));
        if (!window.__kamprVoices) window.__kamprVoices = { next: 1, held: {} };
        var token = window.__kamprVoices.next++;
        window.__kamprVoices.held[token] = { audio: new Audio(url), url: url };
        return token;
      } catch (e) {
        return -1;
      }
    })()
    """
)

@OptIn(ExperimentalWasmJsInterop::class)
private fun jsPlay(token: Int) {
    js(
        """
        (function () {
          var held = window.__kamprVoices && window.__kamprVoices.held[token];
          if (!held) return;
          if (held.audio.ended) held.audio.currentTime = 0;
          var started = held.audio.play();
          if (started && started.catch) started.catch(function () {});
        })()
        """
    )
}

@OptIn(ExperimentalWasmJsInterop::class)
private fun jsPause(token: Int) {
    js(
        """
        (function () {
          var held = window.__kamprVoices && window.__kamprVoices.held[token];
          if (held) held.audio.pause();
        })()
        """
    )
}

@OptIn(ExperimentalWasmJsInterop::class)
private fun jsPlaying(token: Int): Boolean = js(
    """
    (function () {
      var held = window.__kamprVoices && window.__kamprVoices.held[token];
      return !!held && !held.audio.paused && !held.audio.ended;
    })()
    """
)

@OptIn(ExperimentalWasmJsInterop::class)
private fun jsRelease(token: Int) {
    js(
        """
        (function () {
          var held = window.__kamprVoices && window.__kamprVoices.held[token];
          if (!held) return;
          held.audio.pause();
          held.audio.removeAttribute('src');
          URL.revokeObjectURL(held.url);
          delete window.__kamprVoices.held[token];
        })()
        """
    )
}

private val answered = HashMap<String, Boolean>()

actual val deviceVoices: Voices = object : Voices {
    override fun canPlay(mime: String?): Boolean {
        val type = audioType(mime) ?: return false
        return answered.getOrPut(type) { runCatching { jsCanPlay(type) }.getOrDefault(false) }
    }

    override fun open(bytes: ByteArray, mime: String?): Voice? {
        val encoded = runCatching { Base64.encode(bytes) }.getOrNull() ?: return null
        // An empty type on the blob leaves the sniffing to the browser, which is a better guess
        // than one written here would be.
        val type = audioType(mime) ?: ""
        val token = runCatching { jsOpen(type, encoded) }.getOrDefault(-1)
        return if (token < 0) null else WebVoice(token)
    }
}

private class WebVoice(private val token: Int) : Voice {
    private var released = false

    override fun play() {
        if (!released) jsPlay(token)
    }

    override fun pause() {
        if (!released) jsPause(token)
    }

    override fun release() {
        if (released) return
        released = true
        jsRelease(token)
    }

    override val playing: Boolean get() = !released && jsPlaying(token)
}
