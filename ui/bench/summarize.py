"""Summarize completed visible viewer runs; retain exclusions and raw evidence."""
import argparse
from collections import defaultdict
import csv
import json
from pathlib import Path
from statistics import median


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("work", type=Path)
    args = parser.parse_args()
    samples = defaultdict(list)
    rows, excluded, runs = [], [], []
    for path in sorted((args.work / "results").glob("*.json")):
        report = json.loads(path.read_text())
        if report.get("partial"):
            continue
        reason = None
        if report.get("errors") and report.get("results"):
            # runScene only appends a scene after every operation and assertion
            # passed. A later rejected workload does not invalidate those scenes.
            excluded.append({"file": path.name, "scope": "uncompleted later scene", "reason": "; ".join(report["errors"])})
        elif report.get("errors"):
            reason = "harness validation failed: " + "; ".join(report["errors"])
        elif report.get("pilot"):
            reason = "pilot"
        elif any("interaction_frames" not in op for scene in report["results"] for op in scene["operations"] if op["name"] == "zoom-in-out"):
            reason = "preliminary run before separate interaction-frame capture"
        if reason:
            excluded.append({"file": path.name, "reason": reason})
            continue
        runs.append({"file": path.name, "variant": report["variant"], "viewport": report["viewport"], "user_agent": report["user_agent"], "started_utc": report["started_utc"]})
        for scene in report["results"]:
            for op in scene["operations"]:
                if op["start_visibility"] != "visible" or not op["start_focused"] or op["visibility"]:
                    excluded.append({"file": path.name, "scene": scene["scene"]["parts"], "operation": op["name"], "reason": "visibility or focus interruption"})
                    continue
                frame = op.get("interaction_frames")
                row = {
                    "file": path.name, "variant": report["variant"], "parts": scene["scene"]["parts"], "max_zoom": report.get("max_zoom", 8), "repetition": scene["repetition"], "operation": op["name"],
                    "elapsed_ms": op["elapsed_ms"], "commit_ms": op.get("commit_ms"), "api_reply_ms": op.get("api_reply_ms"),
                    "frame_p50_ms": frame["p50_ms"] if frame else None,
                    "frame_p95_ms": frame["p95_ms"] if frame else None,
                    "frame_max_ms": frame["max_ms"] if frame else None,
                    "slow_frame_percent": 100 * frame["over_33ms"] / frame["count"] if frame and frame["count"] else None,
                    "events": op.get("dispatched"), "event_p95_ms": op.get("dispatch", {}).get("p95_ms"),
                    "groups": op["groups"], "contours": op["contours"], "points": op["points"], "lead_ins": op["lead_ins"], "svg_nodes": op["svg_nodes"],
                }
                rows.append(row)
                samples[(row["variant"], row["parts"], row["max_zoom"], row["operation"])].append(row)
    aggregates = []
    for (variant, parts, max_zoom, operation), records in sorted(samples.items()):
        aggregate = {"variant": variant, "parts": parts, "max_zoom": max_zoom, "operation": operation, "runs": len(records)}
        for key in ["elapsed_ms", "commit_ms", "api_reply_ms", "frame_p50_ms", "frame_p95_ms", "frame_max_ms", "slow_frame_percent", "events", "event_p95_ms"]:
            values = [r[key] for r in records if r[key] is not None]
            aggregate[key] = median(values) if values else None
        aggregates.append(aggregate)
    result = {"aggregation": "Median of per-run statistics. Interaction frame samples exclude server commit/preparation time. Slow means over 33.34 ms. Unsupported browser APIs remain null in raw results.", "runs": runs, "excluded": excluded, "aggregates": aggregates}
    (args.work / "summary.json").write_text(json.dumps(result, indent=2) + "\n")
    if rows:
        with (args.work / "measurements.csv").open("w") as stream:
            writer = csv.DictWriter(stream, fieldnames=list(rows[0]))
            writer.writeheader()
            writer.writerows(rows)
    print(json.dumps({"completed_reports": len(runs), "measurements": len(rows), "exclusions": len(excluded)}))


if __name__ == "__main__":
    main()
