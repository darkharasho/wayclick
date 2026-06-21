import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { LogicalSize } from "@tauri-apps/api/dpi";
import { codeToKeyName } from "./keymap";
import GrantAccess from "./GrantAccess";

type Phase = "idle" | "arming" | "running" | "holding";
type Action = "click" | "hold";
type Button = "left" | "middle" | "right";
type Access = {
  writable: boolean;
  moduleLoaded: boolean;
  ruleInstalled: boolean;
  inGroup: boolean;
};

const win = getCurrentWindow();

export default function App() {
  // Persisted settings so config survives restarts.
  const saved = useMemo<any>(() => {
    try {
      return JSON.parse(localStorage.getItem("wc.config") || "{}");
    } catch {
      return {};
    }
  }, []);

  // interval pieces
  const [hr, setHr] = useState<number>(saved.hr ?? 0);
  const [min, setMin] = useState<number>(saved.min ?? 0);
  const [sec, setSec] = useState<number>(saved.sec ?? 0);
  const [ms, setMs] = useState<number>(saved.ms ?? 100);

  const [button, setButton] = useState<Button>(saved.button ?? "left");
  const [action, setAction] = useState<Action>(saved.action ?? "click");
  const [doubleClick, setDoubleClick] = useState<boolean>(saved.doubleClick ?? false);
  const [holdKey, setHoldKey] = useState<string | null>(saved.holdKey ?? "W");

  const [repeatCount, setRepeatCount] = useState<number | null>(saved.repeatCount ?? null);
  const [fixed, setFixed] = useState<[number, number] | null>(saved.fixed ?? null);
  const [jitter, setJitter] = useState<number>(saved.jitter ?? 0);

  // Window pinned (always-on-top), persisted and applied to the window.
  const [pinned, setPinned] = useState<boolean>(saved.pinned ?? false);
  useEffect(() => {
    win.setAlwaysOnTop(pinned).catch(() => {});
  }, [pinned]);

  // Save settings whenever they change.
  useEffect(() => {
    localStorage.setItem(
      "wc.config",
      JSON.stringify({
        hr, min, sec, ms, button, action, doubleClick, holdKey,
        repeatCount, fixed, jitter, pinned,
      })
    );
  }, [hr, min, sec, ms, button, action, doubleClick, holdKey, repeatCount, fixed, jitter, pinned]);

  // Collapse states persist so a configured widget stays compact across runs.
  const [settingsOpen, setSettingsOpen] = useState(
    () => localStorage.getItem("wc.settingsOpen") !== "0"
  );
  const [advanced, setAdvanced] = useState(
    () => localStorage.getItem("wc.advancedOpen") === "1"
  );
  useEffect(() => {
    localStorage.setItem("wc.settingsOpen", settingsOpen ? "1" : "0");
  }, [settingsOpen]);
  useEffect(() => {
    localStorage.setItem("wc.advancedOpen", advanced ? "1" : "0");
  }, [advanced]);
  const [phase, setPhase] = useState<Phase>("idle");
  // The trigger the portal/compositor bound (e.g. "F6"), or null if unbound.
  const [hotkey, setHotkey] = useState<string | null>(null);
  const [capturing, setCapturing] = useState<"key" | null>(null);
  const [update, setUpdate] = useState<{ version: string; obj: any } | null>(null);
  const [access, setAccess] = useState<Access | null>(null);

  function checkAccess() {
    invoke<Access>("access_status")
      .then(setAccess)
      // Outside Tauri (plain browser preview) assume granted so the UI shows.
      .catch(() =>
        setAccess({ writable: true, moduleLoaded: true, ruleInstalled: true, inGroup: true })
      );
  }
  useEffect(checkAccess, []);

  // The portal binds asynchronously, so query the current hotkey on load (and
  // shortly after) in case the bound event fired before this mounted.
  useEffect(() => {
    const poll = () =>
      invoke<string | null>("hotkey_status")
        .then((v) => v && setHotkey(v))
        .catch(() => {});
    poll();
    const t = setTimeout(poll, 1500);
    return () => clearTimeout(t);
  }, []);


  const actionRef = useRef(action);
  actionRef.current = action;
  const phaseRef = useRef(phase);
  phaseRef.current = phase;
  const rootRef = useRef<HTMLDivElement>(null);
  // Remember the last key so toggling Hold target Mouse→Key restores it.
  const lastHoldKey = useRef<string>(saved.holdKey ?? "W");
  useEffect(() => {
    if (holdKey) lastHoldKey.current = holdKey;
  }, [holdKey]);

  // Grow/shrink the window to fit content (Advanced, Hold rows, etc.) so nothing
  // clips and there's no inner scrollbar.
  useEffect(() => {
    const el = rootRef.current;
    if (!el) return;
    let last = 0;
    const ro = new ResizeObserver(() => {
      const h = Math.ceil(el.getBoundingClientRect().height);
      if (h > 0 && Math.abs(h - last) > 1) {
        last = h;
        win.setSize(new LogicalSize(384, h)).catch(() => {});
      }
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const intervalMs = useMemo(
    () => ((hr * 60 + min) * 60 + sec) * 1000 + ms,
    [hr, min, sec, ms]
  );
  const cps = intervalMs > 0 ? 1000 / intervalMs : 0;
  const sweep = Math.min(2000, Math.max(300, intervalMs)) / 1000;

  // ---- engine status + hotkey wiring ----
  useEffect(() => {
    const unlistens: Array<Promise<() => void>> = [];
    unlistens.push(
      listen<{ phase: string }>("engine:status", (e) => {
        const p = e.payload.phase;
        if (p === "arming") setPhase("arming");
        else if (p === "running")
          setPhase(actionRef.current === "hold" ? "holding" : "running");
        else setPhase("idle");
      })
    );
    unlistens.push(listen("hotkey:toggle", () => toggle()));
    unlistens.push(listen<string>("hotkey:bound", (e) => setHotkey(e.payload)));
    unlistens.push(listen("hotkey:unbound", () => setHotkey(null)));
    unlistens.push(
      listen<[number, number]>("point:picked", (e) => setFixed([e.payload[0], e.payload[1]]))
    );
    return () => {
      unlistens.forEach((u) => u.then((f) => f()));
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // ---- key capture (hold key / hotkey rebind) ----
  useEffect(() => {
    if (!capturing) return;
    const handler = (e: KeyboardEvent) => {
      e.preventDefault();
      const name = codeToKeyName(e);
      if (name) {
        setHoldKey(name);
        setCapturing(null);
      }
    };
    window.addEventListener("keydown", handler, true);
    return () => window.removeEventListener("keydown", handler, true);
  }, [capturing]);

  // ---- auto update check (no-op until signing keys are set) ----
  useEffect(() => {
    (async () => {
      try {
        const { check } = await import("@tauri-apps/plugin-updater");
        const update = await check();
        if (update) setUpdate({ version: update.version, obj: update });
      } catch {
        /* updater not configured yet */
      }
    })();
  }, []);

  // ---- actions ----
  async function start() {
    const config = {
      intervalMs,
      button,
      action,
      clickKind: doubleClick ? "double" : "single",
      holdKey: action === "hold" ? holdKey : null,
      repeat: repeatCount,
      position: fixed,
      jitterMs: jitter,
      repositionEachClick: false,
    };
    await invoke("start", { config });
  }
  async function stop() {
    await invoke("stop");
    setPhase("idle");
  }
  function toggle() {
    if (phaseRef.current === "idle") start();
    else stop();
  }
  function pickPoint() {
    invoke("pick_point").catch(() => {});
  }

  const running = phase !== "idle";

  // Compact "what's selected" summaries shown in each disclosure when collapsed.
  const settingsSummary =
    action === "hold"
      ? `hold ${holdKey ?? button}`
      : `${intervalMs} ms · ${button} · ${doubleClick ? "double" : "single"}`;
  const advancedSummary = [
    repeatCount == null ? "until stopped" : `${repeatCount}×`,
    fixed ? `fixed ${fixed[0]},${fixed[1]}` : "follow cursor",
    action === "click" && jitter ? `±${jitter}ms` : null,
  ]
    .filter(Boolean)
    .join(" · ");

  const num = (v: number, set: (n: number) => void, max: number) => (
    <input
      value={v}
      onChange={(e) => {
        const n = parseInt(e.target.value.replace(/\D/g, "") || "0", 10);
        set(Math.max(0, Math.min(max, n)));
      }}
    />
  );

  return (
    <div className="app" data-phase={phase} data-mode={action} ref={rootRef}>
      {/* title bar */}
      <div className="titlebar" data-tauri-drag-region>
        <div className="tb-brand" data-tauri-drag-region>
          <span className="mk" />
          <b>
            way<i>click</i>
          </b>
          <span className="tb-tag">wayland · uinput</span>
        </div>
        <div className="tb-ctrls">
          <button
            className={"tb-btn" + (pinned ? " on" : "")}
            title={pinned ? "Unpin (allow behind other windows)" : "Keep on top"}
            onClick={() => setPinned((p) => !p)}
          >
            <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
              <path d="M12 17v5" />
              <path d="M9 10.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.76V7a1 1 0 0 1 1-1 2 2 0 0 0 0-4H8a2 2 0 0 0 0 4 1 1 0 0 1 1 1z" />
            </svg>
          </button>
          <button className="tb-btn" title="Minimize" onClick={() => win.minimize()}>
            <svg width="11" height="11" viewBox="0 0 11 11">
              <rect x="1" y="5" width="9" height="1.4" fill="currentColor" />
            </svg>
          </button>
          <button className="tb-btn close" title="Close" onClick={() => win.close()}>
            <svg width="11" height="11" viewBox="0 0 11 11">
              <path
                d="M1 1 L10 10 M10 1 L1 10"
                stroke="currentColor"
                strokeWidth="1.4"
                fill="none"
              />
            </svg>
          </button>
        </div>
      </div>

      {access && !access.writable ? (
        <GrantAccess status={access} onRecheck={checkAccess} />
      ) : (
      <div className="body">
        {/* cadence signature */}
        <div className="cad" style={{ ["--sweep" as any]: `${sweep}s` }}>
          <div className="ticks" />
          <div className="mid" />
          <div className="play" />
          <div className="sustain" />
          <div className="state">
            <span className="ld" />
            {phase === "idle"
              ? "ready"
              : phase === "arming"
              ? "arming"
              : phase === "holding"
              ? "holding"
              : "firing"}
          </div>
          <div className="read">
            {action === "hold" ? (
              <>
                <div className="big">
                  <span className="kc">{holdKey ?? button}</span>
                  <span className="lab">held down</span>
                </div>
                <div className="sub">
                  target
                  <br />
                  <b>{holdKey ? "key" : "mouse"}</b>
                </div>
              </>
            ) : (
              <>
                <div className="big">
                  {cps.toFixed(1)}
                  <span className="u">/sec</span>
                </div>
                <div className="sub">
                  every <b>{intervalMs} ms</b>
                  <br />
                  {running
                    ? "running…"
                    : hotkey
                    ? `press ${hotkey} to start`
                    : "press Start to begin"}
                </div>
              </>
            )}
          </div>
        </div>

        {/* primary */}
        <button className="primary" onClick={toggle}>
          {!running && <span className="tri" />}
          {running ? (action === "hold" ? "Release" : "Stop") : "Start"}
          {hotkey && <span className="k">{hotkey}</span>}
        </button>

        {/* common options */}
        <button className="disc" onClick={() => setSettingsOpen((s) => !s)}>
          Settings
          {settingsOpen ? (
            <span className="ln" />
          ) : (
            <span className="sum">{settingsSummary}</span>
          )}
          {settingsOpen ? "–" : "+"}
        </button>

        {settingsOpen && (
        <div className="list">
          {action === "click" && (
            <div className="row">
              <div className="nm">Interval</div>
              <div className="field">
                <div className="cell">
                  {num(hr, setHr, 99)}
                  <u>hr</u>
                </div>
                <div className="cell">
                  {num(min, setMin, 59)}
                  <u>min</u>
                </div>
                <div className="cell">
                  {num(sec, setSec, 59)}
                  <u>sec</u>
                </div>
                <div className="cell">
                  {num(ms, setMs, 999)}
                  <u>ms</u>
                </div>
              </div>
            </div>
          )}

          {action === "click" && (
            <Seg
              label="Button"
              value={button}
              onChange={(v) => setButton(v as Button)}
              options={[
                ["left", "Left"],
                ["middle", "Mid"],
                ["right", "Right"],
              ]}
            />
          )}

          <Seg
            label="Action"
            hint="click, or hold a key down"
            value={action}
            onChange={(v) => setAction(v as Action)}
            options={[
              ["click", "Click"],
              ["hold", "Hold"],
            ]}
          />

          {action === "hold" && (
            <>
              <div className="row">
                <div className="nm">
                  Hold
                  <small>a key, or a mouse button</small>
                </div>
                <div className="seg">
                  <button
                    className={holdKey !== null ? "on" : ""}
                    onClick={() => setHoldKey(lastHoldKey.current)}
                  >
                    Key
                  </button>
                  <button
                    className={holdKey === null ? "on" : ""}
                    onClick={() => setHoldKey(null)}
                  >
                    Mouse
                  </button>
                </div>
              </div>

              {holdKey !== null ? (
                <div className="row">
                  <div className="nm">Key</div>
                  <div className="pick">
                    <span className="kc">{holdKey}</span>
                    <button
                      className={"setk" + (capturing === "key" ? " capturing" : "")}
                      onClick={() => setCapturing("key")}
                    >
                      {capturing === "key" ? "press a key…" : "Set key"}
                    </button>
                  </div>
                </div>
              ) : (
                <Seg
                  label="Button"
                  value={button}
                  onChange={(v) => setButton(v as Button)}
                  options={[
                    ["left", "Left"],
                    ["middle", "Mid"],
                    ["right", "Right"],
                  ]}
                />
              )}
            </>
          )}

          {action === "click" && (
            <Seg
              label="Click"
              value={doubleClick ? "double" : "single"}
              onChange={(v) => setDoubleClick(v === "double")}
              options={[
                ["single", "Single"],
                ["double", "Double"],
              ]}
            />
          )}
        </div>
        )}

        {/* advanced */}
        <button className="disc" onClick={() => setAdvanced((a) => !a)}>
          Advanced
          {advanced ? (
            <span className="ln" />
          ) : (
            <span className="sum">{advancedSummary}</span>
          )}
          {advanced ? "–" : "+"}
        </button>

        {advanced && (
          <div className="list">
            {action === "click" && (
              <Seg
                label="Repeat"
                hint="how many clicks"
                value={repeatCount == null ? "inf" : "count"}
                onChange={(v) => setRepeatCount(v === "inf" ? null : 100)}
                options={[
                  ["inf", "Until stopped"],
                  ["count", "Count"],
                ]}
              />
            )}
            {action === "click" && repeatCount != null && (
              <div className="row">
                <div className="nm">Count</div>
                <div className="field">
                  <div className="cell" style={{ width: 80 }}>
                    <input
                      value={repeatCount}
                      onChange={(e) =>
                        setRepeatCount(
                          Math.max(1, parseInt(e.target.value.replace(/\D/g, "") || "1", 10))
                        )
                      }
                    />
                    <u>clicks</u>
                  </div>
                </div>
              </div>
            )}

            <div className="row">
              <div className="nm">
                Position
                <small>where it clicks</small>
              </div>
              <div className="pick">
                <div className="seg">
                  <button className={fixed ? "" : "on"} onClick={() => setFixed(null)}>
                    Follow
                  </button>
                  <button
                    className={fixed ? "on" : ""}
                    onClick={() => (fixed ? undefined : pickPoint())}
                  >
                    Fixed
                  </button>
                </div>
                {fixed && (
                  <button className="setk" onClick={pickPoint}>
                    Set point
                  </button>
                )}
              </div>
            </div>

            {action === "click" && (
              <div className="row">
                <div className="nm">
                  Randomize
                  <small>humanize timing</small>
                </div>
                <div className="seg">
                  {[
                    [0, "Off"],
                    [25, "±25"],
                    [50, "±50"],
                  ].map(([v, l]) => (
                    <button
                      key={v}
                      className={jitter === v ? "on" : ""}
                      onClick={() => setJitter(v as number)}
                    >
                      {l}
                    </button>
                  ))}
                </div>
              </div>
            )}

            <div className="row">
              <div className="nm">
                Hotkey
                <small>
                  {hotkey
                    ? "toggles start/stop, system-wide"
                    : "assign a key in your shortcut settings"}
                </small>
              </div>
              <div className="pick">
                <span className="kc" style={hotkey ? undefined : { opacity: 0.4 }}>
                  {hotkey ?? "—"}
                </span>
                <button className="setk" onClick={() => invoke("open_shortcut_settings")}>
                  {hotkey ? "Change" : "Set"}
                </button>
              </div>
            </div>
          </div>
        )}
      </div>
      )}

      {update && (
        <div className="toast">
          <div className="t">
            Update available
            <small>v{update.version} ready to install</small>
          </div>
          <button
            onClick={async () => {
              await update.obj.downloadAndInstall();
              const { relaunch } = await import("@tauri-apps/plugin-process");
              await relaunch();
            }}
          >
            Restart
          </button>
        </div>
      )}
    </div>
  );
}

function Seg({
  label,
  hint,
  value,
  onChange,
  options,
}: {
  label: string;
  hint?: string;
  value: string;
  onChange: (v: string) => void;
  options: string[][];
}) {
  return (
    <div className="row">
      <div className="nm">
        {label}
        {hint && <small>{hint}</small>}
      </div>
      <div className="seg">
        {options.map(([v, l]) => (
          <button key={v} className={value === v ? "on" : ""} onClick={() => onChange(v)}>
            {l}
          </button>
        ))}
      </div>
    </div>
  );
}
