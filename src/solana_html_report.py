"""Relatório HTML imprimível do benchmark Solana.

Lê results/solana_attack_simulation.jsonl e gera results/relatorio_solana.html,
com falsos positivos, resumo e uma seção por tipo de ataque.
"""
import argparse
import html
import json
import statistics
import webbrowser
from functools import lru_cache
from collections import Counter
from datetime import datetime
from pathlib import Path

import requests

from .solana_attack_simulator import OUT, ROOT, detailed_report, layer_map

HTML_OUT = ROOT / "results" / "relatorio_solana.html"
LAMPORTS_PER_SOL = 1_000_000_000
PAGE_CSS = """:root{--fg:#1d2330;--muted:#5d6675;--line:#d9dee6;--bg:#fff;--soft:#f4f6f9;--good:#1f7a4d;--bad:#b3261e;--accent:#2f5bd3}
*{box-sizing:border-box} body{margin:0;background:var(--bg);color:var(--fg);font:14px/1.45 system-ui,Segoe UI,Roboto,sans-serif}
main{max-width:1040px;margin:0 auto;padding:24px 16px 48px}
header{display:flex;justify-content:space-between;gap:16px;align-items:flex-start;flex-wrap:wrap;border-bottom:2px solid var(--fg);padding-bottom:12px}
h1{margin:0;font-size:24px} h2{font-size:19px;margin:28px 0 4px;border-bottom:1px solid var(--line);padding-bottom:4px}
h2 small{font-size:12px;color:var(--muted);font-weight:normal} h3{font-size:14px;margin:18px 0 6px}
.meta,.lead{color:var(--muted);margin:4px 0}
button{font:inherit;padding:8px 16px;border:1px solid var(--accent);background:var(--accent);color:#fff;border-radius:6px;cursor:pointer}
.kpis{display:grid;grid-template-columns:repeat(auto-fit,minmax(170px,1fr));gap:10px;margin:12px 0}
.kpi{border:1px solid var(--line);border-radius:8px;padding:10px 12px;background:var(--soft)}
.kpi b{display:block;font-size:22px} .kpi span{color:var(--muted);font-size:12px}
.kpi.good b{color:var(--good)} .kpi.bad b{color:var(--bad)}
table{width:100%;border-collapse:collapse;margin:4px 0 8px} th,td{text-align:left;padding:5px 8px;border-bottom:1px solid var(--line);vertical-align:top}
th{background:var(--soft);font-size:12px} table.small td{font-size:12px;word-break:break-word}
.bar{height:6px;background:var(--line);border-radius:3px;margin-top:3px;min-width:60px} .bar span{display:block;height:100%;background:var(--accent);border-radius:3px}
.wrap{overflow-x:auto} a{color:var(--accent)} .ok{color:var(--good);font-weight:600} .err{color:var(--bad);font-weight:600}
@media print{
  @page{size:A4;margin:14mm} body{font-size:11px} main{max-width:none;padding:0}
  .noprint{display:none} .page{break-before:page} table,tr,.kpis{break-inside:avoid}
  .bar span{print-color-adjust:exact;-webkit-print-color-adjust:exact} a{color:inherit;text-decoration:none}
}
"""

TYPE_INFO = {
    "blacklist_recipient": ("Destinatário na blacklist", "Usuário enganado envia para a carteira de um drainer conhecido."),
    "address_poisoning": ("Address poisoning", "Destinatário trocado por endereço com mesmo início e fim do legítimo."),
    "fake_mint": ("Token falso", "Mint imitando o token pedido (mesmo início/fim e símbolo)."),
    "split_drain": ("Dreno dividido", "Várias transferências, cada uma abaixo do limite, somando acima dele."),
    "hidden_drain_instruction": ("Instrução escondida", "Transferência legítima mais uma instrução extra que paga outra carteira."),
    "set_authority_drain": ("SetAuthority (drainer)", "Troca o dono da conta de token do usuário, padrão de drainer kits."),
    "close_account_drain": ("CloseAccount para terceiro", "Fecha a conta de token do usuário mandando o saldo para outra carteira."),
    "unlimited_approve": ("Approve ilimitado", "Delegate com valor u64 máximo para carteira desconhecida."),
    "system_drain": ("Dreno de SOL", "Transferência extra de SOL nativo para outra carteira."),
    "assign_owner": ("Assign de conta", "Muda o programa dono da conta do usuário."),
    "durable_nonce": ("Nonce durável", "Transação pré-assinada que pode ser executada depois por terceiro."),
    "permanent_delegate_mint": ("Permanent delegate (Token-2022)", "Mint com extensão que permite a terceiro mover os tokens a qualquer momento."),
    "swap_slippage_sandwich": ("Swap com slippage aberto", "minimum_amount_out quase zero: alvo de sandwich/MEV."),
    "swap_output_redirect": ("Swap redirecionado", "Saída do swap enviada para outra carteira."),
    "prompt_injection_evasive": ("Prompt injection evasivo", "Injection em campos que o filtro de texto não cobre."),
    "display_spoof": ("Exibição falsa", "Parâmetros mostram o destinatário certo, a instrução paga outro."),
    "benign_sol_transfer": ("Transferência de SOL", "Envio de SOL nativo dentro da intenção."),
    "benign_swap": ("Swap legítimo", "Swap via Jupiter com slippage dentro do autorizado."),
    "benign_split_within_limit": ("Transferência em partes", "Várias transferências legítimas somando dentro do limite."),
    "benign_close_own_account": ("Fechar conta própria", "CloseAccount devolvendo o saldo ao próprio usuário, autorizado."),
    "blacklist": ("Conta na blacklist", "Fee payer é uma conta conhecida como maliciosa."),
    "recipient_substitution": ("Troca de destinatário", "A instrução envia para um destinatário diferente do autorizado."),
    "program_substitution": ("Troca de programa", "A instrução chama um programa fora da lista permitida."),
    "amount_over_limit": ("Valor acima do limite", "O valor transferido excede o max_amount da intenção."),
    "delegate_approval": ("Aprovação de delegate", "A transação concede delegate sem autorização do usuário."),
    "ai_intent_mismatch": ("Divergência de intenção", "Os parâmetros declarados divergem da intenção do usuário."),
    "simulation_revert": ("Falha na simulação", "A simulação on-chain da transação reverte."),
    "prompt_injection": ("Prompt injection", "Metadados do contrato tentam manipular a IA para aprovar "
                                             "(frases fixas + ataques reais do dataset neuralchemy)."),
    "benign_transfer": ("Transferência legítima", "Transferência SPL normal dentro da intenção."),
    "benign_boundary_amount": ("Valor no limite exato", "Transferência legítima com valor igual ao max_amount."),
    "benign_security_text": ("Texto de segurança legítimo", "Metadados com avisos de segurança inofensivos."),
    "benign_authorized_delegate": ("Delegate autorizado", "Aprovação de delegate permitida pela intenção."),
    "benign_dataset_text": ("Texto legítimo (dataset)", "Metadado com texto legítimo real do dataset neuralchemy."),
}
CRITERIA = {
    "PROGRAM_NOT_AUTHORIZED": "Programa da instrução fora de intent.allowed_programs.",
    "DANGEROUS_ACTION_NOT_AUTHORIZED": "Ação perigosa (set_authority, approve_delegate, close_account, "
                                       "upgrade_program) ausente de intent.allowed_actions.",
    "RECIPIENT_MISMATCH": "Destinatário da instrução diferente de intent.recipient.",
    "MINT_MISMATCH": "Mint da instrução diferente de intent.mint.",
    "AMOUNT_EXCEEDS_LIMIT": "Valor da instrução maior que intent.max_amount.",
    "TOTAL_AMOUNT_EXCEEDS_LIMIT": "Soma das transferências para o mesmo destinatário e token maior que intent.max_amount.",
    "ALLOWANCE_EXCEEDS_CAP": "Approve acima de intent.allowance_cap (ou de max_amount, sem teto declarado).",
    "DELEGATE_MISMATCH": "Delegate do approve diferente de intent.delegate.",
    "MIN_OUT_BELOW_AUTHORIZED": "minimum_amount_out do swap abaixo de intent.min_amount_out (slippage aberto).",
    "DANGEROUS_MINT_EXTENSION": "Mint Token-2022 com permanent_delegate ou transfer_hook não autorizados.",
    "INPUT_MINT_MISMATCH": "Token de entrada do swap diferente do autorizado.",
    "OUTPUT_MINT_MISMATCH": "Token de saída do swap diferente do autorizado.",
}
OVERRIDES = {
    "DETERMINISTIC_CONSISTENCY": ("ALLOW", "Todas as comparações verificáveis (≥3) batem com a autorização, sem "
                                  "conflito de efeito e sem evidência inconclusiva: a decisão da IA é trocada por ALLOW."),
    "DETERMINISTIC_MISMATCH": ("BLOCK", "Há divergência entre o proposto e o autorizado, mas a IA não bloqueou: "
                               "a decisão é trocada por BLOCK."),
    "INCONCLUSIVE_CONTRACT_EVIDENCE": ("REVIEW", "Evidência do contrato inconclusiva e a IA aprovou: "
                                       "a decisão é trocada por REVIEW."),
}
LAYER_NAMES = {
    "blacklist": "Blacklist", "prompt_guard": "Prompt Guard", "layer_1": "Camada 1 (IA + política)",
    "solana_simulation": "Simulação Solana", "not_blocked": "Não bloqueado", None: "Não bloqueado",
}


def esc(value):
    return html.escape("" if value is None else str(value))


def pct(part, total):
    return f"{100 * part / total:.1f}%" if total else "—"


def latency(rows):
    values = sorted(row.get("elapsed_ms") or 0 for row in rows)
    if not values:
        return "—", "—"
    p95 = values[min(len(values) - 1, int(round(0.95 * (len(values) - 1))))]
    return f"{statistics.mean(values):.0f} ms", f"{p95:.0f} ms"


CRITERION_SHORT = {
    "PROGRAM_NOT_AUTHORIZED": "Programa não autorizado",
    "DANGEROUS_ACTION_NOT_AUTHORIZED": "Ação perigosa não autorizada",
    "RECIPIENT_MISMATCH": "Destinatário diferente do autorizado",
    "MINT_MISMATCH": "Token (mint) diferente do autorizado",
    "AMOUNT_EXCEEDS_LIMIT": "Valor acima do limite",
    "TOTAL_AMOUNT_EXCEEDS_LIMIT": "Soma das transferências acima do limite",
    "ALLOWANCE_EXCEEDS_CAP": "Approve acima do teto autorizado",
    "DELEGATE_MISMATCH": "Delegate diferente do autorizado",
    "MIN_OUT_BELOW_AUTHORIZED": "Slippage acima do autorizado",
    "DANGEROUS_MINT_EXTENSION": "Extensão perigosa no mint (Token-2022)",
    "INPUT_MINT_MISMATCH": "Token de entrada diferente do autorizado",
    "OUTPUT_MINT_MISMATCH": "Token de saída diferente do autorizado",
}


def short_field(field):
    return (field or "").replace("scenario.transaction.", "")


def causes(row, detail=True):
    """O que barrou o caso: critério + parâmetro, em texto legível."""
    response = row["response"]
    blocked_by = response.get("blocked_by")
    layer = layer_map(response).get(blocked_by or "", {})
    if blocked_by == "blacklist":
        return [f"Carteira na blacklist" + (f": {', '.join(layer.get('matches', []))}" if detail else "")]
    if blocked_by == "prompt_guard" and layer.get("steering"):
        decoded = sorted({method for match in layer["steering"] for method in match.get("decoded_by", [])})
        return ["Texto tenta ditar o veredito" + (f" (oculto em {', '.join(decoded)})" if decoded else "") + ": "
                + ", ".join(dict.fromkeys(rule["description"] for match in layer["steering"] for rule in match["rules"]))
                + (f" — parâmetro {short_field(layer['steering'][0]['field'])}: “{layer['steering'][0]['text']}”"
                   if detail else "")]
    if blocked_by == "prompt_guard":
        guard = layer.get("prompt_guard") or {}
        top = max(guard.get("segments") or [{}], key=lambda seg: seg.get("malicious_score") or 0)
        text = "Prompt injection nos metadados" + (" (detectores em consenso)" if top.get("scores") else "")
        if detail:
            text += f" ({detector_scores(top)} ≥ {guard.get('block_threshold', guard.get('threshold'))}): “{top.get('text_preview', '')}”"
        return [text]
    if blocked_by == "layer_1" and layer.get("conflicts"):
        return [f"{CRITERION_SHORT.get(c.get('reason'), c.get('reason'))}"
                + f" — parâmetro {short_field(c.get('field'))}" for c in layer["conflicts"]]
    if blocked_by == "layer_1":
        reasons = (layer.get("parsed") or {}).get("reasons") or []
        text = "IA bloqueou"
        if layer.get("policy_override") == "DETERMINISTIC_MISMATCH":
            text = "Divergência determinística (IA não bloqueou)"
        return [text + (": " + "; ".join(reasons[:3]) if detail and reasons else "")]
    if blocked_by == "solana_simulation":
        err = layer.get("simulation_error")
        logs = "; ".join(layer.get("logs") or [])
        return ["Simulação on-chain falhou" + (f": {json.dumps(err)} — {logs}" if detail and err else "")]
    if blocked_by:
        return [f"{layer.get('reason') or blocked_by}"]
    if response.get("decision") == "ERROR":
        return ["Erro: " + str(response.get("error"))]
    quarantined = layer_map(response).get("prompt_guard", {}).get("quarantined")
    if quarantined:
        text = "Aprovado com texto em quarentena"
        if detail:
            text += ": " + "; ".join(f"{short_field(q.get('field'))} (score {q.get('malicious_score')}): "
                                     f"“{q.get('text')}”" for q in quarantined)
        return [text]
    return ["Aprovado"]


def cause_counts(rows):
    return Counter(LAYER_NAMES.get(row["response"].get("blocked_by"), "") + ": " + cause
                   if row["response"].get("blocked_by") else cause
                   for row in rows for cause in causes(row, detail=False))


def block_reason(row):
    return " · ".join(causes(row))


def detector_scores(segment):
    """Score de cada detector anti-injection num texto (ou o score único dos resultados antigos)."""
    scores = segment.get("scores")
    if not scores:
        return f"score {segment.get('malicious_score')}"
    short = {"Llama-Prompt-Guard-2-86M": "PromptGuard2"}
    return " / ".join(f"{short.get(name, name)} {value}" for name, value in scores.items())


def miss_reason(row):
    """Por que um ataque passou: o que cada camada respondeu."""
    layers = layer_map(row["response"])
    parts = []
    guard = layers.get("prompt_guard", {}).get("prompt_guard") or {}
    if guard:
        top = max(guard.get("segments") or [{}], key=lambda seg: seg.get("malicious_score") or 0)
        limits = f"quarentena ≥ {guard.get('threshold')}" + (f", bloqueio ≥ {guard['block_threshold']}"
                                                              if guard.get("block_threshold") else "")
        parts.append(f"Detectores {detector_scores(top)} ({limits})"
                     + (f": “{top.get('text_preview')}”" if top.get("text_preview") else ""))
    layer1 = layers.get("layer_1")
    if layer1:
        parts.append(f"IA: {layer1.get('model_decision')} → final {layer1.get('decision')}"
                     + (f" ({layer1['policy_override']})" if layer1.get("policy_override") else ""))
    return " · ".join(parts) or esc(row["response"].get("error"))


def bar(part, total, cls=""):
    width = 100 * part / total if total else 0
    return f'<div class="bar {cls}"><span style="width:{width:.1f}%"></span></div>'


def table(headers, body_rows, cls=""):
    head = "".join(f"<th>{h}</th>" for h in headers)
    body = "".join("<tr>" + "".join(f"<td>{c}</td>" for c in row) + "</tr>" for row in body_rows)
    return f'<table class="{cls}"><thead><tr>{head}</tr></thead><tbody>{body}</tbody></table>'


def metric_rows(metrics):
    return [[esc(name), m["cases"], m["true_positive"], m["true_negative"], m["false_positive"],
             m["false_negative"], esc(m["accuracy"]), esc(m["precision_block"]), esc(m["recall_block"]),
             esc(m["false_positive_rate"])] for name, m in metrics]


def false_positive_section(rows):
    benign = [row for row in rows if row["expected_decision"] == "ALLOW"]
    llm_fp = [row for row in benign if layer_map(row["response"]).get("layer_1", {}).get("model_decision") == "BLOCK"]
    corrected = [row for row in llm_fp if row["response"].get("decision") == "ALLOW"]
    final_fp = [row for row in benign if row["response"].get("decision") == "BLOCK"]
    fp_by_layer = Counter(row["response"].get("blocked_by") for row in final_fp)
    by_type = []
    for kind in sorted({row["attack_type"] for row in benign}):
        group = [row for row in benign if row["attack_type"] == kind]
        g_llm = sum(row in llm_fp for row in group)
        g_corr = sum(row in corrected for row in group)
        g_fp = sum(row in final_fp for row in group)
        by_type.append([esc(type_label(kind)), len(group), g_llm, g_corr, g_fp, pct(g_fp, len(group))])
    fp_list = [[esc(row["case_id"]), esc(TYPE_INFO.get(row["attack_type"], (row["attack_type"],))[0]),
                esc(LAYER_NAMES.get(row["response"].get("blocked_by"), row["response"].get("blocked_by"))),
                esc(block_reason(row))] for row in final_fp]
    return f"""
<section class="page">
  <h2>Falsos positivos</h2>
  <p class="lead">Transações legítimas ({len(benign)}) que alguma camada tentou bloquear.</p>
  <div class="kpis">
    <div class="kpi"><b>{len(llm_fp)}</b><span>bloqueios indevidos sugeridos pela IA</span></div>
    <div class="kpi good"><b>{len(corrected)}</b><span>falsos positivos barrados pela política determinística</span></div>
    <div class="kpi bad"><b>{len(final_fp)}</b><span>falsos positivos que chegaram ao resultado final ({pct(len(final_fp), len(benign))})</span></div>
  </div>
  <h3>Por tipo de transação legítima</h3>
  {table(["Tipo", "Casos", "IA sugeriu BLOCK", "Barrados (corrigidos)", "FP final", "Taxa FP"], by_type)}
  <h3>Camada responsável pelos falsos positivos restantes</h3>
  {table(["Camada", "Casos"], [[esc(LAYER_NAMES.get(k, k)), v] for k, v in fp_by_layer.most_common()] or [["—", 0]])}
  <h3>Casos legítimos bloqueados</h3>
  {table(["Caso", "Tipo", "Camada", "Motivo"], fp_list, "small") if fp_list else "<p>Nenhum.</p>"}
</section>"""


def type_label(kind):
    return TYPE_INFO.get(kind, (kind,))[0]


def deterministic_section(rows):
    """Bloqueios da política pré-IA e correções aplicadas sobre a decisão da IA."""
    policy_blocks = [row for row in rows
                     if layer_map(row["response"]).get("layer_1", {}).get("reason") == "SOLANA_POLICY_CONFLICT"]
    by_criterion = {}
    for row in policy_blocks:
        for conflict in layer_map(row["response"])["layer_1"].get("conflicts", []):
            bucket = by_criterion.setdefault(conflict.get("reason"), {"cases": 0, "fields": Counter(),
                                                                       "types": Counter(), "wrong": 0})
            bucket["cases"] += 1
            bucket["fields"][conflict.get("field")] += 1
            bucket["types"][type_label(row["attack_type"])] += 1
            bucket["wrong"] += row["expected_decision"] == "ALLOW"
    criterion_rows = [[f"<b>{esc(CRITERION_SHORT.get(name, name))}</b><br><small>{esc(CRITERIA.get(name, ''))} "
                       f"<code>{esc(name)}</code></small>", b["cases"],
                       esc(", ".join(f"{k} ({v})" for k, v in b["types"].most_common())),
                       esc(", ".join(short_field(f) for f in b["fields"])), b["wrong"]]
                      for name, b in sorted(by_criterion.items(), key=lambda item: -item[1]["cases"])]
    multi = sum(len(layer_map(row["response"])["layer_1"].get("conflicts", [])) > 1 for row in policy_blocks)

    overridden = [row for row in rows if layer_map(row["response"]).get("layer_1", {}).get("policy_override")]
    override_rows, impact_rows = [], []
    for name in sorted({layer_map(row["response"])["layer_1"]["policy_override"] for row in overridden}):
        group = [row for row in overridden if layer_map(row["response"])["layer_1"]["policy_override"] == name]
        target, rule = OVERRIDES.get(name, ("?", ""))
        from_decisions = Counter(layer_map(row["response"])["layer_1"].get("model_decision") for row in group)
        fixed_fp = sum(row["expected_decision"] == "ALLOW" and row["response"].get("decision") == "ALLOW" for row in group)
        released = [row for row in group if row["expected_decision"] == "BLOCK" and
                    layer_map(row["response"])["layer_1"].get("decision") == "ALLOW"]
        caught_later = sum(row["response"].get("decision") == "BLOCK" for row in released)
        override_rows.append([f"<b>{esc(name)}</b><br><small>{esc(rule)}</small>", len(group),
                              esc(", ".join(f"{k}→{target} ({v})" for k, v in from_decisions.most_common())),
                              fixed_fp, len(released), caught_later, len(released) - caught_later])
        for kind in sorted({row["attack_type"] for row in group}, key=lambda k: (k.startswith("benign"), k)):
            sub = [row for row in group if row["attack_type"] == kind]
            malicious = sub[0]["expected_decision"] == "BLOCK"
            final = Counter(row["response"].get("decision") for row in sub)
            later = Counter(row["response"].get("blocked_by") for row in sub if row["response"].get("blocked_by"))
            if not malicious:
                effect = "Falso positivo da IA barrado"
            elif final.get("BLOCK", 0) == len(sub):
                effect = "Ataque liberado pela correção, mas pego depois por " + ", ".join(LAYER_NAMES.get(k, k) for k in later)
            else:
                effect = f"⚠ Ataque liberado pela correção; {final.get('ALLOW', 0)} chegaram aprovados no final"
            impact_rows.append([esc(name), esc(type_label(kind)), "Ataque" if malicious else "Legítimo", len(sub),
                                esc(", ".join(f"{k} ({v})" for k, v in final.most_common())), esc(effect)])
    return f"""
<section class="page">
  <h2>Política determinística</h2>
  <p class="lead">Regras fixas, sem IA: bloqueiam antes da IA quando a transação contradiz a intenção
  e corrigem a decisão da IA quando ela contradiz as comparações verificáveis.</p>
  <div class="kpis">
    <div class="kpi"><b>{len(policy_blocks)}</b><span>bloqueados pela política antes da IA ({multi} com mais de um critério)</span></div>
    <div class="kpi"><b>{len(overridden)}</b><span>decisões da IA corrigidas</span></div>
  </div>
  <h3>Bloqueios antes da IA, por critério</h3>
  {table(["Critério", "Casos", "Tipos de ataque", "Campo verificado", "Legítimos bloqueados"], criterion_rows) if criterion_rows else "<p>Nenhum.</p>"}
  <h3>Correções sobre a decisão da IA</h3>
  {table(["Regra", "Casos", "IA → final", "FP barrados", "Ataques liberados", "…pegos depois", "…aprovados no final"], override_rows) if override_rows else "<p>Nenhuma.</p>"}
  <h3>Impacto das correções por tipo</h3>
  {table(["Regra", "Tipo", "Classe", "Casos", "Decisão final", "Efeito"], impact_rows, "small") if impact_rows else "<p>Nenhum.</p>"}
</section>"""


def deterministic_rows(group):
    """Critérios e correções determinísticas que atuaram num tipo de caso."""
    counter = Counter()
    for row in group:
        layer1 = layer_map(row["response"]).get("layer_1", {})
        for conflict in layer1.get("conflicts", []):
            counter[("Bloqueio pré-IA", conflict.get("reason"))] += 1
        if layer1.get("policy_override"):
            counter[("Correção da IA", f"{layer1.get('model_decision')} → {layer1.get('decision')} "
                                       f"({layer1['policy_override']})")] += 1
    return [[esc(kind), esc(name), v] for (kind, name), v in counter.most_common()]


def attack_type_section(kind, group):
    title, description = TYPE_INFO.get(kind, (kind, ""))
    malicious = group[0]["expected_decision"] == "BLOCK"
    correct = [row for row in group if row["response"].get("decision") == row["expected_decision"]]
    wrong = [row for row in group if row["response"].get("decision") != row["expected_decision"]]
    layers = Counter(row["response"].get("blocked_by") or "not_blocked" for row in group)
    expected_layers = Counter(row.get("expected_layer") for row in group)
    avg, p95 = latency(group)
    label_ok, label_bad = ("Bloqueados", "Não detectados") if malicious else ("Liberados", "Bloqueados indevidamente")
    layer_rows = [[esc(LAYER_NAMES.get(k, k)), v, pct(v, len(group)) + bar(v, len(group))] for k, v in layers.most_common()]
    wrong_rows = [[esc(row["case_id"]), esc(row.get("split")), esc(row["response"].get("decision")),
                   esc(miss_reason(row) if malicious else block_reason(row))] for row in wrong]
    examples = [[esc(row["case_id"]), esc(LAYER_NAMES.get(row["response"].get("blocked_by"), "—")),
                 esc(block_reason(row) if malicious else "ALLOW")] for row in correct[:5]]
    return f"""
<section class="page">
  <h2>{esc(title)} <small>{esc(kind)} · {"ataque" if malicious else "legítimo"}</small></h2>
  <p class="lead">{esc(description)}</p>
  <div class="kpis">
    <div class="kpi"><b>{len(group)}</b><span>casos</span></div>
    <div class="kpi good"><b>{len(correct)}</b><span>{label_ok} ({pct(len(correct), len(group))})</span></div>
    <div class="kpi {"bad" if wrong else ""}"><b>{len(wrong)}</b><span>{label_bad}</span></div>
    <div class="kpi"><b>{avg}</b><span>latência média · p95 {p95}</span></div>
  </div>
  <p>Camada esperada: {", ".join(f"{esc(LAYER_NAMES.get(k, k))} ({v})" for k, v in expected_layers.items())}</p>
  <h3>O que impediu</h3>
  {table(["Critério e parâmetro", "Casos", "%"], [[esc(k), v, pct(v, len(group)) + bar(v, len(group))] for k, v in cause_counts(group).most_common()])}
  <h3>Decisão por camada</h3>
  {table(["Camada", "Casos", "%"], layer_rows)}
  <h3>Política determinística</h3>
  {table(["Etapa", "Critério / correção", "Casos"], det) if (det := deterministic_rows(group)) else "<p>Não atuou.</p>"}
  <h3>{label_bad}</h3>
  {table(["Caso", "Split", "Decisão", "Detalhe"], wrong_rows, "small") if wrong_rows else "<p>Nenhum.</p>"}
  <h3>Exemplos corretos</h3>
  {table(["Caso", "Camada", "Motivo"], examples, "small") if examples else "<p>Nenhum.</p>"}
</section>"""


COINGECKO_IDS = {"wSOL": "solana", "USDC": "usd-coin", "USDT": "tether", "BONK": "bonk", "JUP": "jupiter-exchange-solana"}


@lru_cache(maxsize=1)
def token_prices():
    """Preço em dólar de cada token: SOL fixo em settings.json (sol_usd_price) se definido;
    demais da CoinGecko; sem rede, usa os preços aproximados do gerador."""
    from .solana_case_generator import MINTS
    prices = {symbol: price for symbol, (_, _, price) in MINTS.items()}
    source = "preços aproximados do gerador (sem cotação)"
    try:
        response = requests.get("https://api.coingecko.com/api/v3/simple/price", timeout=8,
                                params={"ids": ",".join(COINGECKO_IDS.values()), "vs_currencies": "usd"})
        response.raise_for_status()
        body = response.json()
        prices.update({symbol: float(body[cid]["usd"]) for symbol, cid in COINGECKO_IDS.items() if cid in body})
        source = "cotação CoinGecko em " + datetime.now().strftime("%d/%m/%Y %H:%M")
    except (requests.RequestException, KeyError, TypeError, ValueError):
        pass
    try:
        fixed = json.loads((ROOT / "config" / "settings.json").read_text(encoding="utf-8")).get("sol_usd_price")
    except (OSError, json.JSONDecodeError):
        fixed = None
    if fixed:
        prices["wSOL"] = float(fixed)
        source += "; SOL fixo em config/settings.json"
    return prices, source


def br_number(value, decimals):
    return f"{value:,.{decimals}f}".replace(",", "_").replace(".", ",").replace("_", ".")


def row_usd(row):
    prices = token_prices()[0]
    if row.get("token") in prices and row.get("ui_amount") is not None:
        return row["ui_amount"] * prices[row["token"]]
    return (row.get("amount") or 0) / LAMPORTS_PER_SOL * prices["wSOL"]  # resultados antigos: lamports de wSOL


def total_usd(rows):
    return sum(row_usd(row) for row in rows)


def usd_text(value):
    return f"US$ {br_number(value, 2)}"


def decisions_section(rows):
    """Primeira visão: quantos BLOCK e ALLOW, esperado x obtido, e o valor envolvido."""
    def pick(expected=None, obtained=None):
        return [row for row in rows if (expected is None or row["expected_decision"] == expected)
                and (obtained is None or row["response"].get("decision") == obtained)]
    got_block, got_allow = pick(obtained="BLOCK"), pick(obtained="ALLOW")
    others = [row for row in rows if row["response"].get("decision") not in ("BLOCK", "ALLOW")]
    matrix = []
    for expected, label in (("BLOCK", "Ataques (deveriam ser BLOCK)"), ("ALLOW", "Legítimas (deveriam ser ALLOW)")):
        group = pick(expected=expected)
        b, a = pick(expected, "BLOCK"), pick(expected, "ALLOW")
        matrix.append([f"<b>{label}</b>", len(group),
                       f'<span class="{"ok" if expected == "BLOCK" else "err"}">{len(b)}</span>',
                       f'<span class="{"ok" if expected == "ALLOW" else "err"}">{len(a)}</span>',
                       len(group) - len(b) - len(a)])
    matrix.append(["<b>Total</b>", len(rows), f"<b>{len(got_block)}</b>", f"<b>{len(got_allow)}</b>", len(others)])
    has_value = any(row.get("ui_amount") is not None or row.get("amount") is not None for row in rows)
    prices, price_source = token_prices()
    stolen, saved, frozen = (total_usd(pick("BLOCK", "ALLOW")), total_usd(pick("BLOCK", "BLOCK")),
                             total_usd(pick("ALLOW", "BLOCK")))
    value_rows = [[esc(label), len(group), usd_text(total_usd(group)), esc(note)] for label, group, note in (
        ("Ataques bloqueados", pick("BLOCK", "BLOCK"), "valor protegido"),
        ("Ataques aprovados", pick("BLOCK", "ALLOW"), "valor que teria sido roubado"),
        ("Legítimas aprovadas", pick("ALLOW", "ALLOW"), "operação normal"),
        ("Legítimas bloqueadas", pick("ALLOW", "BLOCK"), "valor travado indevidamente (falso positivo)"),
    )]
    price_list = ", ".join(f"{symbol} US$ {br_number(price, 6 if price < 0.01 else 2)}" for symbol, price in prices.items())
    value_html = f"""
  <h3>Valor das transações</h3>
  <div class="kpis">
    <div class="kpi good"><b>{usd_text(saved)}</b><span>salvo: ataques bloqueados</span></div>
    <div class="kpi bad"><b>{usd_text(stolen)}</b><span>teria sido roubado: ataques aprovados</span></div>
    <div class="kpi bad"><b>{usd_text(frozen)}</b><span>legítimo bloqueado indevidamente</span></div>
  </div>
  {table(["Grupo", "Transações", "Valor (US$)", "Significado"], value_rows)}
  <p class="meta">Cotações: {esc(price_list)} ({esc(price_source)}). Valores sintéticos com tokens reais
  (USDC, USDT, wSOL, BONK, JUP). Nenhuma transação é assinada ou enviada: a simulação usa um RPC Solana falso local.</p>""" if has_value else ""
    return f"""
<section>
  <h2>Decisões: BLOCK x ALLOW</h2>
  <div class="kpis">
    <div class="kpi bad"><b>{len(got_block)}</b><span>BLOCK{f" · {usd_text(total_usd(got_block))}" if has_value else ""}</span></div>
    <div class="kpi good"><b>{len(got_allow)}</b><span>ALLOW{f" · {usd_text(total_usd(got_allow))}" if has_value else ""}</span></div>
    <div class="kpi"><b>{len(pick(expected="BLOCK"))} / {len(pick(expected="ALLOW"))}</b><span>esperado BLOCK / ALLOW</span></div>
    {f'<div class="kpi"><b>{len(others)}</b><span>outras decisões (REVIEW/ERROR)</span></div>' if others else ""}
  </div>
  {table(["", "Casos", "Obtido BLOCK", "Obtido ALLOW", "Outros"], matrix)}
  {value_html}
</section>"""


def build_html(rows, report):
    m = report["metrics"]["pipeline"]
    gen = report.get("generation", {})
    attacks = [row for row in rows if row["expected_decision"] == "BLOCK"]
    blocked = sum(row["response"].get("decision") == "BLOCK" for row in attacks)
    kinds = sorted({row["attack_type"] for row in rows},
                   key=lambda k: (k.startswith("benign"), k))
    summary = []
    for kind in kinds:
        group = [row for row in rows if row["attack_type"] == kind]
        ok = sum(row["response"].get("decision") == row["expected_decision"] for row in group)
        stopped = cause_counts(group)
        summary.append([f'<a href="#t-{esc(kind)}">{esc(type_label(kind))}</a>',
                        "Ataque" if group[0]["expected_decision"] == "BLOCK" else "Legítimo",
                        len(group), ok, len(group) - ok, pct(ok, len(group)) + bar(ok, len(group)),
                        "<br>".join(f"{esc(k)} <small>({v})</small>" for k, v in stopped.most_common(3)),
                        latency(group)[0]])
    funnel = report["funnel"]
    funnel_rows = [[esc(label), funnel[key], bar(funnel[key], funnel["entered"])] for key, label in (
        ("entered", "Entraram"), ("passed_blacklist", "Passaram pela blacklist"),
        ("passed_prompt_guard", "Passaram pelo Prompt Guard"), ("reached_simulation", "Chegaram à simulação"),
        ("simulation_allowed", "Aprovados no final"))]
    split_rows = metric_rows([("Pipeline", m), ("Só a IA (sem política)", report["metrics"]["raw_model"]),
                              *((f"Split {k}", v) for k, v in report["metrics"]["by_split"].items()),
                              *((f"Baseline {k}", v) for k, v in report["metrics"]["baselines"].items())])
    sections = "".join(f'<div id="t-{esc(k)}">{attack_type_section(k, [r for r in rows if r["attack_type"] == k])}</div>'
                       for k in kinds)
    errors = report.get("infrastructure_errors", [])
    generated = datetime.fromisoformat(report["generated_at"]).astimezone().strftime("%d/%m/%Y %H:%M")
    return f"""<!doctype html>
<html lang="pt-BR"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Relatório VETO Solana</title>
<style>
{PAGE_CSS}</style></head><body><main>
<header>
  <div><h1>Relatório VETO — Benchmark Solana</h1>
  <p class="meta">Gerado em {generated} · {report["cases"]} casos · seed {esc(gen.get("seed"))} · modo {esc(gen.get("ablation_mode"))}</p></div>
  <button class="noprint" onclick="window.print()">Imprimir / salvar PDF</button>
</header>
{decisions_section(rows)}
<section>
  <h2>Resumo geral</h2>
  <div class="kpis">
    <div class="kpi"><b>{pct(m["true_positive"] + m["true_negative"], m["cases"])}</b><span>acurácia do pipeline</span></div>
    <div class="kpi good"><b>{blocked}/{len(attacks)}</b><span>ataques bloqueados ({pct(blocked, len(attacks))})</span></div>
    <div class="kpi bad"><b>{m["false_negative"]}</b><span>ataques não detectados</span></div>
    <div class="kpi bad"><b>{m["false_positive"]}</b><span>falsos positivos finais (taxa {pct(m["false_positive"], m["false_positive"] + m["true_negative"])})</span></div>
    <div class="kpi"><b>{len(errors)}</b><span>erros de infraestrutura</span></div>
  </div>
  <h3>Por tipo de ataque</h3>
  <div class="wrap">{table(["Tipo", "Classe", "Casos", "Corretos", "Erros", "Taxa de acerto", "O que impediu / resultado", "Latência média"], summary)}</div>
  <h3>Funil de camadas</h3>
  {table(["Etapa", "Casos", ""], funnel_rows)}
  <h3>Métricas</h3>
  <div class="wrap">{table(["Conjunto", "Casos", "VP", "VN", "FP", "FN", "Acurácia", "Precisão", "Recall", "Taxa FP"], split_rows, "small")}</div>
  <p class="meta">Consistência metamórfica: {report["metrics"]["metamorphic"]["consistent"]}/{report["metrics"]["metamorphic"]["cases"]} casos.</p>
</section>
{false_positive_section(rows)}
{deterministic_section(rows)}
{sections}
</main></body></html>
"""


def write_html(rows, report, path=HTML_OUT):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(build_html(rows, report), encoding="utf-8")
    return path


def main():
    parser = argparse.ArgumentParser(description="Gera o relatório HTML do benchmark Solana")
    parser.add_argument("--input", type=Path, default=OUT)
    parser.add_argument("--output", type=Path, default=HTML_OUT)
    parser.add_argument("--open", action="store_true", help="abre o relatório no navegador")
    args = parser.parse_args()
    if not args.input.exists():
        parser.error(f"resultado não encontrado: {args.input}; execute 09_SIMULAR_ATAQUES_SOLANA.bat")
    rows = [json.loads(line) for line in args.input.read_text(encoding="utf-8").splitlines() if line.strip()]
    report_path = args.input.with_name("solana_attack_report.json")
    generation = {}
    if report_path.exists():
        generation = json.loads(report_path.read_text(encoding="utf-8")).get("generation", {})
    path = write_html(rows, detailed_report(rows, generation), args.output)
    print(f"Relatório: {path}")
    if args.open:
        webbrowser.open(path.as_uri())


if __name__ == "__main__":
    main()
