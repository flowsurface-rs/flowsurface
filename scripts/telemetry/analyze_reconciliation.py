#!/usr/bin/env python3
"""Analyze reconciliation events from telemetry NDJSON artifacts.

Reads Reconcile/MicroLoss events and reports:
- Repaint detection: REPLACE actions where incoming differs from existing
- Magnitude of OHLCV differences (i64 units and f32)
- Frequency of each reconciliation action
- f64→f32 precision loss from ChPollBar events

Usage:
    python3 analyze_reconciliation.py ~/.local/share/flowsurface/telemetry/rb-*.ndjson
"""
from __future__ import annotations

import json
import sys
from collections import Counter
from pathlib import Path


def load_events(paths: list[Path]) -> list[dict]:
    events = []
    for p in paths:
        with open(p) as f:
            for line in f:
                line = line.strip()
                if line:
                    try:
                        events.append(json.loads(line))
                    except json.JSONDecodeError:
                        pass
    return events


def analyze_reconciliation(events: list[dict]) -> None:
    reconcile = [e for e in events if e.get("event") == "Reconcile"]
    micro_loss = [e for e in events if e.get("event") == "MicroLoss"]
    ch_poll = [e for e in events if e.get("event") == "ChPollBar"]

    if not reconcile:
        print("No Reconcile events found.")
        return

    # Action frequency
    actions = Counter(e["action"] for e in reconcile)
    print("=== Reconciliation Action Frequency ===")
    for action, count in actions.most_common():
        print(f"  {action}: {count}")
    print()

    # Repaint detection: REPLACE where OHLCV differs
    repaints = []
    for e in reconcile:
        if e["action"] != "Replace" or not e.get("existing_last"):
            continue
        inc = e["incoming"]
        ext = e["existing_last"]
        diffs = {}
        for field in ["open_units", "close_units", "high_units", "low_units"]:
            delta = abs(inc[field] - ext[field])
            if delta > 0:
                diffs[field] = delta
        if diffs:
            repaints.append({"ts_ms": e["ts_ms"], "diffs": diffs, "incoming": inc, "existing": ext})

    print("=== Repaint Detection (REPLACE with OHLCV diff) ===")
    print(f"  Total REPLACE actions: {actions.get('Replace', 0)}")
    print(f"  Repaints (OHLCV changed): {len(repaints)}")
    if repaints:
        print("\n  Top 10 largest repaints (by max unit delta):")
        repaints.sort(key=lambda r: max(r["diffs"].values()), reverse=True)
        for r in repaints[:10]:
            max_delta = max(r["diffs"].values())
            fields = ", ".join(f"{k}={v}" for k, v in r["diffs"].items())
            print(f"    bar_ts={r['incoming']['time_ms']}  max_delta={max_delta}  [{fields}]")
    print()

    # f64→f32 precision loss
    if ch_poll:
        print("=== f64→f32 Precision Loss (from ChPollBar) ===")
        print(f"  Total ChPollBar events: {len(ch_poll)}")
        max_drift = 0.0
        drift_count = 0
        for e in ch_poll:
            raw = e.get("raw_f64")
            snap = e.get("kline")
            if not raw or not snap:
                continue
            for f64_key, f32_key in [
                ("open_f64", "open_f32"), ("high_f64", "high_f32"),
                ("low_f64", "low_f32"), ("close_f64", "close_f32"),
            ]:
                f64_val = raw[f64_key]
                f32_val = snap[f32_key]
                drift = abs(f64_val - f32_val)
                if drift > 1e-10:
                    drift_count += 1
                    max_drift = max(max_drift, drift)
        print(f"  Fields with f64→f32 drift: {drift_count}")
        print(f"  Max drift: {max_drift:.12f}")
        print()

    # Microstructure loss
    print("=== Microstructure Loss ===")
    print(f"  MicroLoss events: {len(micro_loss)}")
    if micro_loss:
        for e in micro_loss[:5]:
            m = e["micro_before"]
            print(f"    bar_ts={e['bar_time_ms']}  trades={m['trade_count']}  ofi={m['ofi']:.4f}")
    print()

    # Silent drops
    drops = actions.get("Drop", 0)
    print("=== Silent Drops ===")
    print(f"  Stale bars silently dropped: {drops}")


def main() -> None:
    if len(sys.argv) < 2:
        print(__doc__)
        sys.exit(1)
    paths = [Path(p) for p in sys.argv[1:] if Path(p).exists()]
    if not paths:
        print("No valid files found.")
        sys.exit(1)
    events = load_events(paths)
    print(f"Loaded {len(events)} events from {len(paths)} file(s)\n")
    analyze_reconciliation(events)


if __name__ == "__main__":
    main()
