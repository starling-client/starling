# Sunrise's mark and icons

**The mark.** A pixel-art sunrise on a dark tile, the website's icon: rays, the sun's dome and the horizon
on whole units of a 16-unit grid, drawn with crisp edges, so it is sharp at 16 px and holds at 1024 px.

## Masters (edit these)

| File | What |
|------|------|
| `icon.svg` | the app icon on the 16 grid: the full square with 3-unit (3/16) corners (Windows, Linux, favicons) |
| `icon-macos.svg` | the same tile on Apple's 1024 grid (832 px tile, 156 px corners, transparent margin) |
| `avatar.svg` | full-bleed square for services that round or crop the image (GitHub, Discord) |
| `mark.svg`, `mark-dark.svg` | the mark alone, for light and for dark backgrounds |
| `social/card.html` | the GitHub social preview, as a page |

## Generated (don't edit)

`python3 tools/render_brand_icons.py` renders `icon.svg` pixel-exact (nearest neighbour) into:

| Output | Used by |
|--------|---------|
| `packaging/icons/hicolor/<size>/apps/sunrise.png` (16–512) and `scalable/apps/sunrise.svg` | Linux: copy the tree into `~/.local/share/icons/hicolor` (or `/usr/share/icons/hicolor`) |
| `sunrise-launcher.desktop`, `sunrise.desktop` (hand-written, in `packaging/linux/` of [sunrise-launcher](https://github.com/sunrise-org/sunrise-launcher)) | Linux: the launcher installs both and the hicolor icons itself, under `$XDG_DATA_HOME` (`~/.local/share`), with its own path in `Exec` (`src/desktop.rs` there; `sunrise-launcher desktop-entry --remove` takes them out). Wayland matches the windows' app ids `sunrise-launcher` and `sunrise` |
| `packaging/icons/sunrise.ico` | Windows: embedded in `sunrise-client.exe`, its tools and `sunrise-launcher.exe` as icon resource 1 (`build.rs` of each, Windows builds only) |
| `packaging/icons/sunrise.icns` | macOS: `Sunrise.app/Contents/Resources/sunrise.icns`, packaged with the launcher's .dmg by sunrise-launcher |
| `crates/sunrise-theme/icons/sunrise-64.png` | the game window's icon at run time (X11 and Windows title bars, `window::icon`) |
| `web/favicon.svg`, `web/favicon-32.png`, `web/apple-touch-icon.png` | the website (its `npm run assets` copies them into its `public/`) |
| `icon-1024.png`, `icon-macos-1024.png` | previews, store listings |
| `social/github-avatar-500.png` (from `avatar.svg` by `cargo run -p xtask -- icons`, which needs `rsvg-convert`) | GitHub: the organisation or account avatar |

The page in `social/` is a screenshot, at their exact size, with Playwright, from the repository's root:

```sh
d=$(mktemp -d) && npm install --prefix "$d" playwright && npx --prefix "$d" playwright install chromium
NODE_PATH="$d/node_modules" node -e 'const {chromium}=require("playwright");(async()=>{const b=await chromium.launch();for(const[f,w,h,o]of[["card",1280,640,"github-social-1280x640"]]){const p=await b.newPage({viewport:{width:w,height:h}});await p.goto("file://"+require("path").resolve("docs/media/brand/social/"+f+".html"));await p.waitForTimeout(300);await p.screenshot({path:"docs/media/brand/social/"+o+".png"})}await b.close()})()'
```

## The README banner

`docs/media/banner.webp` (1600x600) is the website's hero, a screenshot of playsunrise.net in its
dark theme. The navbar, the hero's buttons, its version line and its lede are hidden first, so the
banner carries no launcher version and no harness count (the README's prose owns the count), and
nothing in it goes stale. The crop starts 56 px above the kicker and ends at the hero's bottom
edge. With Playwright, from the repository's root:

```sh
d=$(mktemp -d) && npm install --prefix "$d" playwright && npx --prefix "$d" playwright install chromium
NODE_PATH="$d/node_modules" node -e 'const {chromium}=require("playwright");(async()=>{const b=await chromium.launch();const p=await b.newPage({viewport:{width:1600,height:1000}});await p.goto("https://playsunrise.net/",{waitUntil:"networkidle"});await p.click(".theme-toggle");await p.waitForTimeout(1200);await p.evaluate(()=>{for(const s of ["header",".hero-actions",".download-meta",".lede"]){const e=document.querySelector(s);if(e)e.style.display="none"}});await p.waitForTimeout(400);await p.screenshot({path:"docs/media/banner.webp",clip:{x:0,y:136,width:1600,height:600}});await b.close()})()'
```

## Uploading (the accounts are Ludovic's)

- **GitHub avatar**: github.com/settings/profile (or the organisation's settings), *Edit* the
  picture, upload `social/github-avatar-500.png`.
- **GitHub social preview**: the repository's *Settings › General › Social preview › Edit*,
  upload `social/github-social-1280x640.png`.
- **Discord** (the server icon and banner, Kira's avatar): they live in the private kira repo since 2026-10-15
  (`assets/`, its README's "Discord brand assets"): this repo holds only the game, the launcher and the website.
