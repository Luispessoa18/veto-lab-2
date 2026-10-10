"""Leitura dos resultados de execuções em lote para o console (página Runs).

Uma execução é um arquivo JSONL com uma linha por caso. São aceitos dois formatos:

* ``results/runs/<run_id>.jsonl``: ``request_id``, ``attack_type``, ``expected``,
  ``decision``, ``blocked_by``, ``latency_ms``, ``timestamp``;
* ``results/solana_attack_simulation.jsonl`` (``09_SIMULAR_ATAQUES_SOLANA``):
  ``case_id``, ``attack_type``, ``expected_decision``, ``timestamp``,
  ``elapsed_ms`` e a resposta da API em ``response``.

Cada linha vira uma linha compacta (id, at, type, expected, decision, by, ms,
error; campos vazios são omitidos); a agregação fica no console. Linhas inválidas são contadas, não
descartadas em silêncio.
"""

import json
import re
from pathlib import Path

RUN_ID = re.compile(r"[A-Za-z0-9][A-Za-z0-9._-]{0,127}")
BENCHMARK_ID = "solana_attack_simulation"
MAX_ROWS = 200_000


def _files(root):
    results = Path(root) / "results"
    found = {path.stem: path for path in sorted((results / "runs").glob("*.jsonl"))}
    benchmark = results / f"{BENCHMARK_ID}.jsonl"
    if benchmark.exists() and BENCHMARK_ID not in found:
        found[BENCHMARK_ID] = benchmark
    return found


def list_runs(root):
    runs = []
    for run_id, path in _files(root).items():
        stat = path.stat()
        with path.open("rb") as stream:
            cases = sum(1 for line in stream if line.strip())
        runs.append({"id": run_id, "file": str(path.relative_to(root)), "bytes": stat.st_size,
                     "modified": stat.st_mtime, "cases": cases,
                     "source": "benchmark" if run_id == BENCHMARK_ID else "run"})
    runs.sort(key=lambda run: run["modified"], reverse=True)
    return {"runs": runs}


def _case_types(root):
    """attack_type/expected por case_id, para linhas antigas do benchmark sem esses campos."""
    path = Path(root) / "results" / "solana_cases.jsonl"
    if not path.exists():
        return {}
    known = {}
    with path.open(encoding="utf-8") as stream:
        for line in stream:
            try:
                case = json.loads(line)
            except json.JSONDecodeError:
                continue
            known[case.get("case_id")] = (case.get("attack_type"), case.get("expected_decision"))
    return known


def _number(value):
    try:
        return round(float(value), 2)
    except (TypeError, ValueError):
        return None


def normalize(row, known=None):
    response = row.get("response") if isinstance(row.get("response"), dict) else {}
    case_id = row.get("request_id") or row.get("case_id") or response.get("request_id")
    attack_type = row.get("attack_type")
    expected = row.get("expected") or row.get("expected_decision")
    if known and case_id in known and (attack_type is None or expected is None):
        attack_type = attack_type or known[case_id][0]
        expected = expected or known[case_id][1]
    latency = row.get("latency_ms", response.get("latency_ms"))
    compact = {
        "id": case_id,
        "at": row.get("timestamp"),
        "type": attack_type,
        "expected": expected,
        "decision": row.get("decision") or response.get("decision") or "ERROR",
        "by": row.get("blocked_by", response.get("blocked_by")),
        "ms": _number(latency if latency is not None else row.get("elapsed_ms")),
        "error": row.get("error") or response.get("error"),
    }
    return {key: value for key, value in compact.items() if value is not None}


def load_run(root, run_id):
    if not RUN_ID.fullmatch(run_id or ""):
        return None
    path = _files(root).get(run_id)
    if path is None:
        return None
    known = _case_types(root) if run_id == BENCHMARK_ID else None
    rows, skipped, truncated = [], 0, False
    with path.open(encoding="utf-8") as stream:
        for line in stream:
            if not line.strip():
                continue
            if len(rows) >= MAX_ROWS:
                truncated = True
                break
            try:
                data = json.loads(line)
            except json.JSONDecodeError:
                skipped += 1
                continue
            if not isinstance(data, dict):
                skipped += 1
                continue
            rows.append(normalize(data, known))
    return {"id": run_id, "file": str(path.relative_to(root)), "modified": path.stat().st_mtime,
            "rows": rows, "skipped": skipped, "truncated": truncated}
