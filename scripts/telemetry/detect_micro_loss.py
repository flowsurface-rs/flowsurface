#!/usr/bin/env python3
"""Detect microstructure loss from telemetry artifacts.

# GitHub Issue: https://github.com/terrylica/flowsurface/issues/telemetry

Cross-references MicroLoss events with ChartSnapshot events to determine:
- Which bars lost microstructure and never recovered
- What percentage of bars have None microstructure at each snapshot
- Time between microstructure loss and potential recovery

Usage:
    python3 detect_micro_loss.py ~/.local/share/flowsurface/telemetry/rb-*.ndjson
"""
from __future__ import annotations

import json
import sys
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


def analyze(events: list[dict]) -> None:
    micro_loss = [e for e in events if e.get("event") == "MicroLoss"]
    snapshots = [e for e in events if e.get("event") == "ChartSnapshot"]
    chart_opens = [e for e in events if e.get("event") == "ChartOpen"]

    print("=== Microstructure Loss Summary ===")
    print(f"  MicroLoss events:   {len(micro_loss)}")
    print(f"  ChartSnapshot events: {len(snapshots)}")
    print(f"  ChartOpen events:   {len(chart_opens)}")
    print()

    if not micro_loss:
        print("No MicroLoss events detected. Microstructure is preserved.")
        return

    # Track lost bar timestamps
    lost_bars: dict[int, dict] = {}
    for e in micro_loss:
        bar_ts = e["bar_time_ms"]
        if bar_ts not in lost_bars:
            lost_bars[bar_ts] = {
                "first_loss_ts": e["ts_ms"],
                "loss_count": 0,
                "micro": e["micro_before"],
            }
        lost_bars[bar_ts]["loss_count"] += 1

    print("=== Unique Bars That Lost Microstructure ===")
    print(f"  Unique bar timestamps: {len(lost_bars)}")
    print(f"  Total loss events:     {len(micro_loss)}")
    repeat_losers = sum(1 for b in lost_bars.values() if b["loss_count"] > 1)
    print(f"  Bars with repeat loss: {repeat_losers}")
    print()

    # Show details for most-affected bars
    sorted_bars = sorted(lost_bars.items(), key=lambda x: x[1]["loss_count"], reverse=True)
    print("  Top 10 most-replaced bars:")
    for bar_ts, info in sorted_bars[:10]:
        m = info["micro"]
        print(f"    bar_ts={bar_ts}  losses={info['loss_count']}  "
              f"trades={m['trade_count']}  ofi={m['ofi']:.4f}  intensity={m['trade_intensity']:.4f}")
    print()

    # Chart open coverage analysis
    if chart_opens:
        print("=== Microstructure Coverage at Chart Open ===")
        for e in chart_opens:
            bar_count = e["bar_count"]
            micro_count = e["micro_coverage"]
            pct = (micro_count / bar_count * 100) if bar_count > 0 else 0
            print(f"  {e['symbol']}@BPR{e['threshold_dbps']//10}: "
                  f"{micro_count}/{bar_count} bars ({pct:.1f}%) have microstructure")
        print()

    # Snapshot time series (if available)
    if snapshots:
        print("=== ChartSnapshot Time Series ===")
        for s in snapshots[:10]:
            forming = f"forming_ts={s['forming_bar_ts']}" if s.get("forming_bar_ts") else "no forming"
            print(f"  ts={s['ts_ms']}  bars={s['total_bars']}  "
                  f"rbp_completed={s['rbp_completed_count']}  {forming}")
        if len(snapshots) > 10:
            print(f"  ... and {len(snapshots) - 10} more snapshots")


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
    analyze(events)


if __name__ == "__main__":
    main()
