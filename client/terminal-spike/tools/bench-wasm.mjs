import { chromium, firefox } from 'playwright';

const url = process.argv[2];
const engine = process.argv[3] || 'chromium';
const shots = process.argv.slice(4).map(s => {
    const i = s.indexOf(':');
    return { at: Number(s.slice(0, i)) * 1000, path: s.slice(i + 1) };
});
const exe = process.env.BROWSER_PATH;

const launcher = engine === 'firefox' ? firefox : chromium;
const opts = {
    headless: false,
    args: engine === 'firefox' ? [] : ['--enable-gpu', '--ignore-gpu-blocklist', '--window-size=1440,900'],
};
if (exe) opts.executablePath = exe;

const browser = await launcher.launch(opts);
const page = await browser.newPage({ viewport: { width: 1400, height: 860 } });
let done = false;
page.on('console', m => {
    const t = m.text();
    if (t.startsWith('KAMPR_')) console.log(t);
    if (t.startsWith('KAMPR_BENCH_DONE')) done = true;
});
page.on('pageerror', e => console.log('PAGEERROR ' + e));

const t0 = Date.now();
await page.goto(url, { waitUntil: 'load' });

for (const s of shots) {
    const wait = s.at - (Date.now() - t0);
    if (wait > 0) await page.waitForTimeout(wait);
    await page.screenshot({ path: s.path });
    console.log('KAMPR_SHOT ' + s.path);
}

const deadline = Date.now() + 20 * 60 * 1000;
while (!done && Date.now() < deadline) await page.waitForTimeout(1000);
if (!done) console.log('KAMPR_BENCH_TIMEOUT');
await browser.close();
