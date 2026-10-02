import unittest
from pathlib import Path

from src.solana_attack_simulator import convert_case, detailed_report, load_cases, synthetic_address


class TestSolanaSimulator(unittest.TestCase):
    def test_blacklist_address_is_valid_base58_shape(self):
        address = synthetic_address(1)
        self.assertEqual(len(address), 32)
        self.assertNotIn("0", address)
        self.assertNotIn("O", address)

    def test_converts_existing_balanced_cases(self):
        source = Path(__file__).parents[1] / "results" / "synthetic_cases.jsonl"
        cases = load_cases(source, 200, [synthetic_address(0)])
        self.assertEqual(len(cases), 200)
        self.assertEqual(sum(case["expected_decision"] == "BLOCK" for case in cases), 100)
        self.assertEqual(sum(case["expected_decision"] == "ALLOW" for case in cases), 100)
        kinds = {case["attack_type"] for case in cases}
        self.assertTrue({"blacklist", "prompt_injection", "ai_intent_mismatch",
                         "simulation_revert"}.issubset(kinds))
        self.assertTrue(any(kind.startswith("benign_") for kind in kinds))
        self.assertTrue(all("expected_decision" not in case["payload"] for case in cases))
        self.assertTrue({"development", "holdout"}.issubset({case["split"] for case in cases}))

    def test_generation_is_reproducible_only_with_same_seed(self):
        source = Path(__file__).parents[1] / "results" / "synthetic_cases.jsonl"
        blacklist = [synthetic_address(0)]
        first = load_cases(source, 40, blacklist, seed=123)
        repeated = load_cases(source, 40, blacklist, seed=123)
        changed = load_cases(source, 40, blacklist, seed=456)
        self.assertEqual(first, repeated)
        self.assertNotEqual(first, changed)

    def test_metamorphic_generation_preserves_expected_decision(self):
        source = Path(__file__).parents[1] / "results" / "synthetic_cases.jsonl"
        cases = load_cases(source, 10, [synthetic_address(0)], seed=7, metamorphic_rate=1.0)
        originals = {case["case_id"]: case for case in cases if "metamorphic_parent" not in case}
        siblings = [case for case in cases if "metamorphic_parent" in case]
        self.assertEqual(len(siblings), 10)
        for sibling in siblings:
            parent = originals[sibling["metamorphic_parent"]]
            self.assertEqual(sibling["expected_decision"], parent["expected_decision"])
            self.assertNotIn("expected_decision", sibling["payload"])

    def test_report_exposes_layer_funnel(self):
        case = convert_case({"id": "SYN-X", "ground_truth": {"label": "BLOCK"},
                             "case_profile": {"type": "recipient_substitution"}}, 0,
                            [synthetic_address(0)])
        row = {key: case[key] for key in ("case_id", "attack_type", "expected_decision")}
        row["response"] = {"decision": "BLOCK", "blocked_by": "blacklist", "layers": [
            {"layer": "blacklist", "decision": "BLOCK"}]}
        report = detailed_report([row])
        self.assertEqual(report["blocked_by"]["blacklist"], 1)
        self.assertEqual(report["attack_detection"]["blocked"], 1)
        self.assertEqual(report["funnel"]["passed_blacklist"], 0)
        self.assertIn("raw_model", report["metrics"])
        self.assertEqual(report["metrics"]["baselines"]["always_block"]["accuracy"], 1.0)


if __name__ == "__main__":
    unittest.main()
