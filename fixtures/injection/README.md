# Datasets de prompt injection

## neuralchemy_core_{validation,test}.jsonl

Splits `validation` e `test` da config `core` de
[neuralchemy/Prompt-injection-dataset](https://huggingface.co/datasets/neuralchemy/Prompt-injection-dataset),
licença **Apache-2.0**. Campos mantidos: `text` (cortado em 2000 caracteres), `label`
(1 = injection/jailbreak, 0 = legítimo), `category`, `source`.

- `test`: usado na avaliação (`11_AVALIAR_INJECTION.bat`) e nos casos Solana de prompt injection.
  As regras de `src/unified_api.py` não devem ser ajustadas olhando este split.
- `validation`: reservado para calibrar regras e limites.

Regerar: `.venv\Scripts\python.exe -m src.injection_datasets neuralchemy`

## Necent (não versionado)

[Necent/llm-jailbreak-prompt-injection-dataset](https://huggingface.co/datasets/Necent/llm-jailbreak-prompt-injection-dataset)
é gated: aceite os termos no Hugging Face e rode
`.venv\Scripts\python.exe -m src.injection_datasets necent` com `HF_TOKEN` definido (ou após
`huggingface-cli login`). Os arquivos ficam em `external/datasets/` (fora do git); só métricas
agregadas entram nos relatórios.
