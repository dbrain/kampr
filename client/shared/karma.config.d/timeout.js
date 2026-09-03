// The same two-second default that made `:terminal:wasmJsBrowserTest` fail on whichever tests
// happened to be running when the machine was busy (#444). This module's browser tests drive real
// DOM work — a keyboard and a paste through the page — so they are the other ones with something
// to spend two seconds on.
//
// Thirty seconds hides no defect. A test that genuinely hangs still fails, and #432 is what tells
// a hang from a slow test: `waitForIdle()` blocks the browser's main thread outright, and no
// timeout of any length rescues that.
//
// `conversation` and `mosaic` deliberately have no copy of this. Their browser runs are their
// common tests compiled for wasm — arithmetic and grouping, with no page to drive — so they have
// nothing to spend the two seconds on and a config file there would be clutter pretending to be
// caution.
config.set({
    client: Object.assign({}, config.client, { mocha: { timeout: 30000 } }),
});
