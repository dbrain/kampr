// karma-webpack builds its bundle under `os.tmpdir()` and leaves `_karma_webpack_*` behind on
// every run. On this machine /tmp is a 32 GB tmpfs shared with everything else, and 517 of those
// directories holding 14 GB is what made `cargo test` fail with "Disk quota exceeded" rather than
// anything to do with the tests. os.tmpdir() reads these on first call, so setting them here —
// before the framework initialises — puts the bundle inside the module's own build directory,
// which `gradlew clean` already owns.
const path = require("path");
const fs = require("fs");
const scratch = path.resolve(__dirname, "..", "karma-tmp");
fs.mkdirSync(scratch, { recursive: true });
process.env.TMPDIR = scratch;
process.env.TMP = scratch;
process.env.TEMP = scratch;
