import { useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";

// Fullscreen transparent click-catcher. One click reports the cursor position
// (read by Rust via the compositor) and closes; Esc cancels.
export default function Overlay() {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") invoke("cancel_pick");
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  return (
    <div className="overlay" onMouseDown={() => invoke("point_picked")}>
      <div className="overlay-hint">
        Click your target
        <small>Esc to cancel</small>
      </div>
    </div>
  );
}
