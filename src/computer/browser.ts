import { chromium, type Browser, type BrowserContext, type Page } from 'playwright-core';
import { activeCdpPort } from './container.ts';

/**
 * Browser control over CDP.
 *
 * We attach to the Chromium already running inside the container rather than
 * launching our own. That matters for step 3: the human takes over the *same*
 * session the bot is driving, via VNC onto the same X display. A separately
 * launched browser could not be handed over.
 *
 * Elements are addressed by `ref` — a number stamped onto the DOM during a
 * snapshot — instead of by CSS selector. Models are far more reliable picking
 * "ref 12" off a list they were just shown than inventing a selector, and a
 * stale ref fails loudly instead of silently clicking the wrong thing.
 */

const REF_ATTR = 'data-mybot-ref';

let browser: Browser | undefined;
let context: BrowserContext | undefined;

/**
 * Resolve the websocket endpoint, correcting the port Chromium reports.
 *
 * Chromium advertises `ws://127.0.0.1:<internal>/devtools/browser/<id>` — its
 * own address inside the container. Playwright reads that straight out of
 * /json/version and dials it verbatim, which from the host is a dead port, so
 * connectOverCDP hangs until it times out.
 *
 * The fix belongs here rather than in the relay. Rewriting the address in the
 * HTTP response body means changing its length (":9223" is 14 bytes,
 * ":52475" is 15), which desynchronises Content-Length and corrupts the reply.
 * Swapping the port on this side costs one fetch and cannot corrupt anything.
 */
async function wsEndpoint(port: number): Promise<string> {
  const res = await fetch(`http://127.0.0.1:${port}/json/version`, {
    signal: AbortSignal.timeout(10_000),
  });
  if (!res.ok) throw new Error(`CDP on :${port} answered ${res.status}`);

  const { webSocketDebuggerUrl } = (await res.json()) as { webSocketDebuggerUrl?: string };
  if (!webSocketDebuggerUrl) throw new Error(`CDP on :${port} advertised no websocket URL`);

  const url = new URL(webSocketDebuggerUrl);
  url.host = `127.0.0.1:${port}`;
  return url.toString();
}

export async function connect(): Promise<BrowserContext> {
  if (context) return context;
  browser = await chromium.connectOverCDP(await wsEndpoint(activeCdpPort()));
  context = browser.contexts()[0] ?? (await browser.newContext());
  return context;
}

export async function disconnect(): Promise<void> {
  await browser?.close().catch(() => {});
  browser = undefined;
  context = undefined;
}

/** The tab the bot is working in. Prefers the last non-blank page. */
export async function activePage(): Promise<Page> {
  const ctx = await connect();
  const pages = ctx.pages().filter((p) => !p.isClosed());
  if (!pages.length) return ctx.newPage();
  const real = pages.filter((p) => p.url() !== 'about:blank');
  return real.at(-1) ?? pages.at(-1)!;
}

export async function navigate(url: string): Promise<{ url: string; title: string }> {
  const page = await activePage();
  // Bare hostnames get https://, but anything that already names a scheme is
  // left alone — prepending https:// to a data:/file:/about: URL corrupts it.
  const target = /^[a-z][a-z0-9+.-]*:/i.test(url) ? url : `https://${url}`;
  await page.goto(target, { waitUntil: 'domcontentloaded', timeout: 45_000 });
  await settle(page);
  return { url: page.url(), title: await page.title() };
}

/** Give SPAs a beat to render without hanging on pages that never go idle. */
async function settle(page: Page): Promise<void> {
  await page.waitForLoadState('networkidle', { timeout: 5000 }).catch(() => {});
}

export interface Snapshot {
  url: string;
  title: string;
  text: string;
  elements: string;
}

/**
 * A compact, model-readable view of the page: visible text plus a numbered list
 * of things that can be interacted with.
 */
export async function snapshot(maxTextChars = 6000): Promise<Snapshot> {
  const page = await activePage();
  await settle(page);

  const result = await page.evaluate(
    ({ refAttr, maxChars }) => {
      for (const el of Array.from(document.querySelectorAll(`[${refAttr}]`))) {
        el.removeAttribute(refAttr);
      }

      const isVisible = (el: Element): boolean => {
        const r = el.getBoundingClientRect();
        if (r.width < 1 || r.height < 1) return false;
        const s = getComputedStyle(el);
        return s.visibility !== 'hidden' && s.display !== 'none' && Number(s.opacity) > 0.05;
      };

      const label = (el: Element): string => {
        const e = el as HTMLElement & { value?: string; placeholder?: string; type?: string };
        const raw =
          e.getAttribute('aria-label') ||
          e.getAttribute('placeholder') ||
          e.getAttribute('name') ||
          e.getAttribute('title') ||
          e.getAttribute('alt') ||
          (e.innerText || '').trim() ||
          e.value ||
          '';
        return raw.replace(/\s+/g, ' ').trim().slice(0, 80);
      };

      const SELECTOR =
        'a[href], button, input, select, textarea, [role=button], [role=link], [role=textbox], [role=checkbox], [contenteditable=true]';

      const lines: string[] = [];
      let ref = 0;
      for (const el of Array.from(document.querySelectorAll(SELECTOR))) {
        if (!isVisible(el)) continue;
        if (++ref > 150) break;
        el.setAttribute(refAttr, String(ref));
        const e = el as HTMLInputElement;
        const tag = el.tagName.toLowerCase();
        const kind = tag === 'input' ? `input:${e.type || 'text'}` : tag;
        const state = e.disabled ? ' (disabled)' : e.checked ? ' (checked)' : '';
        lines.push(`  [${ref}] ${kind} "${label(el)}"${state}`);
      }

      const text = (document.body?.innerText ?? '').replace(/\n{3,}/g, '\n\n').trim();

      return {
        url: location.href,
        title: document.title,
        text: text.length > maxChars ? `${text.slice(0, maxChars)}\n…[truncated]` : text,
        elements: lines.join('\n') || '  (no interactive elements found)',
      };
    },
    { refAttr: REF_ATTR, maxChars: maxTextChars },
  );

  return result;
}

function locate(page: Page, ref: number) {
  return page.locator(`[${REF_ATTR}="${ref}"]`);
}

export async function click(ref: number): Promise<string> {
  const page = await activePage();
  const el = locate(page, ref);
  if ((await el.count()) === 0) {
    throw new Error(`No element with ref ${ref}. Take a fresh page_read — refs change on every snapshot.`);
  }
  const desc = (await el.first().innerText().catch(() => ''))?.slice(0, 60) ?? '';
  await el.first().click({ timeout: 15_000 });
  await settle(page);
  return `clicked [${ref}] ${desc}`.trim();
}

export async function type(ref: number, text: string, submit = false): Promise<string> {
  const page = await activePage();
  const el = locate(page, ref);
  if ((await el.count()) === 0) {
    throw new Error(`No element with ref ${ref}. Take a fresh page_read — refs change on every snapshot.`);
  }
  await el.first().fill(text, { timeout: 15_000 });
  if (submit) {
    await el.first().press('Enter');
    await settle(page);
  }
  return `typed into [${ref}]${submit ? ' and pressed Enter' : ''}`;
}

export async function pressKey(key: string): Promise<string> {
  const page = await activePage();
  await page.keyboard.press(key);
  await settle(page);
  return `pressed ${key}`;
}

export async function scroll(direction: 'up' | 'down', amount = 600): Promise<string> {
  const page = await activePage();
  await page.mouse.wheel(0, direction === 'down' ? amount : -amount);
  await page.waitForTimeout(250);
  return `scrolled ${direction}`;
}

export async function screenshot(): Promise<Buffer> {
  const page = await activePage();
  return page.screenshot({ type: 'png' });
}
