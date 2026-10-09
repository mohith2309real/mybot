// MyBot site: reveal on scroll, the faces (drawn live, as the app draws
// them), and download buttons pointed at the latest release.

document.documentElement.classList.replace("no-js", "js");
const still = matchMedia("(prefers-reduced-motion: reduce)").matches;

// ── reveal ───────────────────────────────────────────────────────────────
const io = "IntersectionObserver" in window
  ? new IntersectionObserver((entries) => {
      for (const e of entries) {
        if (e.isIntersecting) {
          e.target.classList.add("in");
          io.unobserve(e.target);
        }
      }
    }, { rootMargin: "0px 0px -8% 0px", threshold: 0.08 })
  : null;
for (const el of document.querySelectorAll(".reveal")) {
  if (io) io.observe(el); else el.classList.add("in");
}

// ── faces ────────────────────────────────────────────────────────────────
// A port of the app's faces (mybot2/crates/app/src/gui/theme.rs): the same
// shapes, the same moves, the same timings. Radii are nw, ne, sw, se.
const FACES = [
  { name: "Pip", head: [0.84, 0.80, [0.30, 0.30, 0.30, 0.30]], eyes: "capsule", gap: 0.13, gaze: [0, -0.03], right: 1, work: ["read", "type", "scan"], idle: ["glance", "breathe"], call: ["bounce", "pulse"], cheer: ["hop", "sparkle"] },
  { name: "Glance", head: [0.84, 0.81, [0.29, 0.29, 0.29, 0.29]], eyes: "capsule", gap: 0.132, gaze: [0.053, -0.078], right: 1.1, work: ["scan", "think", "read"], idle: ["glance", "breathe"], call: ["pulse", "wiggle"], cheer: ["sparkle", "hop"] },
  { name: "Bean", head: [0.62, 0.88, [0.31, 0.31, 0.31, 0.31]], eyes: "dot", gap: 0.10, gaze: [0, -0.10], right: 1, work: ["type", "hum", "read"], idle: ["breathe", "doze"], call: ["bounce", "wiggle"], cheer: ["hop", "sway"] },
  { name: "Brick", head: [0.92, 0.64, [0.20, 0.20, 0.20, 0.20]], eyes: "bar", gap: 0.16, gaze: [0, -0.02], right: 1, work: ["read", "type"], idle: ["doze", "glance"], call: ["wiggle", "pulse"], cheer: ["sway", "sparkle"] },
  { name: "Leaf", head: [0.82, 0.82, [0.06, 0.41, 0.41, 0.06]], eyes: "capsule", gap: 0.12, gaze: [0.02, -0.02], right: 1, work: ["think", "scan", "hum"], idle: ["glance", "breathe"], call: ["pulse", "bounce"], cheer: ["sparkle", "sway"] },
  { name: "Ghost", head: [0.78, 0.86, [0.39, 0.39, 0.10, 0.10]], eyes: "dot", gap: 0.12, gaze: [0, -0.08], right: 1, work: ["hum", "scan"], idle: ["breathe", "glance"], call: ["wiggle", "bounce"], cheer: ["sway", "hop"] },
  { name: "Moon", head: [0.86, 0.86, [0.43, 0.43, 0.43, 0.43]], eyes: "square", gap: 0.13, gaze: [0, -0.03], right: 1, work: ["scan", "read", "think"], idle: ["doze", "glance"], call: ["bounce", "pulse"], cheer: ["hop", "sparkle"] },
  { name: "Blob", head: [0.88, 0.78, [0.40, 0.28, 0.28, 0.40]], eyes: "capsule", gap: 0.14, gaze: [-0.04, -0.04], right: 0.9, work: ["hum", "type", "think"], idle: ["breathe", "glance"], call: ["pulse", "wiggle"], cheer: ["sway", "hop"] },
];
const EYES = { capsule: [0.10, 0.25], dot: [0.13, 0.13], bar: [0.19, 0.075], square: [0.12, 0.12] };
const SHADES = ["#f5f5f5", "#dfdfdf", "#c6c6c6", "#b1b1b1"];
const INK = "#0e0e0e";
const MOODS = ["idle", "working", "needs", "done"];
const MOOD_LABEL = { idle: "Idle", working: "Working", needs: "Needs you", done: "Done" };
const WORK_LABEL = { read: "Reading", type: "Typing", think: "Thinking", scan: "Scanning", hum: "Humming along" };
const faceSceneOnly = document.documentElement.dataset.faceScene === "true";

const clamp01 = (u) => Math.max(0, Math.min(1, u));
const ease = (u) => { u = clamp01(u); return u * u * (3 - 2 * u); };
const bump = (u) => Math.sin(Math.PI * clamp01(u));
const hold = (u) => ease(u / 0.2) * ease((1 - u) / 0.2);
function episode(t, every, len, offset) {
  const x = t + offset, n = Math.floor(x / every), local = x - n * every;
  return local < len ? [n, local / len] : [n, null];
}
const rest = () => ({ hx: 0, hy: 0, sx: 1, sy: 1, gx: 0, gy: 0, e: [[1, 1], [1, 1]], happy: 0, sparkle: 0 });
function lerpPose(a, b, t) {
  const l = (x, y) => x + (y - x) * t;
  return { hx: l(a.hx, b.hx), hy: l(a.hy, b.hy), sx: l(a.sx, b.sx), sy: l(a.sy, b.sy), gx: l(a.gx, b.gx), gy: l(a.gy, b.gy),
    e: [[l(a.e[0][0], b.e[0][0]), l(a.e[0][1], b.e[0][1])], [l(a.e[1][0], b.e[1][0]), l(a.e[1][1], b.e[1][1])]], happy: l(a.happy, b.happy), sparkle: l(a.sparkle, b.sparkle) };
}
function workPose(w, t) {
  const p = rest();
  if (w === "read") {
    const local = (t / 2) % 1;
    let x;
    if (local < 0.86) { const v = local / 0.86 * 4; x = (Math.floor(v) + ease((v % 1) / 0.25)) / 4; } else x = 1 - ease((local - 0.86) / 0.14);
    p.gx = -0.07 + 0.14 * x; p.gy = 0.01; p.e = [[1, 0.8], [1, 0.8]];
  } else if (w === "type") {
    p.gx = 0.012 * Math.sin(t * 7); p.gy = 0.055;
    p.hy = 0.014 * Math.abs(Math.sin(t * 13)) * Math.max(0, 0.5 + 0.5 * Math.sin(t * 1.7));
    p.e = [[1, 0.62], [1, 0.62]];
  } else if (w === "think") {
    p.gx = 0.05 * Math.sin(t * 0.8); p.gy = -0.085; p.e = [[1, 1], [1, 0.6]]; p.hx = 0.012 * Math.sin(t * 0.8);
  } else if (w === "scan") {
    p.gx = 0.065 * Math.cos(t * 2.4); p.gy = -0.01 + 0.035 * Math.sin(t * 2.4);
  } else if (w === "hum") {
    p.e = [[1, 0.32], [1, 0.32]]; p.hx = 0.022 * Math.sin(t * 2.8); p.hy = -0.008 * Math.abs(Math.sin(t * 5.6));
  }
  return p;
}
function pose(f, mood, t, phase) {
  let p = rest(), doing = null;
  if (mood === "working") {
    const n = f.work.length, x = t + phase * 7, i = Math.floor(x / 5.5), local = x - i * 5.5;
    const now = workPose(f.work[i % n], t);
    doing = f.work[i % n];
    p = local > 5.1 ? lerpPose(now, workPose(f.work[(i + 1) % n], t), ease((local - 5.1) / 0.4)) : now;
  } else if (mood === "idle") {
    const [i, u] = episode(t, 6.5, 1.6, phase * 4.1);
    if (u !== null) {
      const m = f.idle[i % f.idle.length];
      if (m === "glance") p.gx = 0.075 * (i % 2 === 0 ? 1 : -1) * hold(u);
      if (m === "breathe") { const b = bump(u); p.sx = 1 - 0.025 * b; p.sy = 1 + 0.045 * b; p.hy = -0.012 * b; }
      if (m === "doze") { const h = hold(u); p.e = [[1, 1 - 0.72 * h], [1, 1 - 0.72 * h]]; p.hy = 0.016 * h; }
    }
  } else if (mood === "needs") {
    p.gx = -f.gaze[0]; p.gy = -0.07 - f.gaze[1]; p.e = [[1.22, 1.22], [1.22, 1.22]];
    const [i, u] = episode(t, 2.4, 0.8, phase);
    if (u !== null) {
      const b = bump(u), m = f.call[i % f.call.length];
      if (m === "bounce") { p.hy = -0.07 * b; p.sx = 1 - 0.03 * b; p.sy = 1 + 0.05 * b; }
      if (m === "wiggle") p.hx = 0.04 * Math.sin(u * 6 * Math.PI) * (1 - u);
      if (m === "pulse") { const k = 1.22 * (1 + 0.3 * b); p.e = [[k, k], [k, k]]; }
    }
  } else if (mood === "done") {
    p.happy = 1;
    const [i, u] = episode(t, 3.0, 0.9, phase);
    if (u !== null) {
      const b = bump(u), m = f.cheer[i % f.cheer.length];
      if (m === "hop") { p.hy = -0.06 * b; p.sx = 1 - 0.03 * b; p.sy = 1 + 0.04 * b; }
      if (m === "sway") p.hx = 0.035 * Math.sin(u * 2 * Math.PI);
      if (m === "sparkle") { p.sparkle = u; p.hy = -0.025 * b; }
    }
  }
  if (mood === "idle" || mood === "needs") {
    const [, u] = episode(t, 4.3, 0.16, phase * 2.3);
    if (u !== null) for (const e of p.e) e[1] *= 1 - 0.94 * bump(u);
  }
  return [p, doing];
}
function drawFace(g, size, f, mood, t, phase, shade) {
  const s = size, [p, doing] = pose(f, mood, t, phase);
  g.clearRect(0, 0, s, s);
  const cx = s / 2 + p.hx * s, cy = s / 2 + p.hy * s;
  const w = f.head[0] * p.sx * s, h = f.head[1] * p.sy * s, k = Math.min(p.sx, p.sy) * s;
  const [nw, ne, sw, se] = f.head[2].map((r) => Math.min(r * k, w / 2, h / 2));
  g.fillStyle = shade;
  g.beginPath(); g.roundRect(cx - w / 2, cy - h / 2, w, h, [nw, ne, se, sw]); g.fill();
  const base = EYES[f.eyes];
  const eyes = [[-f.gap + f.gaze[0], f.gaze[1], 1], [f.gap + f.gaze[0], f.gaze[1], f.right]];
  g.fillStyle = INK; g.strokeStyle = INK;
  if (p.happy > 0.5) {
    g.lineWidth = Math.max(1.2, 0.045 * s); g.lineCap = "round"; g.lineJoin = "round";
    for (const [ex, ey] of eyes) {
      const x = cx + (ex + p.gx) * s, y = cy + (ey + p.gy - 0.01) * s, ww = 0.075 * s, hh = 0.06 * s;
      g.beginPath(); g.moveTo(x - ww, y + hh / 2); g.lineTo(x, y - hh / 2); g.lineTo(x + ww, y + hh / 2); g.stroke();
    }
  } else {
    eyes.forEach(([ex, ey, r], i) => {
      const ew = base[0] * r * p.e[i][0] * s, eh = Math.max(0.03 * s, base[1] * r * p.e[i][1] * s);
      const rad = f.eyes === "square" ? Math.min(ew, eh) * 0.28 : Math.min(ew, eh) / 2;
      g.beginPath(); g.roundRect(cx + (ex + p.gx) * s - ew / 2, cy + (ey + p.gy) * s - eh / 2, ew, eh, rad); g.fill();
    });
  }
  if (p.sparkle > 0) {
    const b = bump(p.sparkle), ax = cx + w / 2 + (-0.02 + 0.02 * p.sparkle) * s, ay = cy - h / 2 + (0.04 - 0.03 * p.sparkle) * s;
    const L = 0.11 * s * b, T = 0.025 * s * b;
    g.fillStyle = shade;
    for (const [a, c] of [[[L, 0], [0, T]], [[0, L], [T, 0]]]) {
      g.beginPath(); g.moveTo(ax - a[0], ay - a[1]); g.lineTo(ax - c[0], ay - c[1]); g.lineTo(ax + a[0], ay + a[1]); g.lineTo(ax + c[0], ay + c[1]); g.closePath(); g.fill();
    }
  }
  return doing;
}

// The launch video calls the same renderer with its own frame time, so its
// expressions stay deterministic and match the website's face design.
window.MyBotFaceDrawing = Object.freeze({ faces: FACES, shades: SHADES, draw: drawFace });

// Every face canvas on the page, drawn each frame while it's on screen.
const live = new Set();
const all = [];
function addFace(canvas, face, mood, phase, onDoing) {
  const item = { canvas, g: canvas.getContext("2d"), face: FACES[face], mood, phase, shade: SHADES[face % 4], onDoing, size: 0 };
  all.push(item);
  if ("IntersectionObserver" in window) {
    new IntersectionObserver((es) => es.forEach((e) => (e.isIntersecting ? live.add(item) : live.delete(item)))).observe(canvas);
  } else live.add(item);
  return item;
}
function fit(item) {
  const dpr = Math.min(2, window.devicePixelRatio || 1);
  const css = item.canvas.clientWidth || 64;
  const px = Math.round(css * dpr);
  if (item.canvas.width !== px) { item.canvas.width = px; item.canvas.height = px; }
  item.size = px;
}
function frame(now) {
  const t = now / 1000;
  for (const item of live) {
    fit(item);
    const doing = drawFace(item.g, item.size, item.face, item.mood, t, item.phase, item.shade);
    if (item.onDoing) item.onDoing(doing);
  }
  if (!still) requestAnimationFrame(frame);
}

document.querySelectorAll(".crew canvas").forEach((c, i) => addFace(c, +c.dataset.face, { needs: "needs", done: "done", idle: "idle" }[c.dataset.mood] || "working", 2 + i * 0.9));

const grid = document.getElementById("face-grid");
if (grid) {
  FACES.forEach((f, i) => {
    const card = document.createElement("button");
    card.className = "face-card";
    card.type = "button";
    card.innerHTML = `<canvas aria-hidden="true"></canvas><b>${f.name}</b><span></span>`;
    grid.appendChild(card);
    const label = card.querySelector("span");
    let m = (i % 4 === 1) ? 1 : (i % 4 === 2 ? 2 : (i % 4 === 3 ? 3 : 0));
    let since = 0;
    const item = addFace(card.querySelector("canvas"), i, MOODS[m], 1 + i * 1.3, (doing) => {
      label.textContent = item.mood === "working" && doing ? WORK_LABEL[doing] : MOOD_LABEL[item.mood];
    });
    const next = () => { m = (m + 1) % MOODS.length; item.mood = MOODS[m]; since = performance.now(); };
    card.addEventListener("click", next);
    card.setAttribute("aria-label", `${f.name}: tap to change its mood`);
    // On their own, the cards move through the moods, out of step.
    if (!still) setInterval(() => { if (performance.now() - since > 4000) next(); }, 6500 + i * 450);
  });
}
if (!faceSceneOnly) requestAnimationFrame(frame);

// ── downloads ────────────────────────────────────────────────────────────
function mine() {
  const ua = navigator.userAgent || "";
  const plat = (navigator.userAgentData && navigator.userAgentData.platform) || navigator.platform || "";
  if (/Win/i.test(plat) || /Windows/i.test(ua)) return { key: "windows-x86_64", label: "Download for Windows" };
  if (/Mac/i.test(plat) || /Mac OS X/i.test(ua)) {
    // Browsers don't reveal Apple silicon vs Intel reliably; most Macs sold
    // since 2020 are Apple silicon, and the Intel build is one card away.
    if (/iPhone|iPad/i.test(ua)) return null;
    return { key: "macos-arm64", label: "Download for Mac" };
  }
  if (/Linux/i.test(plat) && !/Android/i.test(ua)) return { key: "linux-x86_64", label: "Download for Linux" };
  return null;
}

const mb = (n) => `${(n / 1048576).toFixed(n > 10485760 ? 0 : 1)} MB`;

// The latest release, read live so the site never needs redeploying when a
// release ships. release.json (a snapshot taken at deploy time) is the
// fallback if that's unreachable or rate-limited.
async function latestRelease() {
  try {
    const r = await fetch("https://api.github.com/repos/mohith2309real/mybot/releases/latest", { headers: { Accept: "application/vnd.github+json" } });
    if (!r.ok) throw r.status;
    const rel = await r.json();
    const assets = {};
    for (const a of rel.assets || []) {
      const m = a.name.match(/^mybot2-v[^-]+-(.+?)\.(tar\.gz|zip)$/);
      if (!m) continue;
      assets[m[1]] = { name: a.name, url: a.browser_download_url, size: a.size, sha256: (a.digest || "").replace(/^sha256:/, "") };
    }
    if (!Object.keys(assets).length) throw "no assets";
    return { version: rel.tag_name.replace(/^v/, ""), assets };
  } catch {
    const r = await fetch("release.json", { cache: "no-cache" });
    if (!r.ok) throw r.status;
    return r.json();
  }
}

if (!faceSceneOnly) {
  latestRelease()
  .then((rel) => {
    const assets = rel.assets || {};
    for (const el of document.querySelectorAll('[data-release="version"]')) el.textContent = `MyBot ${rel.version}`;
    for (const el of document.querySelectorAll('[data-release="label"]')) el.textContent = `MyBot ${rel.version} · free`;

    for (const card of document.querySelectorAll("[data-asset]")) {
      const a = assets[card.dataset.asset];
      if (!a) continue;
      card.href = a.url;
      const ext = a.name.endsWith(".zip") ? ".zip" : ".tar.gz";
      card.querySelector(".meta").textContent = `${ext} · ${mb(a.size)}`;
    }

    const m = mine();
    const cta = document.getElementById("cta");
    if (m && assets[m.key]) {
      cta.href = assets[m.key].url;
      document.getElementById("cta-label").textContent = m.label;
      document.getElementById("cta-fine").textContent =
        `MyBot ${rel.version} · ${mb(assets[m.key].size)}${m.key === "macos-arm64" ? " · Apple silicon (Intel below)" : ""} · needs Docker`;
      const card = document.querySelector(`[data-asset="${m.key}"]`);
      if (card) card.classList.add("mine");
    }

    const sums = Object.values(assets).filter((a) => a.sha256).map((a) => `${a.name}  ${a.sha256}`).join("\n");
    const el = document.getElementById("sums");
    if (el && sums) el.innerText = `SHA-256\n${sums}`;
  })
  .catch(() => {
    // Nothing to point at yet: the buttons stay on the download section.
  });
}
