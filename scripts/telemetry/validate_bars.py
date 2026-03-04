#!/usr/bin/env python3
"""Validate live-built bars against ClickHouse bars — the definitive repaint answer.

# GitHub Issue: https://github.com/terrylica/flowsurface/issues/telemetry

Reads ChPollBar (what ClickHouse delivered) and RbpBarComplete (what local
processor built) events from NDJSON artifacts, matches by time_ms, and
compares OHLCV in all representations.

Usage:
    python3 validate_bars.py ~/.local/share/flowsurface/telemetry/rb-*.ndjson
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


def validate(events: list[dict]) -> None:
    ch_bars: dict[int, dict] = {}
    rbp_bars: dict[int, dict] = {}

    for e in events:
        if e.get("event") == "ChPollBar":
            ts = e["kline"]["time_ms"]
            ch_bars[ts] = e
        elif e.get("event") == "RbpBarComplete":
            ts = e["kline"]["time_ms"]
            rbp_bars[ts] = e

    matched_ts = sorted(set(ch_bars) & set(rbp_bars))
    ch_only = sorted(set(ch_bars) - set(rbp_bars))
    rbp_only = sorted(set(rbp_bars) - set(ch_bars))

    print("=== Bar Matching Summary ===")
    print(f"  ChPollBar events:     {len(ch_bars)}")
    print(f"  RbpBarComplete events: {len(rbp_bars)}")
    print(f"  Matched by time_ms:   {len(matched_ts)}")
    print(f"  CH-only (no local):   {len(ch_only)}")
    print(f"  RBP-only (no CH):     {len(rbp_only)}")
    print()

    # Compare matched bars
    discrepancies = []
    for ts in matched_ts:
        ch_k = ch_bars[ts]["kline"]
        rbp_k = rbp_bars[ts]["kline"]

        diffs = {}
        for field in ["open_units", "close_units", "high_units", "low_units",
                       "buy_vol_units", "sell_vol_units"]:
            ch_val = ch_k[field]
            rbp_val = rbp_k[field]
            if ch_val != rbp_val:
                diffs[field] = {"ch": ch_val, "rbp": rbp_val, "delta": abs(ch_val - rbp_val)}

        f32_diffs = {}
        for field in ["open_f32", "close_f32", "high_f32", "low_f32"]:
            ch_val = ch_k[field]
            rbp_val = rbp_k[field]
            if abs(ch_val - rbp_val) > 1e-6:
                f32_diffs[field] = {"ch": ch_val, "rbp": rbp_val, "delta": abs(ch_val - rbp_val)}

        if diffs or f32_diffs:
            discrepancies.append({
                "time_ms": ts,
                "unit_diffs": diffs,
                "f32_diffs": f32_diffs,
            })

    print("=== OHLCV Comparison (matched bars) ===")
    print(f"  Exact matches:    {len(matched_ts) - len(discrepancies)}")
    print(f"  Discrepancies:    {len(discrepancies)}")

    if discrepancies:
        print("\n  Top 10 discrepancies:")
        for d in discrepancies[:10]:
            print(f"\n    bar_ts={d['time_ms']}")
            for field, vals in d["unit_diffs"].items():
                print(f"      {field}: CH={vals['ch']}  RBP={vals['rbp']}  delta={vals['delta']}")
            for field, vals in d["f32_diffs"].items():
                print(f"      {field}: CH={vals['ch']:.8f}  RBP={vals['rbp']:.8f}  delta={vals['delta']:.8f}")

    # Root cause analysis
    if discrepancies:
        print("\n=== Root Cause Analysis ===")
        unit_only = sum(1 for d in discrepancies if d["unit_diffs"] and not d["f32_diffs"])
        f32_only = sum(1 for d in discrepancies if not d["unit_diffs"] and d["f32_diffs"])
        both = sum(1 for d in discrepancies if d["unit_diffs"] and d["f32_diffs"])
        print(f"  Unit-level differences only: {unit_only}")
        print(f"  f32-level differences only (precision): {f32_only}")
        print(f"  Both unit + f32 differences: {both}")
        if f32_only > 0 and unit_only == 0 and both == 0:
            print("  → All differences are f64→f32 precision loss (not logic errors)")
        elif unit_only > 0 or both > 0:
            print("  → Some differences indicate logic-level discrepancies (investigate)")
    else:
        print("\n  All matched bars are bit-identical between CH and RBP.")

    print()


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
    validate(events)


if __name__ == "__main__":
    main()
