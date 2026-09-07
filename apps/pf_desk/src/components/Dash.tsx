import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";

import {
  FRAME_EVENT,
  TELEMETRY_ENDED_EVENT,
  gearLabel,
  lapTime,
  type Frame,
} from "../lib/telemetry";

/**
 * The driving screen: one fixed, non-scrolling grid of everything a frame
 * carries, sized for a small monitor above the wheel.
 *
 * Display only — the driver is driving, so there is nothing to click and no
 * navigation. Telemetry is started and stopped from the tray window; this
 * window renders whatever arrives on `telemetry-frame` and says so plainly
 * when nothing does.
 *
 * ponytail: raw channels, no strategy math (fuel to the end, stint projection,
 * live delta against a reference lap). That is boxbox-engine's job and lands as
 * its own port; the one derived number here is the session best, a running
 * minimum of the lap times already seen.
 */

const CORNERS = ["FL", "FR", "RL", "RR"] as const;

function Panel({
  title,
  tag,
  children,
}: {
  title: string;
  tag?: string;
  children: React.ReactNode;
}) {
  return (
    <section className="flex flex-col rounded-xl bg-card p-3 ring-1 ring-border">
      <header className="mb-2 flex items-baseline justify-between gap-2">
        <h2 className="text-[10px] font-semibold tracking-[0.2em] text-muted">{title}</h2>
        {tag && <span className="text-[10px] text-muted">{tag}</span>}
      </header>
      {children}
    </section>
  );
}

/**
 * A labelled 0-1 bar. The track stays visible at zero, so a pedal that is up
 * reads differently from a channel that is missing.
 */
function Bar({ label, v, tone }: { label: string; v: number; tone: string }) {
  return (
    <div className="flex items-center gap-2">
      <span className="w-8 shrink-0 text-[10px] text-muted">{label}</span>
      <div className="h-2 flex-1 overflow-hidden rounded-full bg-border">
        <div className={"h-full " + tone} style={{ width: pct(v) }} />
      </div>
      <span className="w-9 shrink-0 text-right text-[11px] tabular-nums">
        {(v * 100).toFixed(0)}%
      </span>
    </div>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <div className="text-[10px] text-muted">{label}</div>
      <div className="text-lg font-semibold tabular-nums">{value}</div>
    </div>
  );
}

/** Corner readouts, always in the game's own FL/FR/RL/RR order. */
function Corners({
  title,
  values,
  format,
}: {
  title: string;
  values: (number | null)[] | undefined;
  format: (v: number) => string;
}) {
  return (
    <Panel title={title}>
      <div className="grid flex-1 grid-cols-2 gap-1">
        {CORNERS.map((corner, i) => {
          const v = values?.[i];
          return (
            <div key={corner} className="rounded-lg bg-background px-2 py-1">
              <div className="text-[9px] text-muted">{corner}</div>
              <div className="text-sm font-semibold tabular-nums">
                {v == null ? "—" : format(v)}
              </div>
            </div>
          );
        })}
      </div>
    </Panel>
  );
}

const clamp01 = (v: number) => Math.max(0, Math.min(1, v));
const pct = (v: number) => (clamp01(v) * 100).toFixed(1) + "%";
const n = (v: number | null | undefined, digits = 1) =>
  v == null ? "—" : v.toFixed(digits);

export function Dash() {
  const [frame, setFrame] = useState<Frame | null>(null);
  const [ended, setEnded] = useState(false);
  const [age, setAge] = useState(0);
  const [best, setBest] = useState<number | null>(null);
  const arrived = useRef(0);

  useEffect(() => {
    const subs = [
      listen<Frame>(FRAME_EVENT, (e) => {
        arrived.current = Date.now();
        setEnded(false);
        setFrame(e.payload);
        // Tracked here rather than in the source: the best is a property of
        // this sitting, and a fresh dash should start it fresh.
        const last = e.payload.last_lap_time_s;
        if (last != null) setBest((b) => (b == null || last < b ? last : b));
      }),
      listen(TELEMETRY_ENDED_EVENT, () => setEnded(true)),
    ];
    // Age is measured on arrival, not from the frame's own clock — a frame
    // replayed from a recording carries last week's timestamps.
    const t = setInterval(
      () => setAge(arrived.current ? (Date.now() - arrived.current) / 1000 : 0),
      250,
    );
    return () => {
      subs.forEach((s) => void s.then((un) => un()));
      clearInterval(t);
    };
  }, []);

  if (!frame) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-2 p-8 text-center">
        <p className="text-sm text-muted">
          {ended ? "Le Mans Ultimate closed." : "Waiting for Le Mans Ultimate…"}
        </p>
        <p className="text-xs text-muted">
          Start telemetry from the ParcFerme window in the tray.
        </p>
      </div>
    );
  }

  const f = frame;
  const stale = age > 2;
  const fuelPct = f.fuel_capacity_l > 0 ? f.fuel_l / f.fuel_capacity_l : 0;

  return (
    <div className="flex h-full flex-col gap-2 overflow-hidden p-2">
      <header className="flex items-baseline justify-between px-1 text-[11px]">
        <span className="font-semibold tracking-[0.2em] text-primary">DASH</span>
        <span className="truncate text-muted">{f.rest.session_name ?? "—"}</span>
        <span className={stale ? "text-destructive" : "text-success"}>
          {ended ? "GAME CLOSED" : stale ? "NO DATA " + age.toFixed(0) + "s" : "LIVE"}
        </span>
      </header>

      <div className="grid flex-1 grid-cols-4 grid-rows-2 gap-2">
        <Panel
          title="SPEED"
          tag={f.in_pit_lane ? "PIT LANE" : f.lap_invalidated ? "LAP INVALID" : undefined}
        >
          <div className="flex flex-1 items-center justify-between gap-2">
            <div className="text-5xl font-bold tabular-nums">{f.speed_kph.toFixed(0)}</div>
            <div className="text-4xl font-bold text-primary">{gearLabel(f.gear)}</div>
          </div>
          <div className="mt-1 flex justify-between text-[11px] tabular-nums text-muted">
            <span>{f.rpm.toFixed(0)} rpm</span>
            <span>
              {n(f.g_lon, 2)} g lon · {n(f.g_lat, 2)} g lat
            </span>
          </div>
        </Panel>

        <Panel title="LAP" tag={f.sector ? "S" + f.sector : undefined}>
          <div className="text-3xl font-semibold tabular-nums">{lapTime(f.lap_time_s)}</div>
          <div className="mt-auto grid grid-cols-3 gap-1 text-[11px] tabular-nums">
            <div>
              <div className="text-[9px] text-muted">LAP</div>
              {f.lap}
            </div>
            <div>
              <div className="text-[9px] text-muted">LAST</div>
              {lapTime(f.last_lap_time_s)}
            </div>
            <div>
              <div className="text-[9px] text-muted">BEST</div>
              <span className="text-primary">{lapTime(best)}</span>
            </div>
          </div>
        </Panel>

        <Panel title="FUEL" tag={f.fuel_capacity_l.toFixed(0) + " L tank"}>
          <div className="flex flex-1 items-baseline gap-2">
            <span className="text-4xl font-semibold tabular-nums">{f.fuel_l.toFixed(1)}</span>
            <span className="text-xs text-muted">L</span>
          </div>
          <div className="space-y-1">
            <Bar label="TANK" v={fuelPct} tone="bg-primary" />
            {f.virtual_energy != null && (
              <Bar label="VE" v={f.virtual_energy} tone="bg-success" />
            )}
          </div>
        </Panel>

        <Panel title="FIELD" tag={f.car_class || undefined}>
          <div className="flex flex-1 items-center gap-6">
            <Stat label="OVERALL" value={"P" + f.place} />
            <Stat
              label="IN CLASS"
              value={f.place_in_class == null ? "—" : "P" + f.place_in_class}
            />
          </div>
          <div className="flex justify-between text-[11px] tabular-nums text-muted">
            <span>ahead {n(f.gap_ahead_s)}s</span>
            <span>behind {n(f.gap_behind_s)}s</span>
          </div>
        </Panel>

        <Panel title="INPUTS">
          <div className="flex-1 space-y-2">
            <Bar label="THR" v={f.throttle} tone="bg-success" />
            <Bar label="BRK" v={f.brake} tone="bg-destructive" />
            {/* Steering is signed, so it grows out of the centre, not the left. */}
            <div className="flex items-center gap-2">
              <span className="w-8 shrink-0 text-[10px] text-muted">STR</span>
              <div className="relative h-2 flex-1 rounded-full bg-border">
                <div
                  className="absolute h-full bg-primary"
                  style={{
                    left: f.steering < 0 ? pct(0.5 + Math.max(-1, f.steering) / 2) : "50%",
                    width: pct(Math.min(1, Math.abs(f.steering)) / 2),
                  }}
                />
              </div>
              <span className="w-9 shrink-0 text-right text-[11px] tabular-nums">
                {(f.steering * 100).toFixed(0)}
              </span>
            </div>
          </div>
        </Panel>

        <Corners title="TYRE °C" values={f.tyre_temp_c} format={(v) => v.toFixed(0) + "°"} />
        <Corners
          title="TYRE LIFE"
          values={f.tyre_life}
          format={(v) => (v * 100).toFixed(0) + "%"}
        />

        <Panel
          title="CONDITIONS"
          tag={f.rest_stale ? "STALE" : f.rest_age_ms == null ? "NO REST" : undefined}
        >
          <div className="grid flex-1 grid-cols-2 content-center gap-1 text-[11px] tabular-nums">
            <span className="text-muted">air</span>
            <span>{n(f.rest.air_c)} °C</span>
            <span className="text-muted">track</span>
            <span>{n(f.rest.track_c)} °C</span>
            <span className="text-muted">rain</span>
            <span>{f.rest.rain == null ? "—" : (f.rest.rain * 100).toFixed(0) + "%"}</span>
            <span className="text-muted">remaining</span>
            <span>
              {f.rest.time_remaining_s == null
                ? "—"
                : Math.floor(f.rest.time_remaining_s / 60) + " min"}
            </span>
          </div>
        </Panel>
      </div>
    </div>
  );
}
