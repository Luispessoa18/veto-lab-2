import json
import tempfile
import unittest
from collections import Counter
from pathlib import Path

from src import solana_case_generator as generator
from src.solana_attack_simulator import detailed_report, synthetic_address
from src.unified_api import VetoPipeline

BLACKLIST = [synthetic_address(index) for index in range(20)]


class TestSolanaCaseGenerator(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.cases, cls.answers = generator.generate(1500, 7, BLACKLIST)
        cls.by_id = {answer["request_id"]: answer for answer in cls.answers}

    def test_blacklist_address_is_valid_base58_shape(self):
        address = synthetic_address(1)
        self.assertEqual(len(address), 32)
        self.assertNotIn("0", address)
        self.assertNotIn("O", address)

    def test_generation_is_reproducible_only_with_same_seed(self):
        first = generator.generate(60, 123, BLACKLIST)
        self.assertEqual(first, generator.generate(60, 123, BLACKLIST))
        self.assertNotEqual(first, generator.generate(60, 456, BLACKLIST))

    def test_all_kinds_appear_and_classes_are_balanced(self):
        kinds = Counter(answer["attack_type"] for answer in self.answers)
        self.assertTrue(set(generator.ATTACK_KINDS) | set(generator.BENIGN_KINDS) <= set(kinds))
        attacks = sum(answer["expected_decision"] == "BLOCK" for answer in self.answers)
        self.assertTrue(0.4 < attacks / len(self.answers) < 0.6)

    def test_payload_never_carries_the_answer(self):
        for case in self.cases:
            self.assertEqual(generator.leak_audit(case["payload"]), [])
            self.assertEqual(case["payload"]["id"], case["request_id"])
            self.assertEqual(set(case), {"request_id", "payload"})

    def test_leak_audit_catches_old_leaks(self):
        self.assertTrue(generator.leak_audit({"tx": {"program_id": "MaLiciousProgramxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"}}))
        self.assertTrue(generator.leak_audit({"solana": {"serialized_transaction": "SIM_REVERT"}}))
        self.assertTrue(generator.leak_audit({"expected_decision": "BLOCK"}))

    def test_ids_and_addresses_do_not_predict_the_label(self):
        # Antes, ID ímpar = ataque. Agora o ID é aleatório: nenhum caractere final separa as classes.
        by_last = Counter((case["request_id"][-1], self.by_id[case["request_id"]]["expected_decision"])
                          for case in self.cases)
        for char in {key[0] for key in by_last}:
            block, allow = by_last[(char, "BLOCK")], by_last[(char, "ALLOW")]
            self.assertFalse(block + allow >= 20 and min(block, allow) == 0, char)
        recipients = Counter(case["payload"]["scenario"]["intent"]["recipient"] for case in self.cases)
        self.assertLess(recipients.most_common(1)[0][1], 25)

    def test_policy_has_no_false_positive_on_benign_cases(self):
        for case in self.cases:
            answer = self.by_id[case["request_id"]]
            if answer["expected_decision"] == "ALLOW":
                self.assertEqual(VetoPipeline.solana_policy(case["payload"]), [], answer["attack_type"])

    def test_policy_closes_the_bypasses_found_in_the_benchmark(self):
        reasons = {"split_drain": "TOTAL_AMOUNT_EXCEEDS_LIMIT", "swap_slippage_sandwich": "MIN_OUT_BELOW_AUTHORIZED",
                   "permanent_delegate_mint": "DANGEROUS_MINT_EXTENSION"}
        for case in self.cases:
            answer = self.by_id[case["request_id"]]
            if answer["attack_type"] in reasons:
                found = {c["reason"] for c in VetoPipeline.solana_policy(case["payload"])}
                self.assertIn(reasons[answer["attack_type"]], found)

    def test_evasive_injection_fields_are_treated_as_free_text(self):
        from src.unified_api import free_texts
        for case in self.cases:
            if self.by_id[case["request_id"]]["attack_type"] == "prompt_injection_evasive":
                self.assertTrue(free_texts(case["payload"]))

    def test_metamorphic_sibling_keeps_answer(self):
        siblings = [answer for answer in self.answers if "metamorphic_parent" in answer]
        self.assertTrue(siblings)
        for sibling in siblings:
            parent = self.by_id[sibling["metamorphic_parent"]]
            self.assertEqual((sibling["attack_type"], sibling["expected_decision"]),
                             (parent["attack_type"], parent["expected_decision"]))

    def test_payloads_and_answers_live_in_separate_files(self):
        with tempfile.TemporaryDirectory() as folder:
            cases_path, answers_path = Path(folder) / "c.jsonl", Path(folder) / "a.jsonl"
            generator.write(self.cases[:20], self.answers[:20], cases_path, answers_path)
            self.assertNotIn("expected_decision", cases_path.read_text(encoding="utf-8"))
            joined = generator.read(cases_path, answers_path)
            self.assertEqual(len(joined), 20)
            self.assertTrue(all("expected_decision" in case and "payload" in case for case in joined))

    def test_api_code_never_reads_the_answers(self):
        # A API e o relatório do painel /admin não podem abrir o gabarito.
        for name in ("unified_api.py", "solana_html_report.py", "eval.py", "guard_server.py"):
            source = (Path(__file__).parents[1] / "src" / name).read_text(encoding="utf-8")
            for forbidden in ("solana_answers", "ANSWERS_OUT", "solana_case_generator.read", "generator.read"):
                self.assertNotIn(forbidden, source, name)
        api = (Path(__file__).parents[1] / "src" / "unified_api.py").read_text(encoding="utf-8")
        self.assertNotIn("expected_decision", api)

    def test_report_exposes_layer_funnel(self):
        row = {"case_id": "x", "attack_type": "recipient_substitution", "expected_decision": "BLOCK",
               "response": {"decision": "BLOCK", "blocked_by": "blacklist", "layers": [
                   {"layer": "blacklist", "decision": "BLOCK"}]}}
        report = detailed_report([row])
        self.assertEqual(report["blocked_by"]["blacklist"], 1)
        self.assertEqual(report["attack_detection"]["blocked"], 1)
        self.assertEqual(report["funnel"]["passed_blacklist"], 0)
        self.assertIn("raw_model", report["metrics"])
        self.assertEqual(report["metrics"]["baselines"]["always_block"]["accuracy"], 1.0)


class TestSolanaHtmlReport(unittest.TestCase):
    def test_report_explains_policy_block_and_false_positive(self):
        from src.solana_attack_simulator import detailed_report
        from src.solana_html_report import build_html
        policy = {"decision": "BLOCK", "layer": "layer_1", "reason": "SOLANA_POLICY_CONFLICT",
                  "conflicts": [{"field": "scenario.transaction.instructions[0].recipient",
                                 "reason": "RECIPIENT_MISMATCH"}]}
        corrected = {"decision": "ALLOW", "layer": "layer_1", "model_decision": "BLOCK",
                     "policy_override": "DETERMINISTIC_CONSISTENCY"}
        base = {"source_case_id": "S", "expected_layer": "x", "split": "holdout", "elapsed_ms": 10}
        rows = [{**base, "case_id": "A", "attack_type": "recipient_substitution", "expected_decision": "BLOCK",
                 "response": {"decision": "BLOCK", "blocked_by": "layer_1", "layers": [policy]}},
                {**base, "case_id": "B", "attack_type": "benign_transfer", "expected_decision": "ALLOW",
                 "response": {"decision": "ALLOW", "blocked_by": None, "layers": [corrected]}}]
        page = build_html(rows, detailed_report(rows))
        self.assertIn("Destinatário diferente do autorizado — parâmetro instructions[0].recipient", page)
        self.assertIn("Falso positivo da IA barrado", page)
        self.assertIn("window.print()", page)
