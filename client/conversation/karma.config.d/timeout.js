// Mocha's two-second default is too short for a composition in ChromeHeadless on a loaded machine;
// see terminal/karma.config.d/timeout.js for the measurement.
config.set({
    client: Object.assign({}, config.client, { mocha: { timeout: 30000 } }),
});
