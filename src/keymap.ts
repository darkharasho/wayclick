// Map a browser KeyboardEvent to the key names the engine understands
// (see wayclick-input::common_keys).

export function codeToKeyName(e: KeyboardEvent): string | null {
  const c = e.code;
  if (/^Key[A-Z]$/.test(c)) return c.slice(3);
  if (/^Digit[0-9]$/.test(c)) return c.slice(5);
  const map: Record<string, string> = {
    Space: "Space",
    Enter: "Enter",
    Tab: "Tab",
    Escape: "Esc",
    Backspace: "Backspace",
    CapsLock: "CapsLock",
    ShiftLeft: "LeftShift",
    ShiftRight: "RightShift",
    ControlLeft: "LeftCtrl",
    ControlRight: "RightCtrl",
    AltLeft: "LeftAlt",
    AltRight: "RightAlt",
    MetaLeft: "LeftMeta",
    MetaRight: "RightMeta",
    ArrowUp: "Up",
    ArrowDown: "Down",
    ArrowLeft: "Left",
    ArrowRight: "Right",
  };
  if (map[c]) return map[c];
  if (/^F([1-9]|1[0-2])$/.test(c)) return c;
  return null;
}

// Build a Tauri accelerator string ("Ctrl+Shift+F6") from a KeyboardEvent.
export function eventToAccelerator(e: KeyboardEvent): string | null {
  const mods: string[] = [];
  if (e.ctrlKey) mods.push("Ctrl");
  if (e.shiftKey) mods.push("Shift");
  if (e.altKey) mods.push("Alt");
  if (e.metaKey) mods.push("Super");

  const c = e.code;
  let key: string | null = null;
  if (/^Key[A-Z]$/.test(c)) key = c.slice(3);
  else if (/^Digit[0-9]$/.test(c)) key = c.slice(5);
  else if (/^F([1-9]|1[0-2])$/.test(c)) key = c;
  else if (c === "Space") key = "Space";

  if (!key) return null;
  return [...mods, key].join("+");
}
