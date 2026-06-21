import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";

type Status = {
  writable: boolean;
  moduleLoaded: boolean;
  ruleInstalled: boolean;
  inGroup: boolean;
};

export default function GrantAccess({
  status,
  onRecheck,
}: {
  status: Status;
  onRecheck: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Setup ran (rule + group present) but the new group isn't active until the
  // user logs back in — the one unavoidable wrinkle, framed calmly.
  const pendingRelogin = status.ruleInstalled && status.inGroup && !status.writable;

  async function grant() {
    setBusy(true);
    setError(null);
    try {
      await invoke("grant_access");
      onRecheck();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="gate">
      <div className="gate-mark">
        <span />
        <span />
        <span className="tall" />
        <span />
      </div>

      {pendingRelogin ? (
        <>
          <h1>Almost there</h1>
          <p>
            Access is set up. Log out and back in once so the change takes effect,
            then reopen wayclick. You'll never see this screen again.
          </p>
          <button className="gate-btn" onClick={onRecheck}>
            Re-check
          </button>
        </>
      ) : (
        <>
          <h1>One-time setup</h1>
          <p>
            wayclick sends clicks straight to your input device, so it never has
            to ask Wayland for permission again. That needs a single grant.
          </p>
          <button className="gate-btn" onClick={grant} disabled={busy}>
            {busy ? "Waiting for password…" : "Grant access"}
          </button>
          <div className="gate-note">
            Adds you to the <code>input</code> group via a udev rule. Reversible
            anytime.
          </div>
        </>
      )}

      {error && <div className="gate-err">{error}</div>}
    </div>
  );
}
