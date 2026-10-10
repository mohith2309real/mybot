#!/usr/bin/env bash
# Build the website and publish it to Firebase Hosting: https://mybot2.web.app
#
#   site/deploy.sh                     build and publish
#   site/deploy.sh --build-only DIR    build into DIR (for a local preview)
#
# Uses the Firebase CLI's own login (`firebase login`), site `mybot2` in the
# project `mohith-sites-2309` (the same project as mohith2309.web.app and
# sparky-code.web.app). The page reads the latest release when it loads, so a
# new release needs no redeploy; release.json below is only a fallback
# snapshot for when that lookup fails.
set -euo pipefail

project="mohith-sites-2309"
site="mybot2"
repo="mohith2309real/mybot"
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if [ "${1:-}" = "--build-only" ]; then
  pub="${2:?usage: deploy.sh --build-only DIR}"
  out=""
else
  out="$(mktemp -d)"
  pub="$out/public"
fi
mkdir -p "$pub/shots" "$pub/fonts"

cp "$root/site/index.html" "$root/site/style.css" "$root/site/site.js" "$root/site/og.jpg" "$root/site/mybot-wallpaper-4k.jpg" "$pub/"
cp "$root/mybot2/assets/icon.svg" "$root/mybot2/assets/wordmark.svg" "$pub/"
cp "$root/mybot2/assets/fonts/selawkl.woff2" "$root/mybot2/assets/fonts/selawksl.woff2" "$root/mybot2/assets/fonts/Selawik-OFL.txt" "$pub/fonts/"
for s in conversation needs-you sign-in-request skills teammate-desktop; do
  cp "$root/mybot2/docs/screenshots/$s.png" "$pub/shots/"
done
curl -fsSL "https://github.com/$repo/releases/latest/download/latest.json" -o "$pub/release.json" \
  || { echo "No release manifest; skipping the fallback snapshot."; rm -f "$pub/release.json"; }

if [ -z "$out" ]; then
  echo "Built: $pub"
  exit 0
fi

cat > "$out/firebase.json" <<JSON
{
  "hosting": {
    "site": "$site",
    "public": "public",
    "cleanUrls": true,
    "headers": [
      { "source": "**/*.@(png|jpg|svg|woff2)", "headers": [{ "key": "Cache-Control", "value": "public, max-age=604800" }] },
      { "source": "**/*.@(html|js|css|json)", "headers": [{ "key": "Cache-Control", "value": "public, max-age=300" }] }
    ]
  }
}
JSON

cd "$out"
firebase deploy --only hosting --project "$project" --non-interactive
echo "Published: https://$site.web.app/"
