import json
import random
import re
import unittest
from collections import Counter

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


def rekey(r, i):
    """Same record with its own fee payer (the builder caps records per payer)."""
    return json.loads(json.dumps(r).replace(A, "A" * 40 + "%03d" % i + "1"))


def sol(account, pre, post):
    return {"account": account, "pre": pre, "post": post}


def tok(account, mint, owner, pre, post, decimals=6):
    return {"account": account, "mint": mint, "owner": owner, "pre": str(pre), "post": str(post), "decimals": decimals}


JUPLIKE = "J" * 43 + "6"
# Assets seen in honest intents (the builder takes them from the same split).
ASSETS = [("SOL", 9), (USDC, 6), (MINT, 9), (JUPLIKE, 5)]

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

    def test_no_outflow_is_an_honest_other_intent(self):
        fee_only = rec("fee", sol=[sol(A, 10_000, 10_000 - FEE)])
        inflow_only = rec("in", tokens=[tok("Z" * 44, MINT, A, 0, 5)])
        for r, effects in ((fee_only, []), (inflow_only, ["token_in:" + MINT])):
            i = sd.derive_intent(r)
            self.assertEqual((i["action"], i["asset"], i["amount"], i["recipient"]), ("other", None, None, None))
            self.assertEqual(i["allowed_effects"], effects)
            self.assertEqual(sd.label(i, r, {}), ([], []))


class MutationTests(unittest.TestCase):
    def check(self, kind, record):
        honest = sd.derive_intent(record)
        out = sd.mutate(kind, honest, record, random.Random(1), twins=TWINS, assets=ASSETS)
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

    def test_injected_instruction_keeps_the_honest_intent_and_the_field(self):
        for seed in range(20):
            honest = sd.derive_intent(TOKEN_TRANSFER)
            field = sd.METADATA_FIELDS[seed % len(sd.METADATA_FIELDS)]
            tx_meta = {field: sd.BENIGN_TEXT[field][0]}
            intent, metadata = sd.mutate("injected_instruction", honest, TOKEN_TRANSFER, random.Random(seed), metadata=tx_meta)
            self.assertEqual(intent, honest)
            self.assertEqual(list(metadata), [field])
            self.assertNotEqual(metadata[field], tx_meta[field])
            self.assertEqual(sd.label(intent, TOKEN_TRANSFER, metadata)[0], ["injected_instruction"])
        # No field to hide an instruction in: not applicable.
        self.assertIsNone(sd.mutate("injected_instruction", sd.derive_intent(TOKEN_TRANSFER), TOKEN_TRANSFER, random.Random(1), metadata={}))

    def test_injection_templates_are_varied_and_all_detected(self):
        self.assertGreaterEqual(len(sd.INJECTIONS), 30)
        self.assertTrue(any("ALLOW" in t and "decision" in t for t in sd.INJECTIONS))
        self.assertTrue(any("autorize" in t or "aprove" in t for t in sd.INJECTIONS))
        self.assertTrue(any("answer field" in t or "campo de resposta" in t for t in sd.INJECTIONS))
        self.assertTrue(any(re.search(r"[A-Za-z0-9+/]{16,}={1,2}", t) for t in sd.INJECTIONS))
        self.assertTrue(any(t.lower().startswith(("please", "por favor")) for t in sd.INJECTIONS))
        reserved = set(sd.TEST_ONLY_INJECTIONS)
        self.assertTrue(0.25 <= len(reserved) / len(sd.INJECTIONS) <= 0.35)
        self.assertTrue(reserved <= set(sd.INJECTIONS))
        for t in sd.INJECTIONS:
            self.assertEqual(sd.label(sd.derive_intent(SWAP), SWAP, {"memo": t})[0], ["injected_instruction"], t)

    def test_benign_texts_are_not_injections(self):
        for field, texts in sd.BENIGN_TEXT.items():
            for t in texts:
                self.assertEqual(sd.label(sd.derive_intent(SWAP), SWAP, {field: t})[0], [], (field, t))
        self.assertEqual(set(sd.BENIGN_TEXT), set(sd.METADATA_FIELDS))


class OutputTests(unittest.TestCase):
    def records(self, n=60):
        base = [SOL_TRANSFER, TOKEN_TRANSFER, SWAP, APPROVE, SWAP_PLUS]
        return [rekey(dict(base[i % len(base)], tx_digest=f"tx{i:03d}"), i) for i in range(n)]

    def test_records_per_fee_payer_are_capped(self):
        same_payer = [dict(SOL_TRANSFER, tx_digest=f"p{i}") for i in range(12)]
        _, stats = sd.build(same_payer, "risk", seed=1)
        self.assertEqual(stats["used_records"], sd.MAX_RECORDS_PER_PAYER)
        self.assertEqual(stats["skipped"]["payer_cap"], 12 - sd.MAX_RECORDS_PER_PAYER)
        self.assertEqual(stats["payers_capped"], 1)
        kept = {r["tx_digest"] for v in sd.build(same_payer, "risk", seed=1)[0].values() for r in v}
        self.assertEqual(kept, {r["tx_digest"] for v in sd.build(list(reversed(same_payer)), "risk", seed=1)[0].values() for r in v})

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
        # SOL leaves the signer but nobody receives it: no sensible intent.
        recs = self.records(10) + [rec("burn", sol=[sol(A, 10_000_000, 5_000_000 - FEE)])]
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
                   closed=["X" * 44], token_accounts={"X" * 44: {"mint": USDC, "owner": A}},
                   authority=[{"account": "X" * 44, "field": "programOwner", "pre": "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
                               "post": "11111111111111111111111111111111"}])
APPROVE_DECIMALS = synth("ad", sol=[sol(A, 10_000_000, 10_000_000 - FEE)],
                         authority=[{"account": "X" * 44, "field": "delegate", "pre": None, "post": C}],
                         token_accounts={"X" * 44: {"mint": USDC, "owner": A, "delegated_amount": "2500000", "decimals": 6}})


# A real transfer plus a permission change in the same transaction (dataset-synth bundled cases).
def bundled(digest, case, **extra):
    r = synth(digest, sol=[sol(A, 10_000_000, 10_000_000 - FEE)] + extra.pop("sol", []),
              tokens=[tok("X" * 44, USDC, A, 9_000_000, 4_000_000), tok("Y" * 44, USDC, B, 0, 5_000_000)], **extra)
    r["case"] = case
    return r


BUNDLED_APPROVE = bundled("ba", "bundled_transfer_approve",
                          authority=[{"account": "X" * 44, "field": "delegate", "pre": None, "post": C}],
                          token_accounts={"X" * 44: {"mint": USDC, "owner": A, "delegated_amount": UNLIMITED, "decimals": 6}})
BUNDLED_SET_OWNER = bundled("bs", "bundled_transfer_set_owner",
                            authority=[{"account": "X" * 44, "field": "owner", "pre": A, "post": C}],
                            token_accounts={"X" * 44: {"mint": USDC, "owner": A, "decimals": 6}})
BUNDLED_CLOSE = bundled("bc", "bundled_transfer_close", sol=[sol("X" * 44, RENT, 0), sol(C, 0, RENT)],
                        closed=["X" * 44], created=[C],
                        authority=[{"account": "X" * 44, "field": "programOwner", "pre": "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA", "post": "11111111111111111111111111111111"}],
                        token_accounts={"X" * 44: {"mint": USDC, "owner": A, "decimals": 6}})

TWIN = dict(TOKEN_TRANSFER, tx_digest="twin", source="synthetic", case="benign_transfer")
TWINS = [("twin", sd.derive_intent(TWIN))]


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
        out = sd.mutate(kind, sd.derive_intent(record), record, random.Random(3), twins=TWINS, assets=ASSETS)
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


class ShortcutTests(unittest.TestCase):
    def test_hidden_main_effect_becomes_a_benign_twin_request(self):
        for kind, r in (("undeclared_approval", APPROVE_UNLIMITED), ("undeclared_approval", APPROVE_BOUNDED),
                        ("undeclared_authority_change", SET_OWNER), ("undeclared_close", CLOSE_STRANGER)):
            intent, meta = sd.mutate(kind, sd.derive_intent(r), r, random.Random(5), twins=TWINS)
            twin = TWINS[0][1]
            self.assertEqual({k: intent[k] for k in ("action", "asset", "amount", "decimals", "recipient")},
                             {k: twin[k] for k in ("action", "asset", "amount", "decimals", "recipient")})
            self.assertNotEqual(intent["action"], "other")
            self.assertEqual(sd.label(intent, r, meta)[0], [kind], r["tx_digest"])

    def test_twin_is_never_the_record_itself_and_is_required(self):
        self.assertIsNone(sd.mutate("undeclared_approval", sd.derive_intent(APPROVE_UNLIMITED), APPROVE_UNLIMITED,
                                    random.Random(1), twins=[]))
        own = [(APPROVE_UNLIMITED["tx_digest"], sd.derive_intent(TOKEN_TRANSFER))]
        self.assertIsNone(sd.mutate("undeclared_approval", sd.derive_intent(APPROVE_UNLIMITED), APPROVE_UNLIMITED,
                                    random.Random(1), twins=own))

    def test_a_declared_transfer_that_does_not_happen_is_not_asset_mismatch_alone(self):
        intent = sd.derive_intent(TOKEN_TRANSFER)
        self.assertEqual(sd.label(intent, SET_OWNER, {})[0], ["undeclared_authority_change"])

    def test_other_never_carries_an_amount(self):
        splits, _ = sd.build(ShortcutTests.mix(), "risk", seed=11)
        for row in [r for v in splits.values() for r in v]:
            intent = json.loads(row["example"]["messages"][1]["content"])["intent"]
            if intent["action"] == "other":
                self.assertEqual((intent["amount"], intent["asset"]), (None, None))

    @staticmethod
    def mix():
        base = [SOL_TRANSFER, TOKEN_TRANSFER, SWAP, APPROVE_BOUNDED, APPROVE_UNLIMITED, SET_OWNER, CLOSE_STRANGER,
                CLOSE_BACK, TWIN, rec("fee", sol=[sol(A, 10_000, 10_000 - FEE)]),
                rec("in", tokens=[tok("Z" * 44, MINT, A, 0, 5)]), BUNDLED_APPROVE, BUNDLED_SET_OWNER, BUNDLED_CLOSE]
        return [rekey(dict(base[i % len(base)], tx_digest=f"m{i:03d}"), i) for i in range(len(base) * 20)]

    def test_no_action_has_a_degenerate_risk_ratio(self):
        _, stats = sd.build(self.mix(), "risk", seed=11)
        ratios = stats["risk_by_action"]
        self.assertIn("other", ratios)
        for action, r in ratios.items():
            if r["n"] >= 20:
                self.assertTrue(0.15 <= r["high_ratio"] <= 0.85, (action, r))
        self.assertEqual(stats["warnings"], [])

    def test_degenerate_ratios_are_warned(self):
        w = sd.ratio_warnings({"approve": {"n": 40, "high": 39, "high_ratio": 0.975}, "x": {"n": 5, "high": 5, "high_ratio": 1.0}})
        self.assertEqual(len(w), 1)
        self.assertIn("approve", w[0])

    def test_test_split_is_written_per_source(self):
        import tempfile
        from pathlib import Path
        with tempfile.TemporaryDirectory() as d:
            eff = Path(d) / "e.jsonl"
            eff.write_text("\n".join(json.dumps(r) for r in self.mix()) + "\n")
            sd.main(["--effects", str(eff), "--out", str(Path(d) / "ds"), "--seed", "11"])
            out = Path(d) / "ds"
            test = [json.loads(l) for l in (out / "test.jsonl").read_text().splitlines()]
            tagged = [json.loads(l) for l in (out / "test_by_source.jsonl").read_text().splitlines()]
            mainnet = (out / "test_mainnet.jsonl").read_text().splitlines()
            synthetic = (out / "test_synthetic.jsonl").read_text().splitlines()
            self.assertEqual(len(tagged), len(test))
            self.assertEqual(len(mainnet) + len(synthetic), len(test))
            self.assertEqual({t["source"] for t in tagged}, {"mainnet", "synthetic"})
            self.assertEqual([t["messages"] for t in tagged], [t["messages"] for t in test])
            stats = json.loads((out / "stats.json").read_text())
            self.assertEqual(stats["splits"]["test"]["by_source"]["mainnet"], len(mainnet))


class ReviewTests(unittest.TestCase):
    """Whole-dataset properties from the branch review."""

    @classmethod
    def setUpClass(cls):
        cls.splits, cls.stats = sd.build(ShortcutTests.mix(), "risk", seed=11)
        cls.rows = [(name, r) for name, v in cls.splits.items() for r in v]

    @staticmethod
    def parts(row):
        user = json.loads(row["example"]["messages"][1]["content"])
        answer = json.loads(row["example"]["messages"][2]["content"])
        return user["intent"], user["metadata"], answer

    def test_closing_a_token_account_is_not_an_authority_change(self):
        i = sd.derive_intent(CLOSE_BACK)
        self.assertFalse(any(e.startswith("authority_change:") for e in i["allowed_effects"]))
        self.assertIsNone(sd.mutate("undeclared_authority_change", i, CLOSE_BACK, random.Random(1), twins=TWINS))
        for _, row in self.rows:
            _, _, answer = self.parts(row)
            if "undeclared_authority_change" in answer["signals"]:
                self.assertNotIn("programOwner", answer["reasons"][0])

    def test_approve_amounts_are_in_decimal_units(self):
        i = sd.derive_intent(APPROVE_DECIMALS)
        self.assertEqual((i["amount"], i["decimals"]), ("2.5", 6))
        m, meta = sd.mutate("approval_exceeds_intent", i, APPROVE_DECIMALS, random.Random(2))
        signals, reasons = sd.label(m, APPROVE_DECIMALS, meta)
        self.assertEqual(signals, ["approval_exceeds_intent"])
        self.assertIn("2.5", reasons[0])

    def test_asset_mismatch_rewrites_the_declared_outflow(self):
        i = sd.derive_intent(TOKEN_TRANSFER)
        for seed in range(10):
            m, _ = sd.mutate("asset_mismatch", i, TOKEN_TRANSFER, random.Random(seed), assets=ASSETS)
            self.assertNotIn("token_out:" + USDC, m["allowed_effects"])
            want = "sol_out" if m["asset"] == "SOL" else "token_out:" + m["asset"]
            self.assertIn(want, m["allowed_effects"])
            self.assertEqual(m["decimals"], dict(ASSETS)[m["asset"]])
            self.assertEqual(m["action"], "transfer_sol" if m["asset"] == "SOL" else "transfer_token")

    def test_asset_mismatch_needs_a_pool_and_rescales(self):
        self.assertIsNone(sd.mutate("asset_mismatch", sd.derive_intent(TOKEN_TRANSFER), TOKEN_TRANSFER, random.Random(1)))
        m, _ = sd.mutate("asset_mismatch", sd.derive_intent(SWAP), SWAP, random.Random(1), assets=[(JUPLIKE, 5)])
        self.assertEqual((m["asset"], m["decimals"], m["action"]), (JUPLIKE, 5, "swap"))
        m, _ = sd.mutate("asset_mismatch", dict(sd.derive_intent(SWAP), amount="0.180780727", decimals=9), SWAP,
                         random.Random(1), assets=[(JUPLIKE, 5)])
        self.assertEqual(m["amount"], "0.18078")
        m, _ = sd.mutate("asset_mismatch", sd.derive_intent(SOL_TRANSFER), SOL_TRANSFER, random.Random(1), assets=[(MINT, 0)])
        self.assertEqual((m["action"], m["amount"], m["decimals"]), ("transfer_token", "1", 0))
        m, _ = sd.mutate("asset_mismatch", sd.derive_intent(TOKEN_TRANSFER), TOKEN_TRANSFER, random.Random(1), assets=[("SOL", 9)])
        self.assertEqual((m["action"], m["amount"], m["decimals"]), ("transfer_sol", "2.5", 9))

    def test_amounts_fit_their_decimals_and_actions_fit_their_asset(self):
        for _, row in self.rows:
            intent, _, _ = self.parts(row)
            if intent["amount"] not in (None, "unlimited") and intent["decimals"] is not None:
                places = len(intent["amount"].split(".")[1]) if "." in intent["amount"] else 0
                self.assertLessEqual(places, intent["decimals"], intent)
            if intent["action"] in ("transfer_sol", "transfer_token"):
                self.assertEqual(intent["action"] == "transfer_sol", intent["asset"] == "SOL", intent)

    def test_substitute_assets_come_from_honest_intents_in_the_same_split(self):
        honest_assets = {}
        for name, row in self.rows:
            intent, _, answer = self.parts(row)
            if not answer["signals"] and intent["asset"]:
                honest_assets.setdefault(name, set()).add(intent["asset"])
        found = 0
        for name, row in self.rows:
            intent, _, answer = self.parts(row)
            if "asset_mismatch" in answer["signals"]:
                found += 1
                self.assertIn(intent["asset"], honest_assets.get(name, set()))
        self.assertGreater(found, 0)

    def test_injection_reaches_transfer_and_bundled_cases(self):
        case = {r["tx_digest"]: r.get("case") for r in ShortcutTests.mix()}
        hit = set()
        for _, row in self.rows:
            _, _, answer = self.parts(row)
            if "injected_instruction" in answer["signals"]:
                hit.add(case[row["tx_digest"]])
        for c in ("benign_transfer", "bundled_transfer_approve", "bundled_transfer_set_owner", "bundled_transfer_close"):
            self.assertIn(c, hit)

    def test_declared_asset_is_always_in_allowed_effects(self):
        for _, row in self.rows:
            intent, _, _ = self.parts(row)
            if intent["action"] in ("transfer_sol", "transfer_token", "swap"):
                want = "sol_out" if intent["asset"] == "SOL" else "token_out:" + intent["asset"]
                self.assertIn(want, intent["allowed_effects"], intent)

    def test_metadata_is_per_transaction_and_on_both_label_sides(self):
        by_tx = {}
        for _, row in self.rows:
            _, meta, _ = self.parts(row)
            by_tx.setdefault(row["tx_digest"], set()).add(tuple(sorted(meta)))
        self.assertTrue(all(len(keys) == 1 for keys in by_tx.values()), "one field choice per transaction")
        sides = {}
        for _, row in self.rows:
            _, meta, answer = self.parts(row)
            key = next(iter(meta), "<empty>")
            sides.setdefault(key, Counter())[answer["risk"]] += 1
        self.assertEqual(set(sides), set(sd.METADATA_FIELDS) | {"<empty>"})
        for key, c in sides.items():
            if sum(c.values()) >= 20:
                self.assertTrue(c["low"] > 0 and c["high"] > 0, (key, c))

    def test_bundled_cases_supply_undeclared_without_twins(self):
        for kind, r in (("undeclared_approval", BUNDLED_APPROVE), ("undeclared_authority_change", BUNDLED_SET_OWNER),
                        ("undeclared_close", BUNDLED_CLOSE)):
            honest = sd.derive_intent(r)
            self.assertEqual(honest["action"], "transfer_token")
            self.assertEqual(sd.label(honest, r, {}), ([], []))
            m, meta = sd.mutate(kind, honest, r, random.Random(1), twins=[])
            self.assertEqual(m["action"], "transfer_token")
            self.assertEqual(sd.label(m, r, meta)[0], [kind])
        twinned = [r for _, r in self.rows if r.get("twin")]
        self.assertEqual(twinned, [], "bundled records cover every undeclared kind, so no twin is needed")

    def test_twins_stay_in_their_split(self):
        recs = [r for r in ShortcutTests.mix() if not r.get("case", "").startswith("bundled")]
        splits, _ = sd.build(recs, "risk", seed=11)
        twinned = [(name, r) for name, v in splits.items() for r in v if r.get("twin")]
        self.assertTrue(twinned)
        for name, r in twinned:
            self.assertEqual(sd.split_of(r["twin"], 11), name)

    def test_reserved_injections_appear_only_in_test(self):
        reserved = set(sd.TEST_ONLY_INJECTIONS)
        seen = Counter()
        for name, row in self.rows:
            _, meta, answer = self.parts(row)
            if "injected_instruction" in answer["signals"]:
                text = next(iter(meta.values()))
                seen[name, text in reserved] += 1
        self.assertEqual(seen[("train", True)] + seen[("valid", True)], 0)
        self.assertEqual(seen[("test", False)], 0)
        self.assertGreater(seen[("train", False)], 0)


if __name__ == "__main__":
    unittest.main()
