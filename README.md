# wayclick

Here's the overview of where we landed.

Wayclick is a sleek, minimal autoclicker for Linux/Wayland, built in Rust with Tauri as the front-end shell. The whole reason it exists is to dodge the per-use approval friction that makes xclicker annoying on Wayland — it does that by leaning on TheClicker's approach of writing input events straight to /dev/uinput, which sits below the layer where Wayland demands consent.

The permission cost doesn't disappear, it relocates to a one-time setup: a udev rule plus group membership granting access to /dev/uinput. That means a two-state first run — a "grant access" screen on first launch, then silence forever after. The one unavoidable wrinkle is that group membership needs a log-out/in to take effect. Designing that gate to feel calm rather than like an error dialog is most of the "sleek" feeling.

On design: "sleek minimal" is a principle, not a feature cut. Under the hood you're going for full parity with xclicker and OPAutoClicker — fixed and randomized intervals, button choice, single/double-click, finite-count vs infinite, fixed-position vs follow-cursor, and hold-to-repeat. The minimalism comes from progressive disclosure: sensible defaults up front, power features one layer down.

Fixed-position is the must-have that carries the most risk. It needs an absolute-capable virtual pointer (uinput ABS_X/ABS_Y, not just relative motion), and because Wayland hides the real cursor position, you can't do xclicker-style "click to capture." The answer is a fullscreen transparent Tauri overlay that grabs one click and reports its own local coordinates.

Two things stay genuinely unknown until you read TheClicker's source — I couldn't fetch it in this session, so don't build on assumptions: whether its engine is a reusable library or welded into the binary (depend vs. fork — my bet's fork), and whether its virtual device is absolute-capable or relative-only.

So the first move in the folder, before any scaffolding, is the spike: read the repo for crate shape, where the clicking lives, the device declaration, and the license — then prove an absolute pointer lands a click on a known pixel reliably and the capture overlay reports a coordinate. If those hold, everything else is plumbing.

The locked decisions are saved to memory, so they'll carry into the session where you actually create the folder.
