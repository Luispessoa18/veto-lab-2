"""Benchmark Solana em camadas com casos adversariais gerados por src/solana_case_generator.py.

O RPC é um mock local: nenhuma transação é assinada ou enviada a uma blockchain.
A API recebe só o payload (ID opaco); o gabarito fica em results/solana_answers.jsonl,
lido apenas por este processo, e o resultado de cada simulação vem de um registro em memória.
"""
import argparse
import hashlib
import json
import secrets
import threading
import time
from collections import Counter
from datetime import datetime, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import urlsplit

import requests

from . import solana_case_generator as generator

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "results" / "solana_attack_simulation.jsonl"
REPORT = ROOT / "results" / "solana_attack_report.json"
BLACKLIST_PATH = ROOT / "config" / "blacklist.json"
BASE58 = generator.BASE58
SIM_ERRORS = {
    "slippage": ({"InstructionError": [2, {"Custom": 6001}]}, "Program log: Error: Slippage tolerance exceeded"),
    "insufficient_funds": ({"InstructionError": [1, {"Custom": 1}]}, "Program log: Error: insufficient funds"),
    "custom_program_error": ({"InstructionError": [0, {"Custom": 3012}]}, "Program log: AnchorError: AccountNotInitialized"),
}


def synthetic_address(index):
    digest = hashlib.sha256(f"veto-solana-blacklist:{index}".encode()).digest()
    return "".join(BASE58[value % len(BASE58)] for value in digest)[:32]


def ensure_benchmark_blacklist(count=20):
    generated = [synthetic_address(index) for index in range(count)]
    try:
        data = json.loads(BLACKLIST_PATH.read_text(encoding="utf-8"))
    except (FileNotFoundError, json.JSONDecodeError):
        data = {}
    solana = data.setdefault("solana", {})
    accounts = solana.setdefault("accounts", [])
    solana["accounts"] = list(dict.fromkeys([*accounts, *generated]))
    BLACKLIST_PATH.write_text(json.dumps(data, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return generated


class MockSolanaRPC(BaseHTTPRequestHandler):
    """simulateTransaction falso: o resultado de cada transação vem de um registro em memória
    (token opaco -> resultado), invisível para a API."""
    registry = {}

    def do_POST(self):
        size = int(self.headers.get("Content-Length", "0"))
        body = json.loads(self.rfile.read(size) or b"{}")
        token = (body.get("params") or [""])[0]
        outcome = self.registry.get(token, "unknown")
        if body.get("method") != "simulateTransaction":
            response = {"jsonrpc": "2.0", "id": body.get("id"), "error": {"code": -32601, "message": "method not found"}}
        elif outcome in SIM_ERRORS:
            err, log = SIM_ERRORS[outcome]
            response = {"jsonrpc": "2.0", "id": body.get("id"), "result": {"value": {
                "err": err, "logs": [log], "unitsConsumed": 18450}}}
        elif outcome == "unknown":
            response = {"jsonrpc": "2.0", "id": body.get("id"), "error": {"code": -32602, "message": "invalid transaction"}}
        else:
            response = {"jsonrpc": "2.0", "id": body.get("id"), "result": {"value": {
                "err": None, "logs": ["Program log: Instruction: Transfer"], "unitsConsumed": 12100}}}
        data = json.dumps(response).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, format, *args):
        pass


def layer_map(response):
    return {layer.get("layer"): layer for layer in response.get("layers", [])}


def decision_metrics(rows, decision_getter):
    usable = [(row, decision_getter(row)) for row in rows]
    usable = [(row, decision) for row, decision in usable if decision in ("ALLOW", "BLOCK")]
    tp = sum(row["expected_decision"] == "BLOCK" and decision == "BLOCK" for row, decision in usable)
    tn = sum(row["expected_decision"] == "ALLOW" and decision == "ALLOW" for row, decision in usable)
    fp = sum(row["expected_decision"] == "ALLOW" and decision == "BLOCK" for row, decision in usable)
    fn = sum(row["expected_decision"] == "BLOCK" and decision == "ALLOW" for row, decision in usable)
    return {"cases": len(usable), "true_positive": tp, "true_negative": tn,
            "false_positive": fp, "false_negative": fn,
            "accuracy": round((tp + tn) / len(usable), 4) if usable else None,
            "precision_block": round(tp / (tp + fp), 4) if tp + fp else None,
            "recall_block": round(tp / (tp + fn), 4) if tp + fn else None,
            "false_positive_rate": round(fp / (fp + tn), 4) if fp + tn else None}


def detailed_report(rows, generation=None):
    decisions = Counter(row["response"].get("decision", "ERROR") for row in rows)
    blocked_by = Counter(row["response"].get("blocked_by") or "not_blocked" for row in rows)
    attack_types, ai_findings, prompt_findings = {}, [], []
    for row in rows:
        response = row["response"]
        layers = layer_map(response)
        bucket = attack_types.setdefault(row["attack_type"], {"total": 0, "expected": Counter(),
                                                               "actual": Counter(), "blocked_by": Counter()})
        bucket["total"] += 1
        bucket["expected"][row["expected_decision"]] += 1
        bucket["actual"][response.get("decision", "ERROR")] += 1
        bucket["blocked_by"][response.get("blocked_by") or "not_blocked"] += 1
        layer1 = layers.get("layer_1")
        if layer1 and layer1.get("decision") == "BLOCK":
            ai_findings.append({"case_id": row["case_id"], "attack_type": row["attack_type"],
                                "reason": layer1.get("reason"),
                                "model_reasons": layer1.get("parsed", {}).get("reasons", []),
                                "evidence_fields": layer1.get("parsed", {}).get("evidence_fields", []),
                                "policy_conflicts": layer1.get("conflicts", [])})
        guard = layers.get("prompt_guard")
        if guard and guard.get("decision") == "BLOCK":
            prompt_findings.append({"case_id": row["case_id"], "attack_type": row["attack_type"],
                                    "score": guard.get("prompt_guard", {}).get("malicious_score")})
    by_type = {key: {**value, "expected": dict(value["expected"]), "actual": dict(value["actual"]),
                     "blocked_by": dict(value["blocked_by"])} for key, value in sorted(attack_types.items())}
    attacks = [row for row in rows if row["expected_decision"] == "BLOCK"]
    correct = sum(row["response"].get("decision") == row["expected_decision"] for row in rows)
    def model_decision(row):
        return layer_map(row["response"]).get("layer_1", {}).get("model_decision")
    overrides = Counter(layer_map(row["response"]).get("layer_1", {}).get("policy_override")
                        for row in rows if layer_map(row["response"]).get("layer_1", {}).get("policy_override"))
    split_metrics = {split: decision_metrics([row for row in rows if row.get("split") == split],
                                              lambda row: row["response"].get("decision"))
                     for split in ("development", "holdout")}
    component_attribution = Counter()
    for row in rows:
        layers = layer_map(row["response"])
        blocked_by_layer = row["response"].get("blocked_by")
        if blocked_by_layer == "layer_1" and layers.get("layer_1", {}).get("reason") == "SOLANA_POLICY_CONFLICT":
            component_attribution["solana_policy"] += 1
        elif blocked_by_layer == "layer_1":
            component_attribution["llm_or_override"] += 1
        elif blocked_by_layer:
            component_attribution[blocked_by_layer] += 1
        else:
            component_attribution["not_blocked"] += 1
    decisions_by_id = {row["case_id"]: row["response"].get("decision") for row in rows}
    metamorphic_rows = [row for row in rows if row.get("metamorphic_parent")]
    metamorphic_consistent = sum(decisions_by_id.get(row["metamorphic_parent"]) ==
                                 row["response"].get("decision") for row in metamorphic_rows)
    return {"generated_at": datetime.now(timezone.utc).isoformat(), "source": "src/solana_case_generator.py",
            "generation": generation or {},
            "cases": len(rows), "expected": dict(Counter(row["expected_decision"] for row in rows)),
            "decisions": dict(decisions), "blocked_by": dict(blocked_by),
            "accuracy": round(correct / len(rows), 4) if rows else None,
            "metrics": {
                "pipeline": decision_metrics(rows, lambda row: row["response"].get("decision")),
                "raw_model": decision_metrics(rows, model_decision),
                "by_split": split_metrics,
                "policy_overrides": dict(overrides),
                "component_attribution": dict(component_attribution),
                "metamorphic": {"cases": len(metamorphic_rows),
                                "consistent": metamorphic_consistent,
                                "consistency_rate": round(metamorphic_consistent / len(metamorphic_rows), 4)
                                if metamorphic_rows else None},
                "baselines": {
                    "always_allow": decision_metrics(rows, lambda row: "ALLOW"),
                    "always_block": decision_metrics(rows, lambda row: "BLOCK")}},
            "funnel": {
                "entered": len(rows),
                "passed_blacklist": sum("prompt_guard" in layer_map(row["response"]) for row in rows),
                "passed_prompt_guard": sum("layer_1" in layer_map(row["response"]) for row in rows),
                "reached_simulation": sum("solana_simulation" in layer_map(row["response"]) for row in rows),
                "simulation_allowed": sum(layer_map(row["response"]).get("solana_simulation", {}).get("decision") == "ALLOW" for row in rows),
                "simulation_blocked": sum(layer_map(row["response"]).get("solana_simulation", {}).get("decision") == "BLOCK" for row in rows)},
            "attack_detection": {"attacks": len(attacks),
                "blocked": sum(row["response"].get("decision") == "BLOCK" for row in attacks),
                "missed": [row["case_id"] for row in attacks if row["response"].get("decision") != "BLOCK"]},
            "infrastructure_errors": [
                {"case_id": row["case_id"], "attack_type": row["attack_type"],
                 "http_status": row.get("http_status"), "error": row["response"].get("error")}
                for row in rows if row["response"].get("decision") == "ERROR"],
            "ai_detections": ai_findings, "prompt_guard_detections": prompt_findings,
            "by_attack_type": by_type}


ROW_KEYS = ("attack_type", "expected_decision", "expected_layer", "split", "text_source", "encoding",
            "metamorphic_parent", "token", "mint", "amount", "ui_amount")


def main():
    parser = argparse.ArgumentParser(description="Gera casos adversariais e executa o pipeline Solana")
    parser.add_argument("--api", default="http://127.0.0.1:8070")
    parser.add_argument("--count", "--limit", dest="count", type=int, default=600,
                        help="transações geradas (fora os irmãos metamórficos)")
    parser.add_argument("--seed", type=int, default=None, help="seed reproduzível; sem ela, uma aleatória")
    parser.add_argument("--attack-ratio", type=float, default=0.5)
    parser.add_argument("--metamorphic-rate", type=float, default=0.15)
    parser.add_argument("--prepared", type=Path, default=None,
                        help="reexecuta um solana_cases.jsonl já gerado (o gabarito ao lado)")
    parser.add_argument("--output", type=Path, default=OUT)
    parser.add_argument("--report", type=Path, default=REPORT)
    parser.add_argument("--prepare-only", action="store_true", help="só gera casos e blacklist")
    args = parser.parse_args()
    blacklist = ensure_benchmark_blacklist()
    if args.prepared:
        answers_path = args.prepared.with_name(generator.ANSWERS_OUT.name)
        if not args.prepared.exists() or not answers_path.exists():
            parser.error(f"casos ou gabarito ausentes ({args.prepared}, {answers_path}); execute 03_GERAR_CASOS.bat")
        cases = generator.read(args.prepared, answers_path)
        seed = None
    else:
        seed = args.seed if args.seed is not None else secrets.randbits(63)
        raw_cases, answers = generator.generate(args.count, seed, blacklist, args.attack_ratio,
                                                max(0.0, min(args.metamorphic_rate, 1.0)))
        generator.write(raw_cases, answers)
        cases = generator.read()
    if args.prepare_only:
        print(json.dumps({"cases": len(cases), "payloads": str(generator.CASES_OUT),
                          "answers": str(generator.ANSWERS_OUT), "seed": seed,
                          "expected": dict(Counter(case["expected_decision"] for case in cases)),
                          "attack_types": dict(Counter(case["attack_type"] for case in cases)),
                          "metamorphic_cases": sum("metamorphic_parent" in case for case in cases),
                          "blacklist_accounts": len(blacklist)}, ensure_ascii=False, indent=2))
        return
    MockSolanaRPC.registry = {case["sim_token"]: case["sim_result"] for case in cases}
    server = ThreadingHTTPServer(("127.0.0.1", 18999), MockSolanaRPC)
    threading.Thread(target=server.serve_forever, name="mock-solana-rpc", daemon=True).start()
    rows = []
    try:
        api_health = requests.get(args.api + "/health", timeout=5)
        api_health.raise_for_status()
        health_body = api_health.json()
        llm_url = health_body.get("llm_url")
        ablation_mode = health_body.get("ablation_mode", "full")
        if not llm_url:
            raise RuntimeError("API desatualizada ou sem llm_url; reinicie com 00_INICIAR_TUDO.bat")
        parsed_llm = urlsplit(llm_url)
        llm_health = requests.get(f"{parsed_llm.scheme}://{parsed_llm.netloc}/health", timeout=5)
        llm_health.raise_for_status()
        if llm_health.json().get("status") != "ok":
            raise RuntimeError("llama-server ainda não está pronto")
        for index, case in enumerate(cases, 1):
            started = time.perf_counter()
            try:
                # Só o payload e o ID opaco saem deste processo: o gabarito nunca vai para a API.
                response = requests.post(args.api + "/v1/transactions/verify", json=case["payload"],
                                         headers={"X-Request-ID": case["case_id"]}, timeout=180)
                body, status = response.json(), response.status_code
            except Exception as exc:
                body, status = {"decision": "ERROR", "error": str(exc)[:500]}, 0
            row = {"case_id": case["case_id"], **{key: case[key] for key in ROW_KEYS if key in case}}
            row.update({"timestamp": datetime.now(timezone.utc).isoformat(), "http_status": status,
                        "elapsed_ms": round((time.perf_counter() - started) * 1000, 2), "response": body})
            rows.append(row)
            print(f"[{index}/{len(cases)}] {case['case_id']} tipo={case['attack_type']} "
                  f"esperado={case['expected_decision']} obtido={body.get('decision')} "
                  f"camada={body.get('blocked_by') or 'fim'}", flush=True)
    finally:
        server.shutdown()
        server.server_close()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("w", encoding="utf-8") as stream:
        for row in rows:
            stream.write(json.dumps(row, ensure_ascii=False, separators=(",", ":")) + "\n")
    report = detailed_report(rows, {"seed": seed, "generator": "adversarial-v2",
                                    "ablation_mode": ablation_mode,
                                    "splits": dict(Counter(case["split"] for case in cases))})
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    from .solana_html_report import write_html
    html_path = write_html(rows, report)
    print(json.dumps({"details": str(args.output), "report": str(args.report), "html": str(html_path),
                      "cases": len(rows), "decisions": report["decisions"], "blocked_by": report["blocked_by"],
                      "funnel": report["funnel"]}, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
