import unittest
from unittest.mock import MagicMock, patch

import requests

from src.unified_api import VetoPipeline

CONFIG = {
    "solana_rpc_url": "",
    "solana_cluster": "devnet",
    "solana_rpc_by_cluster": {"devnet": "https://public.devnet"},
    "aval_svm_url": "http://127.0.0.1:8899",
    "aval_svm_cluster": "devnet",
    "solana_timeout_seconds": 5,
}
PAYLOAD = {"chain": "solana", "solana": {"serialized_transaction": "AQ=="}}


def reply(body):
    resp = MagicMock()
    resp.json.return_value = body
    resp.raise_for_status.return_value = None
    return resp


OK = {"result": {"context": {"slot": 1}, "value": {"err": None, "logs": [], "unitsConsumed": 150},
                 "aval": {"digest": "ab" * 32, "stateSlot": 1, "cache": {"hits": 2, "misses": 0}, "elapsedUs": 900}}}


class AvalSvmTests(unittest.TestCase):
    def pipe(self, **over):
        return VetoPipeline(config={**CONFIG, **over}, blacklist=set())

    def test_prefers_aval_svm_for_its_cluster(self):
        self.assertEqual(self.pipe().solana_rpc_targets({}, "devnet"),
                         [("aval-svm", "http://127.0.0.1:8899"), ("rpc", "https://public.devnet")])
        self.assertEqual(self.pipe().solana_rpc_targets({}, "mainnet"), [])

    def test_explicit_rpc_url_wins(self):
        self.assertEqual(self.pipe().solana_rpc_targets({"rpc_url": "http://x"}, "devnet"), [("rpc", "http://x")])

    def test_uses_aval_svm_and_reports_it(self):
        with patch("src.unified_api.requests.post", return_value=reply(OK)) as post:
            out = self.pipe().simulate_solana(PAYLOAD, "r1")
        self.assertEqual(post.call_args.args[0], "http://127.0.0.1:8899")
        self.assertEqual(out["decision"], "ALLOW")
        self.assertEqual(out["engine"], "aval-svm")
        self.assertEqual(out["aval"]["elapsedUs"], 900)

    def test_falls_back_when_aval_svm_is_not_running(self):
        with patch("src.unified_api.requests.post",
                   side_effect=[requests.ConnectionError("refused"), reply({"result": {"value": {"err": None}}})]) as post:
            out = self.pipe().simulate_solana(PAYLOAD, "r2")
        self.assertEqual(post.call_args.args[0], "https://public.devnet")
        self.assertEqual(out["engine"], "rpc")
        self.assertEqual(out["decision"], "ALLOW")

    def test_falls_back_on_unsupported_program_or_upstream_error(self):
        for code in (-32603, -32004, -32005):
            with patch("src.unified_api.requests.post",
                       side_effect=[reply({"error": {"code": code, "message": "x"}}), reply({"result": {"value": {"err": None}}})]):
                out = self.pipe().simulate_solana(PAYLOAD, "r3")
            self.assertEqual(out["engine"], "rpc")

    def test_falls_back_on_aval_svm_timeout(self):
        with patch("src.unified_api.requests.post",
                   side_effect=[requests.ReadTimeout("slow"), reply({"result": {"value": {"err": None}}})]):
            out = self.pipe().simulate_solana(PAYLOAD, "r5")
        self.assertEqual((out["engine"], out["decision"]), ("rpc", "ALLOW"))

    def test_reply_without_aval_meta_is_not_aval_svm(self):
        # e.g. a solana-test-validator listening on 8899
        foreign = {"result": {"context": {"slot": 1}, "value": {"err": None, "logs": [], "unitsConsumed": 150}}}
        with patch("src.unified_api.requests.post",
                   side_effect=[reply(foreign), reply({"result": {"value": {"err": None}}})]) as post:
            out = self.pipe().simulate_solana(PAYLOAD, "r6")
        self.assertEqual(post.call_args.args[0], "https://public.devnet")
        self.assertEqual((out["engine"], out["decision"]), ("rpc", "ALLOW"))

    def test_any_aval_svm_exception_falls_back(self):
        bad = MagicMock()
        bad.raise_for_status.side_effect = requests.HTTPError("502 Bad Gateway")
        not_json = MagicMock()
        not_json.raise_for_status.return_value = None
        not_json.json.side_effect = ValueError("not json")
        for first in (bad, not_json, reply(["not", "an", "object"])):
            with patch("src.unified_api.requests.post",
                       side_effect=[first, reply({"result": {"value": {"err": None}}})]):
                out = self.pipe().simulate_solana(PAYLOAD, "r7")
            self.assertEqual((out["engine"], out["decision"]), ("rpc", "ALLOW"))

    def test_public_rpc_http_error_is_still_review(self):
        bad = MagicMock()
        bad.raise_for_status.side_effect = requests.HTTPError("502 Bad Gateway")
        with patch("src.unified_api.requests.post", return_value=bad):
            out = self.pipe(aval_svm_url="").simulate_solana(PAYLOAD, "r8")
        self.assertEqual((out["engine"], out["decision"], out["reason"]), ("rpc", "REVIEW", "SOLANA_RPC_UNAVAILABLE"))

    def test_aval_svm_has_its_own_timeout(self):
        with patch("src.unified_api.requests.post",
                   side_effect=[requests.ConnectionError("refused"), reply({"result": {"value": {"err": None}}})]) as post:
            self.pipe().simulate_solana(PAYLOAD, "r9")
        self.assertEqual([c.kwargs["timeout"] for c in post.call_args_list], [12, 5])
        with patch("src.unified_api.requests.post", return_value=reply(OK)) as post:
            self.pipe(aval_svm_timeout_seconds=3).simulate_solana(PAYLOAD, "r10")
        self.assertEqual(post.call_args.kwargs["timeout"], 3)

    def test_divergent_worlds_turn_allow_into_review(self):
        divergent = {"result": {**OK["result"], "aval": {**OK["result"]["aval"], "worlds": 2, "divergent": True}}}
        with patch("src.unified_api.requests.post", return_value=reply(divergent)) as post:
            out = self.pipe().simulate_solana(PAYLOAD, "r11")
        self.assertEqual((out["decision"], out["reason"], out["engine"]), ("REVIEW", "SOLANA_SIMULATION_DIVERGENT", "aval-svm"))
        self.assertEqual(out["aval"]["worlds"], 2)
        sent = post.call_args.kwargs["json"]["params"][1]
        self.assertEqual(sent["aval"], {"worlds": True}, "opts in to the two-worlds reply")
        same = {"result": {**OK["result"], "aval": {**OK["result"]["aval"], "worlds": 2, "divergent": False}}}
        with patch("src.unified_api.requests.post", return_value=reply(same)):
            self.assertEqual(self.pipe().simulate_solana(PAYLOAD, "r12")["decision"], "ALLOW")
        failed = {"result": {"value": {"err": {"InstructionError": [0, "x"]}, "logs": []},
                             "aval": {**OK["result"]["aval"], "worlds": 2, "divergent": True}}}
        with patch("src.unified_api.requests.post", return_value=reply(failed)):
            self.assertEqual(self.pipe().simulate_solana(PAYLOAD, "r13")["decision"], "BLOCK")

    def test_everything_down_is_review(self):
        with patch("src.unified_api.requests.post", side_effect=requests.ConnectionError("down")):
            out = self.pipe().simulate_solana(PAYLOAD, "r4")
        self.assertEqual((out["decision"], out["reason"]), ("REVIEW", "SOLANA_RPC_UNAVAILABLE"))


if __name__ == "__main__":
    unittest.main()
