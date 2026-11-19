// Helpers for tools/gui-drive/cdp.mjs steps: cell(row, col), col(title), edit(row, title, value), nav(title).
const cell = (r, c) => document.querySelectorAll('#content tbody tr')[r].children[c + 1];
const col = (title) => [...document.querySelectorAll('#content thead th')].findIndex(th => (th.querySelector('.th-title') || th).firstChild.textContent === title) - 1;
const sleep = (ms) => new Promise(r => setTimeout(r, ms));
async function edit(r, title, value) {
  const td = cell(r, col(title));
  td.dispatchEvent(new MouseEvent('dblclick', { bubbles: true }));
  await sleep(50);
  const ctl = td.querySelector('input, select');
  ctl.value = value;
  if (ctl.tagName === 'SELECT') ctl.dispatchEvent(new Event('change'));
  else ctl.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
  await sleep(400);
  return document.getElementById('status').textContent;
}
const nav = async (title) => { [...document.querySelectorAll('nav button')].find(b => b.firstChild.textContent === title).click(); await sleep(300); };
const btn = (text) => [...document.querySelectorAll('button')].find(b => b.textContent === text && b.offsetParent !== null);
