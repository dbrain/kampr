package dev.kampr.conversation

// The browser is the only honest answer about what the browser will play, and the answer differs
// by engine and by build — Chromium ships no AAC in some Linux packages, Firefox reads FLAC and
// not ALAC. So it is asked, rather than guessed at from a list written here.
private fun jsCanPlay(mime: String): Boolean = js(
    """
    (function () {
      try { return new Audio().canPlayType(mime) !== ''; } catch (e) { return false; }
    })()
    """,
)

// The element is pointed at the node's stream ticket and fetches its own ranges, so seeking an hour
// in reads from there rather than from the start. It is held on the JS side under a token rather
// than handed back across the boundary.
private fun jsOpen(url: String): Int = js(
    """
    (function () {
      try {
        if (!window.__kamprVoices) window.__kamprVoices = { next: 1, held: {} };
        var audio = new Audio();
        audio.preload = 'metadata';
        audio.src = url;
        var token = window.__kamprVoices.next++;
        window.__kamprVoices.held[token] = audio;
        return token;
      } catch (e) {
        return -1;
      }
    })()
    """,
)

private fun jsPlay(token: Int) {
    js(
        """
        (function () {
          var audio = window.__kamprVoices && window.__kamprVoices.held[token];
          if (!audio) return;
          if (audio.ended) audio.currentTime = 0;
          var started = audio.play();
          if (started && started.catch) started.catch(function () {});
        })()
        """,
    )
}

private fun jsPause(token: Int) {
    js(
        """
        (function () {
          var audio = window.__kamprVoices && window.__kamprVoices.held[token];
          if (audio) audio.pause();
        })()
        """,
    )
}

private fun jsSeek(token: Int, seconds: Double) {
    js(
        """
        (function () {
          var audio = window.__kamprVoices && window.__kamprVoices.held[token];
          if (audio) audio.currentTime = seconds;
        })()
        """,
    )
}

private fun jsPlaying(token: Int): Boolean = js(
    """
    (function () {
      var audio = window.__kamprVoices && window.__kamprVoices.held[token];
      return !!audio && !audio.paused && !audio.ended;
    })()
    """,
)

private fun jsFailed(token: Int): Boolean = js(
    """
    (function () {
      var audio = window.__kamprVoices && window.__kamprVoices.held[token];
      return !audio || !!audio.error;
    })()
    """,
)

private fun jsPosition(token: Int): Double = js(
    """
    (function () {
      var audio = window.__kamprVoices && window.__kamprVoices.held[token];
      return audio ? audio.currentTime : 0;
    })()
    """,
)

// `NaN` until the metadata has arrived, and `Infinity` for a stream that never says.
private fun jsDuration(token: Int): Double = js(
    """
    (function () {
      var audio = window.__kamprVoices && window.__kamprVoices.held[token];
      return audio && isFinite(audio.duration) ? audio.duration : -1;
    })()
    """,
)

// Emptying `src` and calling `load` is what makes the element drop its connection and its buffer;
// a paused element with its source still set keeps both for as long as the page lives.
private fun jsRelease(token: Int) {
    js(
        """
        (function () {
          var audio = window.__kamprVoices && window.__kamprVoices.held[token];
          if (!audio) return;
          audio.pause();
          audio.removeAttribute('src');
          audio.load();
          delete window.__kamprVoices.held[token];
        })()
        """,
    )
}

private val answered = HashMap<String, Boolean>()

actual val deviceVoices: Voices = object : Voices {
    override fun canPlay(mime: String?): Boolean {
        val type = audioType(mime) ?: return false
        return answered.getOrPut(type) { runCatching { jsCanPlay(type) }.getOrDefault(false) }
    }

    override fun stream(url: String, mime: String?): Voice = WebVoice(runCatching { jsOpen(url) }.getOrDefault(-1))
}

private class WebVoice(private val token: Int) : Voice {
    private var released = token < 0

    override fun play() {
        if (!released) jsPlay(token)
    }

    override fun pause() {
        if (!released) jsPause(token)
    }

    override fun seek(ms: Long) {
        if (!released) jsSeek(token, ms / 1000.0)
    }

    override fun release() {
        if (released) return
        released = true
        jsRelease(token)
    }

    override val playing: Boolean get() = !released && jsPlaying(token)

    override val failed: Boolean get() = token < 0 || (!released && jsFailed(token))

    override val positionMs: Long get() = if (released) 0L else (jsPosition(token) * 1000).toLong()

    override val durationMs: Long? get() = if (released) null else jsDuration(token).takeIf { it >= 0 }?.let { (it * 1000).toLong() }
}
