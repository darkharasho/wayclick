import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { codeToKeyName, eventToAccelerator } from "./keymap";
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
  // interval pieces
  const [hr, setHr] = useState(0);
  const [min, setMin] = useState(0);
  const [sec, setSec] = useState(0);
  const [ms, setMs] = useState(100);

  const [button, setButton] = useState<Button>("left");
  const [action, setAction] = useState<Action>("click");
  const [doubleClick, setDoubleClick] = useState(false);
  const [holdKey, setHoldKey] = useState<string | null>("W");

  const [repeatCount, setRepeatCount] = useState<number | null>(null); // null = infinite
  const [fixed, setFixed] = useState<[number, number] | null>(null);
  const [jitter, setJitter] = useState(0);

  const [advanced, setAdvanced] = useState(false);
  const [phase, setPhase] = useState<Phase>("idle");
  const [hotkey, setHotkey] = useState("F6");
  const [capturing, setCapturing] = useState<"key" | "hotkey" | null>(null);
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

  const actionRef = useRef(action);
  actionRef.current = action;
  const phaseRef = useRef(phase);
  phaseRef.current = phase;

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
      if (capturing === "key") {
        const name = codeToKeyName(e);
        if (name) {
          setHoldKey(name);
          setCapturing(null);
        }
      } else {
        const accel = eventToAccelerator(e);
        if (accel) {
          setHotkey(accel);
          invoke("set_hotkey", { accelerator: accel }).catch(() => {});
          setCapturing(null);
        }
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
    <div className="app" data-phase={phase} data-mode={action}>
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
                  {running ? "press hotkey to stop" : `press ${hotkey} to start`}
                </div>
              </>
            )}
          </div>
        </div>

        {/* primary */}
        <button className="primary" onClick={toggle}>
          {!running && <span className="tri" />}
          {running ? (action === "hold" ? "Release" : "Stop") : "Start"}
          <span className="k">{hotkey}</span>
        </button>

        {/* common options */}
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
            <div className="row">
              <div className="nm">
                Hold key
                <small>press any key, or use the mouse button above</small>
              </div>
              <div className="pick">
                {holdKey && <span className="kc">{holdKey}</span>}
                <button
                  className={"setk" + (capturing === "key" ? " capturing" : "")}
                  onClick={() => setCapturing("key")}
                >
                  {capturing === "key" ? "press a key…" : "Set key"}
                </button>
                {holdKey && (
                  <button className="setk" onClick={() => setHoldKey(null)}>
                    use mouse
                  </button>
                )}
              </div>
            </div>
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

        {/* advanced */}
        <button className="disc" onClick={() => setAdvanced((a) => !a)}>
          Advanced
          <span className="ln" />
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
                <small>toggle start/stop</small>
              </div>
              <div className="pick">
                <span className="kc">{hotkey}</span>
                <button
                  className={"setk" + (capturing === "hotkey" ? " capturing" : "")}
                  onClick={() => setCapturing("hotkey")}
                >
                  {capturing === "hotkey" ? "press keys…" : "Rebind"}
                </button>
              </div>
            </div>
          </div>
        )}

        <div className="foot">
          repeat <b>{repeatCount == null ? "until stopped" : `${repeatCount}x`}</b> · position{" "}
          <b>{fixed ? `${fixed[0]},${fixed[1]}` : "follow cursor"}</b>
        </div>
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
