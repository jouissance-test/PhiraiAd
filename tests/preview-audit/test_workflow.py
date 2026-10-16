"""Analyzer tests. These complement, not replace, actual production-render scans."""
import copy
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("preview_audit", ROOT / "scripts/preview_audit.py")
workflow = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(workflow)


def fixture(first=9.55):
    sample = {"chart_time": first, "quads": [{"ndc": [[-0.1, 1.01], [0.1, 1.01], [0.1, 0.99], [-0.1, 0.99]]}]}
    stats = {"line": 0, "note": 0, "visible_frames": 50, "first_sample": sample,
             "spans": [[first, 9.995]], "max_center_step_per_real_second": 2.0}
    return {"schema_version": 1, "calibration": {"passed": True},
            "metadata": {"chart_sha256": "test-hash", "chart_name": "fixture", "preview_real_seconds": 0.45,
                         "preview_chart_seconds": 0.45, "speed": 1.0, "start_chart_time": 8.0,
                         "end_chart_time": 11.0, "hz_real": 120, "scenario": [],
                         "metrics_scope": "pre_hit_only", "renderer": "production Note::render",
                         "coverage_limitations": ["no pixel readback"], "viewport": [1600, 900],
                         "engine": {"sha256": "fixture-engine"},
                         "resource_pack_manifest": [{"path": "click.png", "bytes": 10, "sha256": "fixture-texture"}]},
            "notes": [{"line": 0, "note": 0, "time": 10.0, "kind": "Click", "fake": False}],
            "solver": [{"line": 0, "note": 0, "plan": "constant", "reason": "solved", "flow": 1.0}],
            "modes": {"baseline": {"notes": [copy.deepcopy(stats)]}, "fixed": {"notes": [copy.deepcopy(stats)]}}}


def cases():
    return {"chart_sha256": "test-hash", "configuration": {"preview_real_seconds": 0.45, "speed": 1.0},
            "notes": [{"id": "click", "line": 0, "kind": "Click", "hit_chart_time": 10.0}]}


class WorkflowTests(unittest.TestCase):
    def test_exact_preview_passes_behavior_check(self):
        report = fixture()
        self.assertTrue(workflow.check_cases(report, cases())["passed"])
        self.assertNotIn("preview_timing_mismatch", workflow.analyze(report)["counts"])

    def test_solved_but_96_ms_preview_is_caught(self):
        report = fixture(9.904)
        summary = workflow.analyze(report)
        self.assertEqual(summary["counts"]["preview_timing_mismatch"], 1)
        self.assertNotIn("solver_fallback", summary["counts"])
        result = workflow.check_cases(report, cases())
        self.assertFalse(result["passed"])
        self.assertAlmostEqual(result["cases"][0]["observed_preview_seconds"], 0.096)

    def test_gate_only_is_supported_but_original_is_a_fallback(self):
        report = fixture()
        report["solver"][0].update(plan="gate_only", reason="gate_only")
        self.assertNotIn("solver_fallback", workflow.analyze(report)["counts"])
        report["solver"][0].update(plan="original", reason="visibility_interval_rejected")
        summary = workflow.analyze(report)
        self.assertEqual(summary["counts"]["solver_fallback"], 1)
        self.assertNotIn("preview_timing_mismatch", summary["counts"])
        self.assertNotIn("fallback_visual_mismatch", summary["counts"])

    def test_original_plan_must_preserve_baseline_visible_behavior(self):
        report = fixture()
        report["solver"][0].update(plan="original", reason="visibility_interval_rejected")
        report["modes"]["fixed"]["notes"][0]["spans"] = [[9.7, 9.995]]
        summary = workflow.analyze(report)
        self.assertEqual(summary["counts"]["fallback_visual_mismatch"], 1)

    def test_behavior_case_can_require_a_supported_plan(self):
        report = fixture()
        required = cases()
        required["notes"][0]["allowed_plans"] = ["constant", "gate_only"]
        self.assertTrue(workflow.check_cases(report, required)["passed"])
        report["solver"][0].update(plan="original", reason="visibility_interval_rejected")
        result = workflow.check_cases(report, required)
        self.assertFalse(result["passed"])
        self.assertEqual(result["cases"][0]["solver_plan"], "original")

    def test_missing_or_failed_calibration_is_not_a_pass(self):
        for value in [None, {}, {"passed": False}]:
            report = fixture()
            report["calibration"] = value or {}
            with self.assertRaisesRegex(ValueError, "calibration"):
                workflow.analyze(report)

    def test_missing_resource_identity_is_not_accepted(self):
        report = fixture()
        report["metadata"]["resource_pack_manifest"] = []
        with self.assertRaisesRegex(ValueError, "identity missing"):
            workflow.analyze(report)

    def test_no_eligible_notes_is_insufficient_not_green(self):
        report = fixture()
        report["notes"][0]["time"] = 12.0
        self.assertEqual(workflow.analyze(report)["coverage_status"], "insufficient_eligible_notes")
        with self.assertRaisesRegex(ValueError, "Insufficient coverage"):
            workflow.check_cases(report, cases())

    def test_censored_first_visibility_cannot_pass_check(self):
        report = fixture(8.0)
        with self.assertRaisesRegex(ValueError, "left-censored"):
            workflow.check_cases(report, cases())

    def test_fake_notes_are_counted_out_not_flagged(self):
        report = fixture(9.99)
        report["notes"][0]["fake"] = True
        self.assertEqual(workflow.analyze(report)["findings"], [])

    def test_invisible_fixed_note_is_detected(self):
        report = fixture()
        report["modes"]["fixed"]["notes"] = []
        summary = workflow.analyze(report)
        self.assertEqual(summary["counts"]["no_fixed_visibility_baseline_visible"], 1)

    def test_active_clock_excludes_pause_and_integrates_rates(self):
        report = fixture()
        report["metadata"]["scenario"] = [{"at": 9.0, "pause_real_seconds": 10, "speed": 0.5}]
        self.assertAlmostEqual(workflow.active_seconds(report, 8.5, 9.5), 1.5)

    def test_scenario_uses_corresponding_solver_segment(self):
        report = fixture(9.8)
        report["metadata"]["scenario"] = [{"at": 9.9, "preview": 0.2}]
        changed = {"line": 0, "note": 0, "plan": "constant", "reason": "solved", "flow": 6.0}
        report["solver"][0].update(plan="original", reason="minimum_not_visible")
        report["solver_segments"] = [{"applied_chart_time": 8.0, "solver": report["solver"]}, {"applied_chart_time": 9.9, "solver": [changed]}]
        # Force a candidate so the selected solver identity is inspectable.
        report["modes"]["fixed"]["notes"][0]["max_center_step_per_real_second"] = 10
        report["modes"]["fixed"]["notes"][0]["spans"] = [[9.8, 9.85], [9.9, 9.995]]
        note = workflow.analyze(report)["findings"][0]
        self.assertEqual(note["flow"], 6.0)
        self.assertEqual(note["solver_plan"], "constant")
        self.assertEqual(note["solver_reason"], "solved")
        self.assertNotIn("solver_fallback", note["flags"])
        self.assertNotIn("visibility_gaps", note["flags"])

    def test_comparison_refuses_changed_sampling_or_resources(self):
        old = workflow.analyze(fixture())
        report = fixture()
        report["metadata"]["hz_real"] = 240
        with self.assertRaisesRegex(ValueError, "sampling"):
            workflow.compare(old, workflow.analyze(report))
        self.assertEqual(workflow.compare(old, old)["added"], [])

    def test_undersampled_case_cannot_pass_with_a_large_tolerance(self):
        report = fixture(9.904)
        report["metadata"]["hz_real"] = 1
        with self.assertRaisesRegex(ValueError, "sampling rate"):
            workflow.check_cases(report, cases())

    def test_empty_coverage_comparison_cannot_look_like_no_regression(self):
        report = fixture()
        report["notes"][0]["time"] = 12.0
        summary = workflow.analyze(report)
        with self.assertRaisesRegex(ValueError, "insufficient coverage"):
            workflow.compare(summary, summary)

    def test_cli_known_failure_has_nonzero_exit(self):
        # Intentionally leave test artifacts in a clearly named temp directory;
        # this task never uses deletion as cleanup.
        root = Path(tempfile.mkdtemp(prefix="phiraiad-preview-workflow-test-"))
        report, case = root / "report.json", root / "cases.json"
        report.write_text(json.dumps(fixture(9.904)), encoding="utf-8")
        case.write_text(json.dumps(cases()), encoding="utf-8")
        process = subprocess.run([sys.executable, str(ROOT / "scripts/preview_audit.py"), "check", str(report), "--cases", str(case)], capture_output=True, text=True)
        self.assertEqual(process.returncode, 1, process.stderr)
        self.assertFalse(json.loads(process.stdout)["passed"])


if __name__ == "__main__":
    unittest.main()
