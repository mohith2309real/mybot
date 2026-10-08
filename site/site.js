// MyBot site: reveal on scroll, play the hand-off, and point the download
// buttons at the latest release.

const REPO = "https://github.com/mohith2309real/mybot";

document.documentElement.classList.remove("no-js");

// Reveal sections as they scroll in.
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

// Start the hand-off loop once the window image has painted.
const stage = document.querySelector(".stage");
const startPlay = () => stage && stage.classList.add("play");
const heroImg = stage && stage.querySelector("img");
if (heroImg && !heroImg.complete) heroImg.addEventListener("load", startPlay, { once: true });
else startPlay();

// Which download is this visitor's?
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

// The latest release, read live from GitHub so the site never needs
// redeploying when a release ships. release.json (a snapshot taken at deploy
// time) is the fallback if the API is unreachable or rate-limited.
async function latestRelease() {
  try {
    const r = await fetch("https://api.github.com/repos/mohith2309real/mybot/releases/latest", { headers: { Accept: "application/vnd.github+json" } });
    if (!r.ok) throw r.status;
    const rel = await r.json();
    const tag = rel.tag_name;
    const assets = {};
    for (const a of rel.assets || []) {
      const m = a.name.match(/^mybot2-v[^-]+-(.+?)\.(tar\.gz|zip)$/);
      if (!m) continue;
      assets[m[1]] = { name: a.name, url: a.browser_download_url, size: a.size, sha256: (a.digest || "").replace(/^sha256:/, "") };
    }
    if (!Object.keys(assets).length) throw "no assets";
    return { version: tag.replace(/^v/, ""), tag, assets };
  } catch {
    const r = await fetch("release.json", { cache: "no-cache" });
    if (!r.ok) throw r.status;
    return r.json();
  }
}

latestRelease()
  .then((rel) => {
    const assets = rel.assets || {};
    const tag = rel.tag || `v${rel.version}`;

    for (const el of document.querySelectorAll('[data-release="version"]')) el.textContent = `MyBot ${rel.version}`;
    for (const el of document.querySelectorAll('[data-release="label"]')) el.textContent = `MyBot ${rel.version} · free and open source`;
    for (const a of document.querySelectorAll('a[href$="/mybot2/README.md"]')) a.href = `${REPO}/blob/${tag}/mybot2/README.md`;

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
    } else {
      cta.href = "#download";
    }

    const sums = Object.values(assets).filter((a) => a.sha256).map((a) => `${a.name}  ${a.sha256}`).join("\n");
    const el = document.getElementById("sums");
    if (el && sums) {
      el.innerText = `SHA-256\n${sums}`;
    }
  })
  .catch(() => {
    // No manifest (yet): every button already points at the releases page.
  });
