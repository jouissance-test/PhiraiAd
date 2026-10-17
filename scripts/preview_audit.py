#!/usr/bin/env python3
"""Local chart corpus runner and deterministic analysis of production render traces.

No third-party Python packages. Source charts stay outside Git; never delete or
overwrite a run directory. Suspect metrics are not automatically gameplay bugs.
"""
import argparse
import collections
import csv
import datetime
import hashlib
import json
import math
from pathlib import Path
import subprocess
import sys
import time
import zipfile


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def save_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("x", encoding="utf-8") as stream:
        json.dump(value, stream, ensure_ascii=False, indent=2, allow_nan=False)
        stream.write("\n")


def corpus_index(root):
    charts = []
    for path in sorted(root.iterdir()):
        if path.suffix.lower() not in (".zip", ".pez") or not path.is_file():
            continue
        with zipfile.ZipFile(path) as archive:
            # Validate every archive member's CRC without extracting user data.
            bad = archive.testzip()
            if bad:
                raise ValueError(f"Corrupt chart member: {path.name}: {bad}")
            info = next((n for n in archive.namelist() if n.endswith("info.yml")), None)
            if not info:
                raise ValueError(f"No info.yml in {path.name}")
        charts.append({"id": path.stem, "file": path.name, "sha256": digest(path), "bytes": path.stat().st_size})
    if not charts:
        raise ValueError("No chart ZIP/PEZ files found")
    return {"schema_version": 1, "charts": charts}


def note_key(note):
    return note["line"], note["note"]


def active_seconds(report, start, end):
    """Integrate chart time / playback rate, excluding intentional pauses."""
    speed = float(report["metadata"]["speed"])
    total = 0.0
    cursor = start
    for event in report.get("metadata", {}).get("scenario", []):
        at = float(event["at"])
        if at <= start:
            speed = float(event.get("speed") or speed)
        elif at < end:
            total += (at - cursor) / speed
            cursor = at
            speed = float(event.get("speed") or speed)
    return total + (end - cursor) / speed


def validate_report(report):
    if report.get("schema_version") != 1:
        raise ValueError("Unsupported audit report schema")
    if report.get("calibration", {}).get("passed") is not True:
        raise ValueError("Production-render calibration missing or failed")
    metadata = report["metadata"]
    if not metadata.get("metrics_scope", "").startswith("pre_hit_only"):
        raise ValueError("Pre-hit metric scope is required")
    if "Note::render" not in metadata.get("renderer", ""):
        raise ValueError("Production renderer identity missing")
    if not metadata.get("coverage_limitations"):
        raise ValueError("Report coverage statement missing")
    if not metadata.get("engine", {}).get("sha256") or not metadata.get("resource_pack_manifest"):
        raise ValueError("Engine/resource identity missing")
    if metadata.get("scenario") and not report.get("solver_segments"):
        raise ValueError("Scenario report requires per-setting solver snapshots")


def span_gaps(spans):
    return [[left[1], right[0]] for left, right in zip(spans, spans[1:])]


def interval_covered(interval, covers, tolerance):
    start, end = interval
    return any(float(cover[0]) <= start + tolerance and float(cover[1]) >= end - tolerance for cover in covers)


def analyze(report, tolerance=0.020):
    validate_report(report)
    meta = report["metadata"]
    hz = float(meta["hz_real"])
    if not math.isfinite(hz) or hz <= 0:
        raise ValueError("Invalid sampling rate")
    margin = max(tolerance, 2.0 / hz)
    start, end = float(meta["start_chart_time"]), float(meta["end_chart_time"])
    solver = {note_key(n): n for n in report["solver"]}
    segments = [{**segment, "by_note": {note_key(n): n for n in segment["solver"]}} for segment in report.get("solver_segments", [])]
    modes = {name: {note_key(n): n for n in report["modes"][name]["notes"]} for name in ("baseline", "fixed")}
    findings = []
    counts = collections.Counter()
    near62 = []
    for note in report["notes"]:
        hit = float(note["time"])
        if not start <= hit <= end or note.get("fake"):
            continue
        key = note_key(note)
        state = solver[key]
        base = modes["baseline"].get(key, {})
        fixed = modes["fixed"].get(key, {})
        preview = float(meta["preview_real_seconds"])
        speed = float(meta["speed"])
        transition = False
        for segment in segments:
            if float(segment["applied_chart_time"]) <= hit:
                state = segment["by_note"][key]
        for event in meta.get("scenario", []):
            if float(event["at"]) <= hit:
                preview = float(event.get("preview") or preview)
                speed = float(event.get("speed") or speed)
                # Do not declare a steady-state contract failure across a setting edit.
                if hit - float(event["at"]) <= preview * max(speed, 2.0) + margin * speed:
                    transition = True
        if hit - preview * speed < start - margin * speed:
            counts["left_censored"] += 1
            continue
        counts["eligible_notes"] += 1
        plan = state.get("plan")
        supported_plan = plan in ("constant", "gate_only") if plan is not None else state["reason"] == "solved"
        item = {"line": key[0], "note": key[1], "hit_chart_time": hit, "kind": note["kind"],
                "solver_plan": plan or ("constant" if state["reason"] == "solved" else "legacy_fallback"),
                "solver_reason": state["reason"], "flow": state["flow"],
                "target_preview_real_seconds": preview, "transition_window": transition,
                "baseline_visible_frames": base.get("visible_frames", 0), "fixed_visible_frames": fixed.get("visible_frames", 0),
                "flags": [], "measurements": {}}
        flags, metrics = item["flags"], item["measurements"]
        authored_visibility = state.get("authored_visibility_audit") or {}
        authored_hidden_spans = authored_visibility.get("hidden_spans") or []
        chart_margin = margin * speed
        metrics["authored_hidden_samples"] = int(authored_visibility.get("hidden_samples", 0))
        if not transition and not supported_plan:
            flags.append("solver_fallback")
            # Fixed Preview deliberately makes no timing promise for Original.
            # Its contract is stricter in another direction: visible production
            # behavior must stay identical to baseline. Ignore submission-only
            # counters because disabling aggressive culling may submit the same
            # off-screen geometry one frame earlier without changing visibility.
            fallback_visual_keys = (
                "visible_frames", "first_sample", "last_sample", "spans", "spans_real",
                "longest_contiguous_visible_real_seconds", "max_center_step_ndc",
                "max_center_step_per_real_second", "max_step_samples", "max_speed_samples",
                "topology_changes",
            )
            if any(base.get(name) != fixed.get(name) for name in fallback_visual_keys):
                flags.append("fallback_visual_mismatch")
        if fixed.get("first_sample"):
            first = fixed["first_sample"]["chart_time"]
            observed = active_seconds(report, first, hit)
            metrics["first_visible_to_hit_active_seconds"] = observed
            metrics["first_visible_error_seconds"] = observed - preview
            first_censored = first <= start + speed / hz * 0.1
            if first_censored:
                flags.append("first_visibility_left_censored")
            if supported_plan and not transition and not first_censored and abs(observed - preview) > margin:
                target = float(state.get("target_chart_time", hit - preview * speed))
                if interval_covered([target, first], authored_hidden_spans, chart_margin):
                    flags.append("authored_preview_constraint")
                else:
                    flags.append("preview_timing_mismatch")
            spans = fixed.get("spans", [])
            if supported_plan and not transition and len(spans) > 1:
                gaps = span_gaps(spans)
                material_gaps = [gap for gap in gaps if active_seconds(report, gap[0], gap[1]) > margin]
                if len(material_gaps) != len(gaps):
                    flags.append("sub_tolerance_visibility_gap")
                authored_gaps = [gap for gap in material_gaps if interval_covered(gap, authored_hidden_spans, chart_margin)]
                if authored_gaps:
                    flags.append("authored_visibility_gaps")
                if len(authored_gaps) != len(material_gaps):
                    flags.append("visibility_gaps")
            if spans:
                last_start, last_end = spans[-1]
                metrics["last_visible_span_active_seconds"] = active_seconds(report, last_start, last_end)
                gap = active_seconds(report, last_end, hit)
                metrics["last_visible_to_hit_gap_seconds"] = gap
                if supported_plan and not transition and gap > margin:
                    if interval_covered([last_end, hit], authored_hidden_spans, chart_margin):
                        flags.append("authored_near_hit_gap")
                    else:
                        flags.append("not_visible_near_hit")
            # A first visible quad wholly inside the viewport indicates a pop-in
            # candidate, not proof: authored alpha events can intentionally do this.
            quads = fixed["first_sample"].get("quads", [])
            if supported_plan and quads and all(abs(v) < 0.97 for q in quads for p in q["ndc"] for v in p):
                flags.append("screen_interior_appearance_candidate")
        elif supported_plan and base.get("visible_frames", 0):
            flags.append("no_fixed_visibility_baseline_visible")
        elif supported_plan:
            flags.append("both_modes_unobserved")
        base_speed = float(base.get("max_center_step_per_real_second", 0.0))
        fixed_speed = float(fixed.get("max_center_step_per_real_second", 0.0))
        metrics.update({"baseline_peak_center_speed_ndc_s": base_speed, "fixed_peak_center_speed_ndc_s": fixed_speed})
        if supported_plan and base_speed > 1e-7:
            ratio = fixed_speed / base_speed
            metrics["peak_center_speed_ratio"] = ratio
            if ratio > 2.0:
                flags.append("speed_amplification_candidate")
        # Prefer pre-hit gaps and timing failures; speed ratios only rank suspects.
        item["score"] = (100 if "no_fixed_visibility_baseline_visible" in flags else 0) + 50 * abs(metrics.get("first_visible_error_seconds", 0.0)) / max(preview, margin) + min(metrics.get("peak_center_speed_ratio", 0), 20)
        for flag in flags:
            counts[flag] += 1
        if flags:
            findings.append(item)
        if 61.0 <= hit <= 63.0:
            near62.append(item)
    findings.sort(key=lambda n: (-n["score"], n["hit_chart_time"], n["line"], n["note"]))
    identity = {key: meta.get(key) for key in ("viewport", "chart_viewport", "effective_aspect_ratio", "hz_real", "start_chart_time", "end_chart_time", "scenario", "note_width", "baseline_note_flow_speed", "metrics_scope")}
    identity["resource_pack"] = sorted((Path(item["path"]).name, item["bytes"], item["sha256"]) for item in meta.get("resource_pack_manifest", []))
    return {"schema_version": 1, "chart_sha256": meta["chart_sha256"], "chart_name": meta["chart_name"],
            "preview": meta["preview_real_seconds"], "speed": meta["speed"], "sampling_error_seconds": margin,
            "comparison_identity": identity, "coverage_status": "measured" if counts["eligible_notes"] else "insufficient_eligible_notes",
            "counts": dict(counts), "findings": findings, "focus_61_to_63": near62,
            "interpretation": "Flags are measurable contract deviations or investigation candidates, not automatically confirmed bugs. Pauses excluded from active-read-time metrics. Texture alpha, shaders and occlusion are not pixel-verified.",
            "coverage_limitations": meta.get("coverage_limitations", [])}


def fingerprint(summary, item, flag):
    # Scoped to an exact chart and configuration. Parser-order changes require
    # explicit rebaselining; do not silently compare different corpus hashes.
    return (summary["chart_sha256"], str(summary["preview"]), str(summary["speed"]), str(item["line"]), str(item["note"]), str(item["hit_chart_time"]), item["kind"].split(" {")[0], flag)


def compare(previous, current):
    if previous.get("coverage_status") != "measured" or current.get("coverage_status") != "measured":
        raise ValueError("Cannot compare insufficient coverage as a passing regression")
    old = {fingerprint(previous, n, flag) for n in previous["findings"] for flag in n["flags"]}
    new = {fingerprint(current, n, flag) for n in current["findings"] for flag in n["flags"]}
    comparable = all(previous[k] == current[k] for k in ("chart_sha256", "preview", "speed", "comparison_identity"))
    if not comparable:
        raise ValueError("Cannot compare changed corpus/configuration/resources/sampling bounds")
    return {"added": sorted(new - old), "resolved": sorted(old - new), "unchanged": len(new & old)}


def check_cases(report, cases):
    """Exit 1 is a still-failing behavioral regression; missing evidence is exit 2."""
    summary = analyze(report)
    if report["metadata"]["hz_real"] < cases.get("minimum_hz", 120):
        raise ValueError("Insufficient sampling rate for this behavioral case")
    if summary["coverage_status"] != "measured":
        raise ValueError("Insufficient coverage: no eligible notes")
    for key, value in cases["configuration"].items():
        if report["metadata"].get(key) != value:
            raise ValueError(f"Case configuration does not match: {key}")
    if cases["chart_sha256"] != report["metadata"]["chart_sha256"]:
        raise ValueError("Case corpus hash does not match")
    if report["metadata"].get("scenario"):
        raise ValueError("Steady-state regression cases cannot use a scenario trace")
    stats = {note_key(n): n for n in report["modes"]["fixed"]["notes"]}
    solver = {note_key(n): n for n in report["solver"]}
    results = []
    for case in cases["notes"]:
        metadata = next((n for n in report["notes"] if n["line"] == case["line"] and abs(n["time"] - case["hit_chart_time"]) < 1e-6 and n["kind"].split(" {")[0] == case["kind"]), None)
        if metadata is None:
            raise ValueError(f"Regression note not found: {case['id']}")
        state = stats.get(note_key(metadata), {})
        hit = metadata["time"]
        meta = report["metadata"]
        if hit > meta["end_chart_time"] or hit - meta["preview_chart_seconds"] < meta["start_chart_time"]:
            raise ValueError(f"Insufficient case scan bounds: {case['id']}")
        first = state.get("first_sample")
        if first is not None and first["chart_time"] <= meta["start_chart_time"] + 1e-7:
            raise ValueError(f"Case first visibility is left-censored: {case['id']}")
        duration = active_seconds(report, first["chart_time"], hit) if first else None
        error = duration - meta["preview_real_seconds"] if duration is not None else None
        solver_state = solver[note_key(metadata)]
        plan = solver_state.get("plan")
        allowed_plans = case.get("allowed_plans")
        plan_passed = allowed_plans is None or plan in allowed_plans
        passed = error is not None and abs(error) <= summary["sampling_error_seconds"] and plan_passed
        results.append({"id": case["id"], "passed": passed, "observed_preview_seconds": duration,
                        "target_preview_seconds": meta["preview_real_seconds"], "error_seconds": error,
                        "solver_plan": plan, "solver_reason": solver_state["reason"],
                        "allowed_plans": allowed_plans, "line": metadata["line"], "note": metadata["note"]})
    if not results:
        raise ValueError("Regression cases are empty")
    return {"passed": all(n["passed"] for n in results), "cases": results}


def write_summary(path, summary):
    save_json(path, summary)
    with path.with_suffix(".csv").open("x", encoding="utf-8-sig", newline="") as stream:
        writer = csv.writer(stream)
        writer.writerow(["line", "note", "hit_chart_time", "kind", "solver_plan", "solver_reason", "flow", "flags", "preview_error_seconds", "speed_ratio"])
        for n in summary["findings"]:
            writer.writerow([n["line"], n["note"], n["hit_chart_time"], n["kind"], n["solver_plan"], n["solver_reason"], n["flow"], ";".join(n["flags"]), n["measurements"].get("first_visible_error_seconds"), n["measurements"].get("peak_center_speed_ratio")])


def triage_run(directory):
    run = json.loads((directory / "run.json").read_text(encoding="utf-8"))
    groups = {}
    for job in run["jobs"]:
        if job.get("error"):
            continue
        path = directory / (job["id"] + ".findings.json")
        summary = json.loads(path.read_text(encoding="utf-8"))
        for note in summary["findings"]:
            for flag in note["flags"]:
                signature = (flag, note["solver_plan"], note["solver_reason"], note["kind"].split(" {")[0])
                group = groups.setdefault(signature, {"flag": flag, "solver_plan": signature[1], "solver_reason": signature[2], "kind": signature[3], "occurrences": 0, "unique_notes": set(), "examples": []})
                group["occurrences"] += 1
                group["unique_notes"].add((summary["chart_sha256"], note["line"], note["note"]))
                example = {"job": job["id"], "chart": summary["chart_name"], "hit_chart_time": note["hit_chart_time"], "line": note["line"], "note": note["note"], "flow": note["flow"], "score": note["score"], "measurements": note["measurements"], "findings_file": str(path.resolve())}
                group["examples"].append(example)
                group["examples"] = sorted(group["examples"], key=lambda n: -n["score"])[:3]
    result = []
    for group in groups.values():
        group["unique_notes"] = len(group["unique_notes"])
        result.append(group)
    return {"source_run": str(directory.resolve()), "engine_failures": run["engine_failures"],
            "interpretation": "Groups classify measured symptoms, NOT proven root causes. Occurrences include repeated configurations; unique_notes deduplicates exact chart/line/note across settings.",
            "groups": sorted(result, key=lambda g: (-g["unique_notes"], g["flag"]))}


CONTRACT_FAILURE_FLAGS = (
    "preview_timing_mismatch",
    "visibility_gaps",
    "not_visible_near_hit",
    "no_fixed_visibility_baseline_visible",
    "fallback_visual_mismatch",
)


def gate_run(path, minimum_supported_coverage=0.90):
    if not math.isfinite(minimum_supported_coverage) or not 0.0 <= minimum_supported_coverage <= 1.0:
        raise ValueError("Minimum supported coverage must be in 0..1")
    run = json.loads(path.read_text(encoding="utf-8"))
    results = []
    passed = run.get("engine_failures") == 0
    for job in run.get("jobs", []):
        counts = job.get("counts") or {}
        eligible = int(counts.get("eligible_notes", 0))
        fallback = int(counts.get("solver_fallback", 0))
        coverage = (eligible - fallback) / eligible if eligible else None
        scenario = job["id"].endswith("-scenario")
        failures = {flag: int(counts.get(flag, 0)) for flag in CONTRACT_FAILURE_FLAGS if counts.get(flag, 0)}
        job_passed = not job.get("error") and not failures and (scenario or coverage is not None and coverage >= minimum_supported_coverage)
        passed &= job_passed
        results.append({
            "id": job["id"],
            "passed": job_passed,
            "scenario": scenario,
            "eligible_notes": eligible,
            "solver_fallback": fallback,
            "supported_coverage": coverage,
            "contract_failures": failures,
            "error": job.get("error"),
        })
    if not results:
        raise ValueError("Run contains no jobs")
    return {"passed": bool(passed), "minimum_supported_coverage": minimum_supported_coverage, "jobs": results}


def engine_path(path):
    text = str(path.resolve())
    if sys.platform == "win32":
        # Forward slashes avoid shell/backslash consumption in wsl.exe's Windows
        # command-line boundary. --exec bypasses the Linux default shell.
        return subprocess.check_output(["wsl.exe", "-d", "UbuntuD", "--exec", "wslpath", "-a", text.replace("\\", "/")], text=True).strip()
    return text


def run_batch(args):
    root, output = args.corpus.resolve(), args.out.resolve()
    if output.exists():
        raise ValueError("Output directory already exists; select a new run directory (no overwrite)")
    if len(set(args.previews)) != len(args.previews) or any(not math.isfinite(p) or not 0.1 <= p <= 0.75 for p in args.previews):
        raise ValueError("Preview values must be unique and in 0.1..0.75 seconds")
    output.mkdir(parents=True)
    manifest = corpus_index(root)
    save_json(output / "corpus.json", manifest)
    engine = engine_path(args.engine)
    engine_hash = digest(args.engine)
    prefix = ["wsl.exe", "-d", "UbuntuD", "--exec"] if sys.platform == "win32" else []
    jobs = []
    for chart in manifest["charts"]:
        if args.only and chart["id"] not in args.only:
            continue
        variants = [(p, None) for p in args.previews]
        if args.scenario and (not args.scenario_chart or chart["id"] == args.scenario_chart):
            variants.append((0.45, args.scenario))
        for preview, scenario in variants:
            name = f"{chart['id']}-p{preview:.3f}" + ("-scenario" if scenario else "")
            raw = output / (name + ".raw.json")
            command = prefix + [engine, "--chart", engine_path(root / chart["file"]), "--out", engine_path(raw), "--preview", str(preview), "--speed", str(args.speed), "--hz", str(args.hz)]
            if args.start is not None:
                command += ["--start", str(args.start)]
            if args.end is not None:
                command += ["--end", str(args.end)]
            if scenario:
                command += ["--scenario", engine_path(scenario)]
            print(f"Scanning {name}", flush=True)
            began = time.monotonic()
            with (output / (name + ".log")).open("x", encoding="utf-8") as log:
                try:
                    result = subprocess.run(command, stdout=log, stderr=subprocess.STDOUT, timeout=args.timeout, check=False)
                    code = result.returncode
                    error = None
                except subprocess.TimeoutExpired:
                    code, error = 124, "engine_timeout"
            job = {"id": name, "chart": chart, "exit_code": code, "elapsed_seconds": time.monotonic() - began, "error": error}
            if code == 0 and raw.exists():
                try:
                    with raw.open(encoding="utf-8") as stream:
                        report = json.load(stream)
                    validate_report(report)
                    if report["metadata"].get("engine", {}).get("sha256") != engine_hash:
                        raise ValueError("Engine binary identity changed or does not match report")
                    if report["metadata"]["chart_sha256"] != chart["sha256"]:
                        raise ValueError("Engine chart hash differs from indexed corpus")
                    summary = analyze(report)
                    write_summary(output / (name + ".findings.json"), summary)
                    job["counts"] = summary["counts"]
                    job["coverage_status"] = summary["coverage_status"]
                    if summary["coverage_status"] != "measured":
                        job["error"] = "insufficient_eligible_notes"
                except (ValueError, KeyError, TypeError, OSError) as failure:
                    job["error"] = f"report_validation_failed: {failure}"
            else:
                job["error"] = error or "engine_failed_or_missing_report"
            jobs.append(job)
            save_json(output / (name + ".job.json"), job)
    if not jobs:
        raise ValueError("No matching charts selected")
    try:
        revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=Path(__file__).resolve().parent.parent, text=True).strip()
    except (OSError, subprocess.CalledProcessError):
        revision = None
    index = {"schema_version": 1, "created_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(), "source_revision": revision,
             "engine_sha256": engine_hash, "runner_sha256": digest(Path(__file__).resolve()), "jobs": jobs, "engine_failures": sum(bool(j["error"]) for j in jobs)}
    save_json(output / "run.json", index)
    rows = ["# Fixed Preview corpus audit", "", f"Source revision: `{revision}`", "", "Suspect flags require classification; zero process failures is not a zero-bug verdict.", "", "| Job | Seconds | Engine status | Flags |", "|---|---:|---|---|"]
    for job in jobs:
        rows.append(f"| {job['id']} | {job['elapsed_seconds']:.2f} | {job['error'] or 'completed'} | {json.dumps(job.get('counts', {}), ensure_ascii=False)} |")
    (output / "README.md").write_text("\n".join(rows) + "\n", encoding="utf-8")
    print(json.dumps({"output": str(output), "jobs": len(jobs), "engine_failures": index["engine_failures"]}, ensure_ascii=False))
    return bool(index["engine_failures"])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    run = commands.add_parser("run")
    run.add_argument("--corpus", type=Path, required=True)
    run.add_argument("--out", type=Path, required=True)
    run.add_argument("--engine", type=Path, required=True)
    run.add_argument("--previews", type=float, nargs="+", default=[0.2, 0.45, 0.75])
    run.add_argument("--speed", type=float, default=1.0)
    run.add_argument("--hz", type=float, default=120.0)
    run.add_argument("--start", type=float)
    run.add_argument("--end", type=float)
    run.add_argument("--only", nargs="+")
    run.add_argument("--scenario", type=Path)
    run.add_argument("--scenario-chart", default="waking-shadows")
    run.add_argument("--timeout", type=float, default=600)
    summary = commands.add_parser("summarize")
    summary.add_argument("report", type=Path)
    summary.add_argument("--out", type=Path, required=True)
    diff = commands.add_parser("compare")
    diff.add_argument("previous", type=Path)
    diff.add_argument("current", type=Path)
    check = commands.add_parser("check")
    check.add_argument("report", type=Path)
    check.add_argument("--cases", type=Path, required=True)
    triage = commands.add_parser("triage")
    triage.add_argument("run", type=Path)
    triage.add_argument("--out", type=Path, required=True)
    gate = commands.add_parser("gate")
    gate.add_argument("run", type=Path)
    gate.add_argument("--minimum-supported-coverage", type=float, default=0.90)
    args = parser.parse_args()
    try:
        if args.command == "run":
            return run_batch(args)
        if args.command == "summarize":
            with args.report.open(encoding="utf-8") as stream:
                summary = analyze(json.load(stream))
            write_summary(args.out, summary)
            print(json.dumps(summary["counts"], ensure_ascii=False))
        elif args.command == "check":
            result = check_cases(json.loads(args.report.read_text(encoding="utf-8")), json.loads(args.cases.read_text(encoding="utf-8")))
            print(json.dumps(result, ensure_ascii=False, indent=2))
            return 0 if result["passed"] else 1
        elif args.command == "triage":
            result = triage_run(args.run)
            save_json(args.out, result)
            rows = ["# Candidate clusters", "", result["interpretation"], "", "| Flag | Plan | Reason | Kind | Unique Notes | Occurrences | Representative chart / time |", "|---|---|---|---|---:|---:|---|"]
            for group in result["groups"]:
                examples = "; ".join(f"{n['chart']} / {n['hit_chart_time']:.3f}s (L{n['line']}:N{n['note']})" for n in group["examples"])
                rows.append(f"| {group['flag']} | {group['solver_plan']} | {group['solver_reason']} | {group['kind']} | {group['unique_notes']} | {group['occurrences']} | {examples} |")
            with args.out.with_suffix(".md").open("x", encoding="utf-8") as stream:
                stream.write("\n".join(rows) + "\n")
            print(json.dumps({"groups": len(result["groups"]), "out": str(args.out)}, ensure_ascii=False))
        elif args.command == "gate":
            result = gate_run(args.run, args.minimum_supported_coverage)
            print(json.dumps(result, ensure_ascii=False, indent=2))
            return 0 if result["passed"] else 1
        else:
            print(json.dumps(compare(json.loads(args.previous.read_text(encoding="utf-8")), json.loads(args.current.read_text(encoding="utf-8"))), ensure_ascii=False, indent=2))
        return 0
    except (ValueError, KeyError, OSError, zipfile.BadZipFile, subprocess.CalledProcessError) as error:
        print(f"Audit workflow error: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
