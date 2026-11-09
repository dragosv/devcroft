import { chromium } from 'playwright';
import { readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';

const cfg = JSON.parse(await readFile(process.argv[2], 'utf8'));
const result = { status: 'failed', phase: 'launch', mode: cfg.mode, websocket_frames_received: 0,
  websocket_frames_sent: 0, blocked_external_origins: [], console_errors: [] };
result.playwright = JSON.parse(await readFile(new URL('./node_modules/playwright/package.json', import.meta.url))).version;
result.client_node = process.version;
let browser;
let page;
const external = new Set();
try {
  browser = await chromium.launch({ executablePath: '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
    headless: true });
  result.browser = browser.version();
  const context = await browser.newContext({ viewport: { width: 1440, height: 1000 }, locale: 'en-US',
    serviceWorkers: 'block', reducedMotion: 'reduce' });
  await context.route('**/*', route => {
    const url = new URL(route.request().url());
    if (url.origin === new URL(cfg.url).origin) return route.continue();
    external.add(url.origin);
    return route.abort();
  });
  page = await context.newPage();
  page.setDefaultTimeout(15000);
  page.on('console', msg => { if (msg.type() === 'error' && result.console_errors.length < 20)
    result.console_errors.push(msg.text().replaceAll(cfg.password, '<redacted>').slice(0, 500)); });
  page.on('websocket', ws => {
    ws.on('framereceived', () => result.websocket_frames_received++);
    ws.on('framesent', () => result.websocket_frames_sent++);
  });
  result.phase = 'login';
  await page.goto(cfg.url, { waitUntil: 'domcontentloaded', timeout: 90000 });
  const password = page.locator('input[type="password"]');
  await password.waitFor({ state: 'visible', timeout: 30000 });
  result.unauthenticated_shows_login = true;
  await password.fill(cfg.password);
  await password.press('Enter');
  result.phase = 'workbench';
  await page.locator('.monaco-workbench').waitFor({ state: 'visible', timeout: 120000 });
  const trust = page.getByRole('button', { name: /Yes, I trust|Trust Folder & Continue/ });
  try { await trust.waitFor({ state: 'visible', timeout: 5000 }); await trust.click();
    result.fixture_workspace_trust_accepted = true; } catch { /* No trust prompt on this boot. */ }
  await page.keyboard.press('Escape');
  result.phase = 'open-file';
  await page.getByRole('treeitem', { name: /^main\.c$/ }).dblclick();
  await page.getByRole('tab', { name: /main\.c/ }).waitFor({ state: 'visible' });
  // Chat also embeds Monaco. Only the file editor group qualifies this probe.
  const editor = page.locator('.editor-group-container .monaco-editor:visible').first();
  await editor.waitFor({ state: 'visible', timeout: 30000 });
  await editor.click({ position: { x: 120, y: 30 } });
  if (cfg.mode === 'edit') {
    result.phase = 'edit-save';
    await page.keyboard.press('Meta+a');
    await page.keyboard.insertText(cfg.source);
    await page.keyboard.press('Meta+s');
    await page.waitForTimeout(500);
    if (!(await editor.locator('.view-lines').innerText()).includes(cfg.marker))
      throw new Error('edited marker not present in the file editor');
    await page.screenshot({ path: path.join(cfg.out, 'editor-saved.png'), fullPage: true });
    result.saved_via_editor_ui = true;
    result.phase = 'terminal';
    await page.keyboard.press('F1');
    const quick = page.locator('.quick-input-box input');
    await quick.waitFor({ state: 'visible' });
    await quick.fill('>Terminal: Create New Terminal');
    await page.keyboard.press('Enter');
    const terminal = page.locator('.xterm:visible').last();
    await terminal.waitFor({ state: 'visible', timeout: 30000 });
    // Current Code can defer the fixture trust prompt until terminal creation.
    const terminalTrust = page.getByRole('button', { name: 'Trust Folder & Continue', exact: true });
    try {
      await terminalTrust.waitFor({ state: 'visible', timeout: 5000 });
      await terminalTrust.click();
      result.fixture_workspace_trust_accepted = true;
    } catch { /* This fixture may already have been trusted on the first boot. */ }
    await terminal.click({ position: { x: 100, y: 60 } });
    await terminal.locator('textarea.xterm-helper-textarea').focus();
    await page.waitForTimeout(1000);
    const command = 'cd /workspace/editor-project && cc -O2 -Wall -Werror main.c -o app && ./app > terminal-output.txt && uname -m > machine.txt && printf "EDITOR_TERMINAL_OK\\n"';
    await terminal.locator('textarea.xterm-helper-textarea').evaluate((input, text) => {
      const clipboard = new DataTransfer();
      clipboard.setData('text/plain', text);
      input.dispatchEvent(new ClipboardEvent('paste', { clipboardData: clipboard, bubbles: true, cancelable: true }));
    }, command);
    await page.waitForTimeout(200);
    const input = terminal.locator('textarea.xterm-helper-textarea');
    await input.focus();
    result.terminal_input_focused = await input.evaluate(el => document.activeElement === el);
    if (!result.terminal_input_focused) throw new Error('terminal input focus was intercepted');
    await input.press('Enter', { delay: 100 });
    await page.waitForTimeout(2000);
    await page.screenshot({ path: path.join(cfg.out, 'editor-terminal.png'), fullPage: true });
    result.terminal_command_entered_in_ui = true;
    result.terminal_input_method = 'xterm paste event; no system clipboard';
  } else {
    result.phase = 'reopen-persisted-file';
    await page.waitForTimeout(500);
    const text = await editor.locator('.view-lines').innerText();
    if (!text.includes(cfg.marker)) throw new Error('persisted source not visible in editor');
    await page.screenshot({ path: path.join(cfg.out, 'editor-reopened.png'), fullPage: true });
    result.persisted_source_visible_in_ui = true;
  }
  if (result.websocket_frames_received === 0 || result.websocket_frames_sent === 0)
    throw new Error('no functioning bidirectional editor WebSocket');
  result.status = 'passed';
} catch (error) {
  result.error = String(error).replaceAll(cfg.password, '<redacted>').slice(0, 3000);
  if (page) {
    await page.screenshot({ path: path.join(cfg.out, 'editor-failure.png'), fullPage: true }).catch(() => {});
    result.visible_text = (await page.locator('body').innerText().catch(() => '')).replaceAll(cfg.password, '<redacted>').slice(0, 5000);
  }
} finally {
  if (browser) await browser.close();
  result.blocked_external_origins = [...external];
  await writeFile(path.join(cfg.out, 'browser-report.json'), JSON.stringify(result, null, 2) + '\n');
}
console.log(JSON.stringify(result));
process.exitCode = result.status === 'passed' ? 0 : 1;
