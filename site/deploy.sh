#!/usr/bin/env bash
# Build the website and publish it to the gh-pages branch (GitHub Pages).
#
#   GH_TOKEN=… site/deploy.sh            # from CI, or locally with: GH_TOKEN=$(gh auth token)
#
# The page is static. The download buttons read release.json, which is the
# latest release's signed latest.json, fetched here at deploy time.
set -euo pipefail

repo="${GITHUB_REPOSITORY:-mohith2309real/mybot}"
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
out="$(mktemp -d)"

cp "$root/site/index.html" "$root/site/style.css" "$root/site/site.js" "$out/"
[ -f "$root/site/og.jpg" ] && cp "$root/site/og.jpg" "$out/"
cp "$root/mybot2/assets/icon.svg" "$out/icon.svg"
mkdir -p "$out/shots"
for s in conversation needs-you sign-in-request skills routines settings; do
  cp "$root/mybot2/docs/screenshots/$s.png" "$out/shots/"
done
if curl -fsSL "https://github.com/$repo/releases/latest/download/latest.json" -o "$out/release.json"; then
  echo "release.json: $(grep -o '"version": *"[^"]*"' "$out/release.json" | head -1)"
else
  echo "No release manifest yet; the buttons will point at the releases page."
  rm -f "$out/release.json"
fi
touch "$out/.nojekyll"   # serve the files as they are

cd "$out"
git init -q -b gh-pages
git add -A
git -c user.name="MyBot site" -c user.email="336427159+mohith2309real@users.noreply.github.com" \
  commit -q -m "Site from $(cd "$root" && git rev-parse --short HEAD)"
git push -q -f "https://x-access-token:${GH_TOKEN}@github.com/${repo}.git" gh-pages
echo "Published to gh-pages: https://${repo%%/*}.github.io/${repo#*/}/"
