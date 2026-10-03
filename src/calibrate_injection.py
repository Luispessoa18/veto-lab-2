"""Calibra os dois limites dos detectores anti-injection no split de validação.

- prompt_guard_threshold (quarentena + alerta, algum detector): maior detecção com falso
  positivo <= --max-fpr-alert. Errar aqui só gera alerta e tira o texto da IA.
- prompt_guard_block_threshold (bloqueio, consenso de todos os detectores): maior detecção
  com falso positivo <= --max-fpr-block. Errar aqui bloqueia transação legítima.
Grava os limites em config/settings.json e o detalhe em results/injection_calibration.json.
O split de teste não é usado aqui.
"""
import argparse
import json
import re
from datetime import datetime
from pathlib import Path

from .eval_injection import GuardScorer, apply_threshold, load_split, metrics, score, set_detectors

ROOT = Path(__file__).resolve().parents[1]
SETTINGS = ROOT / "config" / "settings.json"
OUT = ROOT / "results" / "injection_calibration.json"
GRID = [round(value / 100, 2) for value in range(1, 100)] + [0.995, 0.999]
REPORTED = (0.1, 0.3, 0.5, 0.7, 0.8, 0.9, 0.95, 0.99, 0.995)


def sweep(rows, key, max_fpr, threshold_for):
    """Testa cada limite do GRID; devolve a tabela e o de maior detecção dentro do FP aceito."""
    table, best = [], None
    for threshold in GRID:
        m = metrics(apply_threshold(rows, *threshold_for(threshold)), key)
        table.append({"threshold": threshold, **m})
        if m["false_positive_rate"] is not None and m["false_positive_rate"] <= max_fpr:
            if best is None or m["detected"] > best["detected"]:
                best = {"threshold": threshold, **m}
    return table, best


def write_setting(text, name, value):
    text, count = re.subn(rf'("{name}":\s*)[0-9.]+', rf"\g<1>{value}", text)
    if not count:
        raise SystemExit(f"{name} não encontrado em config/settings.json")
    return text


def main():
    parser = argparse.ArgumentParser(description="Calibra os limites dos detectores na validação")
    parser.add_argument("--max-fpr-alert", type=float, default=0.15, help="FP máximo da quarentena/alerta")
    parser.add_argument("--max-fpr-block", type=float, default=0.01, help="FP máximo do bloqueio por consenso")
    parser.add_argument("--dry-run", action="store_true", help="não altera config/settings.json")
    args = parser.parse_args()
    datasets = load_split("validation")
    if not datasets:
        parser.error("nenhum dataset local; rode: .venv\\Scripts\\python.exe -m src.injection_datasets all")
    scorer = GuardScorer()
    set_detectors(scorer.models)
    rows, per_dataset = [], {}
    for name, (items, _) in datasets.items():
        print(f"{name}: {len(items)} textos de validação", flush=True)
        score(items, scorer)
        rows += items
        per_dataset[name] = items
    # Quarentena: só o limite de alerta varia (bloqueio desligado acima de 1).
    alert_table, alert = sweep(rows, "veto", args.max_fpr_alert, lambda t: (t, 1.01))
    # Bloqueio: regras + consenso dos detectores.
    block_table, block = sweep(rows, "veto_block", args.max_fpr_block, lambda t: (1.01, t))
    if alert is None or block is None:
        raise SystemExit("Nenhum limite atinge o FP pedido; aumente --max-fpr-alert/--max-fpr-block.")
    final = {name: {key: metrics(apply_threshold(items, alert["threshold"], block["threshold"]), key)
                    for key in ("veto_block", "veto")} for name, items in per_dataset.items()}
    result = {"generated_at": datetime.now().isoformat(), "detectors": scorer.models,
              "threshold": alert["threshold"], "block_threshold": block["threshold"],
              "max_fpr": args.max_fpr_alert, "max_fpr_block": args.max_fpr_block,
              "validation": alert, "validation_block": block, "validation_by_dataset": final,
              "sweep_alert": [row for row in alert_table if row["threshold"] in (*REPORTED, alert["threshold"])],
              "sweep_block": [row for row in block_table if row["threshold"] in (*REPORTED, block["threshold"])]}
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    for title, table_rows, chosen in (("Quarentena (algum detector)", result["sweep_alert"], alert),
                                      ("Bloqueio (regras + consenso)", result["sweep_block"], block)):
        print(f"\n{title}\nlimite  detecção  FP")
        for row in table_rows:
            mark = "  <- escolhido" if row["threshold"] == chosen["threshold"] else ""
            print(f"{row['threshold']:<7} {row['recall']:<9} {row['false_positive_rate']}{mark}")
    for name, m in final.items():
        print(f"{name}: bloqueio {m['veto_block']['recall']} (FP {m['veto_block']['false_positive_rate']}) · "
              f"bloqueio+quarentena {m['veto']['recall']} (FP {m['veto']['false_positive_rate']})")
    if not args.dry_run:
        text = SETTINGS.read_text(encoding="utf-8")
        text = write_setting(text, "prompt_guard_threshold", alert["threshold"])
        text = write_setting(text, "prompt_guard_block_threshold", block["threshold"])
        SETTINGS.write_text(text, encoding="utf-8")
        print(f"prompt_guard_threshold = {alert['threshold']}, prompt_guard_block_threshold = "
              f"{block['threshold']} gravados em {SETTINGS}")


if __name__ == "__main__":
    main()
