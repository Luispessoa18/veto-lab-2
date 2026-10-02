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
                         "simulation_revert", "benign"}.issubset(kinds))

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


if __name__ == "__main__":
    unittest.main()
