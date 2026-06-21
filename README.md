<div align="center">

<img src="assets/icon.png" width="104" alt="wayclick icon" />

# wayclick

**A sleek, minimal autoclicker for Linux / Wayland.**

Built in Rust + Tauri. Writes input events straight to `/dev/uinput`, so it works
on Wayland without asking for permission on every use — the consent cost moves to
a single, one-time setup.

</div>

---

## Why

Most Linux autoclickers lean on X11 input injection, which Wayland deliberately
blocks — so on Wayland they nag for approval every time (or don't work at all).
wayclick goes *under* that layer: it creates virtual input devices via the
kernel's `uinput` interface. The trade is a one-time grant (a udev rule + group
membership) instead of per-use friction. After first run, it's silent forever.

## Features

- **Click or hold** — rapid clicking, or press-and-hold a key / mouse button
- **Button choice** — left / middle / right
- **Single or double** click
- **Fixed or randomized** intervals (down to the millisecond) with optional jitter
- **Finite count or infinite** ("until stopped")
- **Follow-cursor or fixed-position** — pick a target pixel with a fullscreen overlay
- **System-wide hotkey** (F6 by default) via the XDG GlobalShortcuts portal
- **Auto-updates**, a calm first-run access gate, and a compact collapsible UI

## Compositor support

The click/hold engine is portable (any compositor that uses libinput). **Absolute
positioning is compositor-specific**: there is no usable absolute virtual pointer
on KWin, so wayclick reads the true cursor position from the compositor and
converges via relative motion. This is implemented for **KWin / Plasma 6**
(Wayland) today, behind a trait so other backends can slot in.

> Note: driving input into a game running under an exclusive **gamescope** grab is
> a separate problem and not currently supported — run wayclick against normal
> windows.

## Install

Download the latest **AppImage** (or `.deb`) from the
[Releases](https://github.com/darkharasho/wayclick/releases) page, make it
executable, and run it:

```sh
chmod +x wayclick_*.AppImage
./wayclick_*.AppImage
```

On first launch, wayclick walks you through the one-time access grant (a udev rule
+ adding you to the `input` group). Log out and back in once for the group to take
effect — then you're set.

## Build from source

Requires Rust, Node 18+, and the Tauri Linux dependencies
(`libwebkit2gtk-4.1-dev`, `libgtk-3-dev`, `librsvg2-dev`, `patchelf`).

```sh
npm install
npm run dev          # run the app in dev mode
npm run tauri build  # produce an AppImage / .deb
```

## How it works

- `crates/wayclick-input` — the engine (no UI deps). A `VirtualMouse` and
  `VirtualKeyboard` over `/dev/uinput`, a `ClickEngine`, and absolute positioning
  behind a `PointerPositioner` trait (KWin closed-loop backend).
- `src-tauri` + `src` — the Tauri shell and React UI.

## Credits & license

Released under the [MIT License](LICENSE). The uinput approach was inspired by
[TheClicker](https://github.com/konkitoman/autoclicker) (MIT) — forked in spirit,
not in code.
