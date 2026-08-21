// Two jobs, both of which have to happen before wasm exists.
//
// This file is external rather than inline because the node's CSP is
// `script-src 'self' 'wasm-unsafe-eval'` with no nonce and no hash for it, so the inline script
// this replaces was blocked on every single load — and the ground it sets is precisely what a
// cold load flashes wrong without it.

// The saved ground has to reach the boot background before the stylesheet resolves; the Compose
// token layer only exists once wasm has loaded. Key and values match AppState.
try {
  var mode = localStorage.getItem('mode');
  if (mode === 'dark' || mode === 'light') document.documentElement.setAttribute('data-ground', mode);
} catch (e) {
}

// `security.installable` has been rendered as "install to home screen" on the setup ladder with
// nothing at all behind it. The event fires once and early — long before wasm is up — so it is
// caught here and held for whoever asks.
window.kamprInstall = (function () {
  var deferred = null;
  window.addEventListener('beforeinstallprompt', function (event) {
    event.preventDefault();
    deferred = event;
  });
  window.addEventListener('appinstalled', function () {
    deferred = null;
  });
  return {
    available: function () {
      return !!deferred;
    },
    prompt: function () {
      if (!deferred) return Promise.resolve(false);
      var event = deferred;
      deferred = null;
      return event.prompt().then(function () {
        return event.userChoice;
      }).then(function (choice) {
        return choice.outcome === 'accepted';
      }).catch(function () {
        return false;
      });
    },
  };
})();
