// Mocha gives a test two seconds by default, and a test in this module drives a real
// `TerminalView` inside ChromeHeadless: a composition, a font measurement, and a frame of a
// full pane per synthesised event. On an idle machine a forty-notch wheel walk fits; with three
// agents building beside it at a load average of 47 it does not, and the failure arrives as
// "Timeout of 2000ms exceeded" on whichever tests happened to be running — including ones nobody
// touched. That reads as a hang and is nothing but a long test on a busy machine.
//
// Thirty seconds hides no defect. A test that really hangs still fails, and #432's watchdog
// measurement is what tells the two apart: `waitForIdle()` blocks the browser's main thread
// outright, and no timeout of any length rescues that.
config.set({
    client: Object.assign({}, config.client, { mocha: { timeout: 30000 } }),
});
