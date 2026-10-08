import json
import random
import unittest

from src import svm_dataset as sd

A, B, C = "A" * 43 + "1", "B" * 43 + "2", "C" * 43 + "3"
POOL = "P" * 43 + "4"
USDC = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"
MINT = "M" * 43 + "5"
FEE = 5000


def rec(digest="d1", sol=(), tokens=(), authority=(), closed=(), created=(), token_accounts=None, signers=(A,)):
    return {"tx_digest": digest, "slot": 1, "signers": list(signers), "fee_payer": signers[0], "programs": [],
            "units": 1000, "fee": FEE, "token_accounts": token_accounts or {},
            "projection": {"sol": list(sol), "tokens": list(tokens), "authority": list(authority),
                           "closed": list(closed), "created": list(created)}}


def sol(account, pre, post):
    return {"account": account, "pre": pre, "post": post}


def tok(account, mint, owner, pre, post, decimals=6):
    return {"account": account, "mint": mint, "owner": owner, "pre": str(pre), "post": str(post), "decimals": decimals}


SOL_TRANSFER = rec("sol", sol=[sol(A, 2_000_000_000, 1_500_000_000 - FEE), sol(B, 0, 500_000_000)], created=[B])
TOKEN_TRANSFER = rec("tok", sol=[sol(A, 10_000_000, 10_000_000 - FEE)],
                     tokens=[tok("X" * 44, USDC, A, 2_500_000, 0), tok("Y" * 44, USDC, B, 0, 2_500_000)])
SWAP = rec("swap", sol=[sol(A, 10_000_000, 10_000_000 - FEE)],
           tokens=[tok("X" * 44, USDC, A, 7_000_000, 2_000_000), tok("Z" * 44, MINT, A, 0, 123_000_000_000, 9),
                   tok("Q" * 44, USDC, POOL, 1, 5_000_001), tok("R" * 44, MINT, POOL, 999_000_000_000, 876_000_000_000, 9)])
APPROVE = rec("appr", sol=[sol(A, 10_000_000, 10_000_000 - FEE)],
              authority=[{"account": "X" * 44, "field": "delegate", "pre": None, "post": C}],
              token_accounts={"X" * 44: {"mint": USDC, "owner": A}})
# A swap that also hands a delegate to someone and closes one of the signer's token accounts.
SWAP_PLUS = rec("plus", sol=[sol(A, 10_000_000, 12_039_280 - FEE), sol("W" * 44, 2_039_280, 0)],
                tokens=SWAP["projection"]["tokens"],
                authority=[{"account": "X" * 44, "field": "delegate", "pre": None, "post": C},
                           {"account": "V" * 44, "field": "owner", "pre": A, "post": C}],
                closed=["W" * 44],
                token_accounts={"X" * 44: {"mint": USDC, "owner": A}, "V" * 44: {"mint": MINT, "owner": A},
                                "W" * 44: {"mint": MINT, "owner": A}})


class IntentTests(unittest.TestCase):
    def test_sol_transfer(self):
        i = sd.derive_intent(SOL_TRANSFER)
        self.assertEqual((i["action"], i["asset"], i["amount"], i["recipient"]), ("transfer_sol", "SOL", "0.5", B))
        self.assertEqual(sd.label(i, SOL_TRANSFER, {}), ([], []))

    def test_token_transfer(self):
        i = sd.derive_intent(TOKEN_TRANSFER)
        self.assertEqual((i["action"], i["asset"], i["amount"], i["recipient"]), ("transfer_token", USDC, "2.5", B))
        self.assertEqual(sd.label(i, TOKEN_TRANSFER, {}), ([], []))

    def test_swap(self):
        i = sd.derive_intent(SWAP)
        self.assertEqual((i["action"], i["asset"], i["amount"], i["recipient"]), ("swap", USDC, "5", None))
        self.assertIn("token_in:" + MINT, i["allowed_effects"])
        self.assertEqual(sd.label(i, SWAP, {}), ([], []))

    def test_approve(self):
        i = sd.derive_intent(APPROVE)
        self.assertEqual((i["action"], i["asset"], i["recipient"]), ("approve", USDC, C))
        self.assertEqual(i["allowed_effects"], ["approve:" + C])
        self.assertEqual(sd.label(i, APPROVE, {}), ([], []))

    def test_honest_intent_declares_every_extra_effect(self):
        i = sd.derive_intent(SWAP_PLUS)
        self.assertEqual(i["action"], "swap")
        for e in ("approve:" + C, "authority_change:" + "V" * 44 + ":owner", "close:" + "W" * 44):
            self.assertIn(e, i["allowed_effects"])
        self.assertEqual(sd.label(i, SWAP_PLUS, {}), ([], []))

    def test_nothing_derivable_is_skipped(self):
        fee_only = rec("fee", sol=[sol(A, 10_000, 10_000 - FEE)])
        self.assertIsNone(sd.derive_intent(fee_only))
        inflow_only = rec("in", tokens=[tok("Z" * 44, MINT, A, 0, 5)])
        self.assertIsNone(sd.derive_intent(inflow_only))


class MutationTests(unittest.TestCase):
    def check(self, kind, record):
        honest = sd.derive_intent(record)
        out = sd.mutate(kind, honest, record, random.Random(1))
        self.assertIsNotNone(out, kind)
        intent, metadata = out
        signals, reasons = sd.label(intent, record, metadata)
        self.assertEqual(signals, [kind])
        self.assertEqual(len(reasons), 1)
        self.assertLessEqual(len(reasons[0]), 180)
        return honest, intent, metadata, reasons[0]

    def test_recipient_mismatch(self):
        _, intent, _, reason = self.check("recipient_mismatch", TOKEN_TRANSFER)
        self.assertNotEqual(intent["recipient"], B)
        self.assertIn(B, reason)
        self.check("recipient_mismatch", SOL_TRANSFER)
        self.check("recipient_mismatch", APPROVE)

    def test_amount_understated_cites_both_numbers(self):
        _, intent, _, reason = self.check("amount_understated", SOL_TRANSFER)
        self.assertLess(float(intent["amount"]), 0.5 / 1.01)
        self.assertIn(intent["amount"], reason)
        self.assertIn("0.5", reason)
        self.check("amount_understated", SWAP)

    def test_small_difference_is_not_understated(self):
        i = dict(sd.derive_intent(SOL_TRANSFER), amount="0.4975")
        self.assertEqual(sd.label(i, SOL_TRANSFER, {})[0], [])

    def test_asset_mismatch(self):
        _, intent, _, _ = self.check("asset_mismatch", TOKEN_TRANSFER)
        self.assertNotEqual(intent["asset"], USDC)
        self.check("asset_mismatch", SOL_TRANSFER)
        self.check("asset_mismatch", SWAP)

    def test_undeclared_effects(self):
        for kind in ("undeclared_approval", "undeclared_authority_change", "undeclared_close"):
            self.check(kind, SWAP_PLUS)
        self.check("undeclared_approval", APPROVE)

    def test_undeclared_needs_the_effect(self):
        honest = sd.derive_intent(TOKEN_TRANSFER)
        for kind in ("undeclared_approval", "undeclared_authority_change", "undeclared_close"):
            self.assertIsNone(sd.mutate(kind, honest, TOKEN_TRANSFER, random.Random(1)))

    def test_injected_instruction_keeps_the_honest_intent(self):
        for seed in range(20):
            honest = sd.derive_intent(TOKEN_TRANSFER)
            intent, metadata = sd.mutate("injected_instruction", honest, TOKEN_TRANSFER, random.Random(seed))
            self.assertEqual(intent, honest)
            self.assertEqual(sd.label(intent, TOKEN_TRANSFER, metadata)[0], ["injected_instruction"])
        self.assertTrue(any("ALLOW" in t and "decision" in t for t in sd.INJECTIONS))
        self.assertTrue(any("autorize" in t or "aprove" in t for t in sd.INJECTIONS))

    def test_benign_memos_are_not_injections(self):
        for memo in sd.BENIGN_MEMOS:
            self.assertEqual(sd.label(sd.derive_intent(SWAP), SWAP, {"memo": memo})[0], [])


class OutputTests(unittest.TestCase):
    def records(self, n=60):
        base = [SOL_TRANSFER, TOKEN_TRANSFER, SWAP, APPROVE, SWAP_PLUS]
        return [dict(base[i % len(base)], tx_digest=f"tx{i:03d}") for i in range(n)]

    def test_split_never_shares_a_tx(self):
        splits, stats = sd.build(self.records(), "risk", seed=42)
        owners = {}
        for name, rows in splits.items():
            for row in rows:
                owners.setdefault(row["tx_digest"], set()).add(name)
        self.assertTrue(all(len(v) == 1 for v in owners.values()))
        self.assertEqual(sum(stats["splits"][k]["examples"] for k in splits), stats["examples"])
        self.assertEqual(set(splits), {"train", "valid", "test"})

    def test_split_is_deterministic_for_a_seed(self):
        self.assertEqual(sd.split_of("abc", 42), sd.split_of("abc", 42))
        counts = {"train": 0, "valid": 0, "test": 0}
        for i in range(2000):
            counts[sd.split_of(f"d{i}", 42)] += 1
        self.assertGreater(counts["train"], 1450)
        self.assertGreater(counts["valid"], 120)
        self.assertGreater(counts["test"], 120)

    def test_risk_format_never_says_allow_block_or_decision(self):
        splits, stats = sd.build(self.records(), "risk", seed=7)
        rows = [r for v in splits.values() for r in v]
        self.assertGreater(stats["per_signal"].get("injected_instruction", 0), 0)
        for r in rows:
            msgs = r["example"]["messages"]
            self.assertEqual([m["role"] for m in msgs], ["system", "user", "assistant"])
            answer = msgs[2]["content"]
            for word in ("ALLOW", "BLOCK", "decision"):
                self.assertNotIn(word, answer)
                self.assertNotIn(word, msgs[0]["content"])
            obj = json.loads(answer)
            self.assertEqual(set(obj), {"risk", "signals", "reasons"})
            self.assertEqual(obj["risk"], "high" if obj["signals"] else "low")

    def test_decision_format_matches_the_lab_shape(self):
        splits, _ = sd.build(self.records(), "decision", seed=7)
        for r in [r for v in splits.values() for r in v]:
            obj = json.loads(r["example"]["messages"][2]["content"])
            self.assertEqual(set(obj), {"decision", "confidence", "reasons", "evidence_fields"})
            self.assertIn(obj["decision"], ("ALLOW", "BLOCK"))
            self.assertTrue(1 <= len(obj["reasons"]) <= 3)

    def test_balance_one_honest_and_one_or_two_mutations_per_tx(self):
        recs = self.records(100)
        splits, stats = sd.build(recs, "risk", seed=3)
        self.assertEqual(stats["per_signal"]["none"], 100)
        mutated = stats["examples"] - 100
        self.assertTrue(100 <= mutated <= 200, mutated)
        top = max(v for k, v in stats["per_signal"].items() if k != "none")
        self.assertLessEqual(top, 0.35 * mutated + 1, stats["per_signal"])

    def test_skips_are_counted(self):
        recs = self.records(10) + [rec("fee", sol=[sol(A, 10_000, 10_000 - FEE)])]
        _, stats = sd.build(recs, "risk", seed=1)
        self.assertEqual(stats["records"], 11)
        self.assertEqual(sum(stats["skipped"].values()), 1)


RENT = 2_039_280
UNLIMITED = str(2 ** 64 - 1)


def synth(digest, **kw):
    r = rec(digest, **kw)
    r["source"] = "synthetic"
    return r


APPROVE_BOUNDED = synth("ab", sol=[sol(A, 10_000_000, 10_000_000 - FEE)],
                        authority=[{"account": "X" * 44, "field": "delegate", "pre": None, "post": C}],
                        token_accounts={"X" * 44: {"mint": USDC, "owner": A, "delegated_amount": "5000000"}})
APPROVE_UNLIMITED = synth("au", sol=[sol(A, 10_000_000, 10_000_000 - FEE)],
                          authority=[{"account": "X" * 44, "field": "delegate", "pre": None, "post": C}],
                          token_accounts={"X" * 44: {"mint": USDC, "owner": A, "delegated_amount": UNLIMITED}})
SET_OWNER = synth("so", sol=[sol(A, 10_000_000, 10_000_000 - FEE)],
                  authority=[{"account": "X" * 44, "field": "owner", "pre": A, "post": C}],
                  token_accounts={"X" * 44: {"mint": USDC, "owner": A}})
CLOSE_STRANGER = synth("cs", sol=[sol(A, 10_000_000, 10_000_000 - FEE), sol("X" * 44, RENT, 0), sol(C, 0, RENT)],
                       closed=["X" * 44], created=[C], token_accounts={"X" * 44: {"mint": USDC, "owner": A}})
CLOSE_BACK = synth("cb", sol=[sol(A, 10_000_000, 10_000_000 + RENT - FEE), sol("X" * 44, RENT, 0)],
                   closed=["X" * 44], token_accounts={"X" * 44: {"mint": USDC, "owner": A}})


class PermissionTests(unittest.TestCase):
    def test_approve_intent_has_spender_and_amount(self):
        i = sd.derive_intent(APPROVE_BOUNDED)
        self.assertEqual((i["action"], i["asset"], i["recipient"], i["amount"]), ("approve", USDC, C, "5000000"))
        self.assertEqual(sd.derive_intent(APPROVE_UNLIMITED)["amount"], "unlimited")
        for r in (APPROVE_BOUNDED, APPROVE_UNLIMITED):
            self.assertEqual(sd.label(sd.derive_intent(r), r, {}), ([], []))

    def test_set_authority_intent_names_the_new_authority(self):
        i = sd.derive_intent(SET_OWNER)
        self.assertEqual((i["action"], i["asset"], i["recipient"]), ("set_authority", USDC, C))
        self.assertEqual(i["allowed_effects"], ["authority_change:" + "X" * 44 + ":owner"])
        self.assertEqual(sd.label(i, SET_OWNER, {}), ([], []))

    def test_close_intent_names_the_rent_destination(self):
        self.assertEqual(sd.derive_intent(CLOSE_STRANGER)["recipient"], C)
        self.assertEqual(sd.derive_intent(CLOSE_BACK)["recipient"], A)
        for r in (CLOSE_STRANGER, CLOSE_BACK):
            i = sd.derive_intent(r)
            self.assertEqual(i["action"], "close")
            self.assertEqual(sd.label(i, r, {}), ([], []))

    def check(self, kind, record):
        out = sd.mutate(kind, sd.derive_intent(record), record, random.Random(3))
        self.assertIsNotNone(out, (kind, record["tx_digest"]))
        signals, reasons = sd.label(out[0], record, out[1])
        self.assertEqual(signals, [kind], record["tx_digest"])
        return out[0], reasons[0]

    def test_approval_exceeds_intent(self):
        intent, reason = self.check("approval_exceeds_intent", APPROVE_UNLIMITED)
        self.assertNotEqual(intent["amount"], "unlimited")
        self.assertIn("unlimited", reason)
        intent, reason = self.check("approval_exceeds_intent", APPROVE_BOUNDED)
        self.assertLess(int(intent["amount"]), 5_000_000)
        self.assertIn("5000000", reason)
        self.assertIsNone(sd.mutate("approval_exceeds_intent", sd.derive_intent(SET_OWNER), SET_OWNER, random.Random(1)))
        # A delegate without a known amount cannot be judged exceeded.
        self.assertIsNone(sd.mutate("approval_exceeds_intent", sd.derive_intent(APPROVE), APPROVE, random.Random(1)))

    def test_undeclared_permission_effects(self):
        self.check("undeclared_approval", APPROVE_UNLIMITED)
        self.check("undeclared_authority_change", SET_OWNER)
        self.check("undeclared_close", CLOSE_STRANGER)

    def test_wrong_destination_or_authority_is_recipient_mismatch(self):
        self.check("recipient_mismatch", CLOSE_STRANGER)
        self.check("recipient_mismatch", SET_OWNER)


class SourceAndFilterTests(unittest.TestCase):
    def test_min_sol_drops_spam_transfers(self):
        spam = rec("spam", sol=[sol(A, 2_000_000_000, 2_000_000_000 - 500_000 - FEE), sol(B, 10, 500_010)])
        _, stats = sd.build([spam, SOL_TRANSFER], "risk", seed=1)
        self.assertEqual(stats["skipped"], {"below_min_sol": 1})
        _, stats = sd.build([spam, SOL_TRANSFER], "risk", seed=1, min_sol=0.0001)
        self.assertEqual(stats["skipped"], {})

    def test_stats_split_by_source_and_missing_source_is_mainnet(self):
        recs = [SOL_TRANSFER, TOKEN_TRANSFER, APPROVE_BOUNDED, SET_OWNER, CLOSE_STRANGER]
        _, stats = sd.build(recs, "risk", seed=1)
        self.assertEqual(stats["per_source"]["mainnet"]["records"], 2)
        self.assertEqual(stats["per_source"]["synthetic"]["records"], 3)
        self.assertEqual(stats["per_source"]["synthetic"]["per_signal"]["none"], 3)

    def test_cli_reads_several_effects_files(self):
        import tempfile
        from pathlib import Path
        with tempfile.TemporaryDirectory() as d:
            a, b = Path(d) / "real.jsonl", Path(d) / "synth.jsonl"
            a.write_text(json.dumps(SOL_TRANSFER) + "\n")
            b.write_text("\n".join(json.dumps(r) for r in (APPROVE_UNLIMITED, SET_OWNER)) + "\n")
            sd.main(["--effects", str(a), str(b), "--out", str(Path(d) / "ds"), "--seed", "1"])
            stats = json.loads((Path(d) / "ds" / "stats.json").read_text())
            self.assertEqual(stats["records"], 3)
            self.assertEqual(set(stats["per_source"]), {"mainnet", "synthetic"})


if __name__ == "__main__":
    unittest.main()
