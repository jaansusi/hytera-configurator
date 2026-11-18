// Drive a running hyt-gui through WebView2's DevTools port (screenshots, scripted clicks/edits).
// Start the app with WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9333 (CDP_PORT overrides).
// Usage: node tools/gui-drive/cdp.mjs <steps.json>, steps = [["prelude", "tools/gui-drive/helpers.js"],
//   ["eval", "<js, may use await and return>"], ["shot", "file.png"], ["wait", ms], ...]
// Never script a click on Read/Write with a real serial port selected: reads/writes need the owner's OK.
import { readFileSync, writeFileSync } from 'node:fs';

const steps = JSON.parse(readFileSync(process.argv[2], 'utf8'));
const port = process.env.CDP_PORT || 9333;
const targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
const page = targets.find((t) => t.type === 'page');
const ws = new WebSocket(page.webSocketDebuggerUrl);
let next = 0;
const pending = new Map();
ws.onmessage = (m) => {
  const d = JSON.parse(m.data);
  if (d.id && pending.has(d.id)) { pending.get(d.id)(d); pending.delete(d.id); }
};
await new Promise((r) => { ws.onopen = r; });
const send = (method, params = {}) => new Promise((r) => {
  const id = ++next;
  pending.set(id, r);
  ws.send(JSON.stringify({ id, method, params }));
});

let prelude = '';
for (const [kind, arg] of steps) {
  if (kind === 'prelude') prelude = readFileSync(arg, 'utf8');
  if (kind === 'wait') await new Promise((r) => setTimeout(r, arg));
  if (kind === 'eval') {
    const r = await send('Runtime.evaluate', { expression: `(async () => { ${prelude}
${arg} })()`, awaitPromise: true, returnByValue: true });
    const v = r.result?.result?.value;
    if (r.result?.exceptionDetails) console.log('EXCEPTION', JSON.stringify(r.result.exceptionDetails).slice(0, 600));
    else if (v !== undefined) console.log(typeof v === 'string' ? v : JSON.stringify(v));
  }
  if (kind === 'shot') {
    const r = await send('Page.captureScreenshot', { format: 'png' });
    writeFileSync(arg, Buffer.from(r.result.data, 'base64'));
    console.log('saved', arg);
  }
}
ws.close();
