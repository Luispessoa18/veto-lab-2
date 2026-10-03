"""Datasets públicos de prompt injection usados para avaliar e enriquecer o VETO.

- neuralchemy/Prompt-injection-dataset (Apache-2.0): splits validation/test salvos em
  fixtures/injection/ e versionados com atribuição.
- Necent/llm-jailbreak-prompt-injection-dataset (MIT, acesso gated): exige aceitar os
  termos no Hugging Face e um token (HF_TOKEN ou ~/.cache/huggingface/token). Fica só em
  external/ (fora do git); apenas métricas agregadas são publicadas.
"""
import argparse
import hashlib
import json
import os
import random
from collections import defaultdict
from pathlib import Path

import requests

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "fixtures" / "injection"
NEURALCHEMY = "neuralchemy/Prompt-injection-dataset"
NECENT = "Necent/llm-jailbreak-prompt-injection-dataset"
NECENT_DIR = ROOT / "external" / "datasets" / "necent"
NECENT_SAMPLE = ROOT / "external" / "datasets" / "necent_injection_sample.jsonl"
# Apenas injection/ofuscação: harmful_behavior e toxicity são moderação de conteúdo,
# fora do modelo de ameaça de um verificador de transações.
NECENT_TYPES = ("prompt_injection", "obfuscation")
MAX_TEXT = 2000


def neuralchemy_path(split):
    return FIXTURES / f"neuralchemy_core_{split}.jsonl"


def hf_token():
    token = os.environ.get("HF_TOKEN")
    if token:
        return token.strip()
    path = Path.home() / ".cache" / "huggingface" / "token"
    return path.read_text(encoding="utf-8").strip() if path.exists() else None


def write_jsonl(path, rows):
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8") as stream:
        for row in rows:
            stream.write(json.dumps(row, ensure_ascii=False, separators=(",", ":")) + "\n")


def read_jsonl(path):
    return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]


def download_neuralchemy(splits=("validation", "test")):
    for split in splits:
        rows, offset = [], 0
        while True:
            response = requests.get("https://datasets-server.huggingface.co/rows", timeout=60, params={
                "dataset": NEURALCHEMY, "config": "core", "split": split, "offset": offset, "length": 100})
            response.raise_for_status()
            page = [item["row"] for item in response.json()["rows"]]
            rows += [{"text": row["text"][:MAX_TEXT], "label": int(row["label"]), "category": row["category"],
                      "source": row["source"], "dataset": "neuralchemy"} for row in page]
            offset += len(page)
            if len(page) < 100:
                break
        write_jsonl(neuralchemy_path(split), rows)
        print(f"neuralchemy {split}: {len(rows)} linhas -> {neuralchemy_path(split)}")


def download_necent():
    token = hf_token()
    if not token:
        raise SystemExit("Defina HF_TOKEN (ou faça login no Hugging Face) e aceite os termos em "
                         f"https://huggingface.co/datasets/{NECENT}")
    NECENT_DIR.mkdir(parents=True, exist_ok=True)
    for index in range(4):
        name = f"train-{index:05d}-of-00004.parquet"
        target = NECENT_DIR / name
        if target.exists() and target.stat().st_size > 0:
            print(f"[OK] {name} já existe")
            continue
        url = f"https://huggingface.co/datasets/{NECENT}/resolve/main/data/{name}"
        with requests.get(url, headers={"Authorization": f"Bearer {token}"}, stream=True, timeout=120) as response:
            if response.status_code in (401, 403):
                raise SystemExit(f"Sem acesso ao dataset. Aceite os termos em https://huggingface.co/datasets/{NECENT}")
            response.raise_for_status()
            partial = target.with_suffix(".part")
            with partial.open("wb") as stream:
                for chunk in response.iter_content(1 << 20):
                    stream.write(chunk)
            partial.replace(target)
        print(f"[OK] {name}")


def sample_necent(per_group=300, seed=42):
    """Amostra estratificada por (fonte, rótulo) das linhas de injection do Necent."""
    import pyarrow.dataset as ds
    table = ds.dataset(NECENT_DIR, format="parquet").to_table(
        columns=["prompt", "prompt_type", "is_dangerous", "source", "attack_technique"],
        filter=ds.field("prompt_type").isin(list(NECENT_TYPES)))
    groups = defaultdict(list)
    for row in table.to_pylist():
        if row["prompt"]:
            groups[(row["source"], int(row["is_dangerous"]))].append(row)
    rng = random.Random(seed)
    rows = []
    for (source, label), items in sorted(groups.items()):
        for row in rng.sample(items, min(per_group, len(items))):
            rows.append({"text": row["prompt"][:MAX_TEXT], "label": label,
                         "category": row["attack_technique"] or row["prompt_type"],
                         "source": source, "dataset": "necent"})
    write_jsonl(NECENT_SAMPLE, rows)
    print(f"Necent: {len(rows)} linhas de {len(groups)} grupos -> {NECENT_SAMPLE}")


def necent_split(text):
    """O Necent não tem splits: metade fixa (por hash do texto) para calibrar, metade para medir."""
    return "validation" if int(hashlib.sha1(text.encode("utf-8")).hexdigest(), 16) % 2 == 0 else "test"


def load(dataset, split="test"):
    """Carrega um split local; lista vazia se o dataset ainda não foi baixado."""
    if dataset == "neuralchemy":
        path = neuralchemy_path(split)
        return read_jsonl(path) if path.exists() else []
    rows = read_jsonl(NECENT_SAMPLE) if NECENT_SAMPLE.exists() else []
    return [row for row in rows if necent_split(row["text"]) == split]


def main():
    parser = argparse.ArgumentParser(description="Baixa os datasets de prompt injection")
    parser.add_argument("command", choices=("neuralchemy", "necent", "all"))
    parser.add_argument("--per-group", type=int, default=300, help="linhas por (fonte, rótulo) na amostra Necent")
    args = parser.parse_args()
    if args.command in ("neuralchemy", "all"):
        download_neuralchemy()
    if args.command in ("necent", "all"):
        download_necent()
        sample_necent(args.per_group)


if __name__ == "__main__":
    main()
