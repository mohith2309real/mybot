/*
 * Tiny DOM helpers.
 *
 * Everything the console renders — task logs above all — contains text the bot
 * scraped off a live web page. That is attacker-controlled input. So there is
 * exactly one rule in this file and it is not negotiable: nothing here ever
 * assigns innerHTML. Text goes in through textContent, attributes through
 * setAttribute. `el()` is the only element factory, which makes that rule
 * enforceable by reading one function.
 */

export function el(tag, props, ...children) {
  const node = document.createElement(tag);
  if (props) {
    for (const [key, value] of Object.entries(props)) {
      if (value === null || value === undefined || value === false) continue;
      if (key === 'class') node.className = value;
      else if (key === 'text') node.textContent = String(value);
      else if (key === 'dataset') Object.assign(node.dataset, value);
      else if (key.startsWith('on') && typeof value === 'function') {
        node.addEventListener(key.slice(2).toLowerCase(), value);
      } else if (value === true) node.setAttribute(key, '');
      else node.setAttribute(key, String(value));
    }
  }
  append(node, children);
  return node;
}

function append(node, children) {
  for (const child of children) {
    if (child === null || child === undefined || child === false) continue;
    if (Array.isArray(child)) append(node, child);
    else if (child instanceof Node) node.appendChild(child);
    else node.appendChild(document.createTextNode(String(child)));
  }
}

export function clear(node) {
  node.replaceChildren();
  return node;
}

export function $(selector, root = document) {
  return root.querySelector(selector);
}

/** Two capital letters for a bot avatar: "web-researcher" -> "WR". */
export function initials(name) {
  const parts = String(name).split(/[^a-z0-9]+/i).filter(Boolean);
  if (parts.length >= 2) return (parts[0][0] + parts[1][0]).toUpperCase();
  return String(name).slice(0, 2).toUpperCase();
}

export function clock(iso) {
  const d = iso ? new Date(iso) : new Date();
  if (Number.isNaN(d.getTime())) return '--:--:--';
  return d.toLocaleTimeString([], { hour12: false });
}

/** SQLite hands back "YYYY-MM-DD HH:MM:SS" in UTC with no zone marker. */
export function parseDbTime(value) {
  if (!value) return null;
  const iso = /\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}/.test(value)
    ? `${value.replace(' ', 'T')}Z`
    : value;
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? null : d;
}

export function whenText(value) {
  const d = parseDbTime(value);
  if (!d) return '—';
  const diff = (Date.now() - d.getTime()) / 1000;
  if (diff < 60) return 'just now';
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
  if (diff < 86_400) return `${Math.floor(diff / 3600)}h ago`;
  return d.toLocaleDateString([], { month: 'short', day: 'numeric' });
}

export function durationText(from, to) {
  const a = parseDbTime(from);
  const b = parseDbTime(to);
  if (!a || !b) return '—';
  const secs = Math.max(0, Math.round((b.getTime() - a.getTime()) / 1000));
  if (secs < 60) return `${secs}s`;
  const mins = Math.floor(secs / 60);
  return mins < 60 ? `${mins}m ${secs % 60}s` : `${Math.floor(mins / 60)}h ${mins % 60}m`;
}
