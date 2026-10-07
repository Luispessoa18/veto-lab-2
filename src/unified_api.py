"""API publica unica para o pipeline VETO.

Fluxo de transacao: blacklist -> politica/IA -> simulacao Anvil.
Fluxo de agente: prompt guard -> IA. Uma decisao BLOCK encerra o fluxo.
"""
import argparse
import json
import os
import re
import threading
import time
import uuid
from collections import Counter, deque
from datetime import datetime, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlsplit

import requests

from .eval import llm_call, untrusted_texts

ROOT = Path(__file__).resolve().parents[1]
CONF = json.loads((ROOT / "config" / "settings.json").read_text(encoding="utf-8"))
if os.environ.get("SOLANA_RPC_URL"):
    CONF["solana_rpc_url"] = os.environ["SOLANA_RPC_URL"]
if os.environ.get("SOLANA_CLUSTER"):
    CONF["solana_cluster"] = os.environ["SOLANA_CLUSTER"].lower()
if os.environ.get("VETO_ABLATION_MODE"):
    CONF["ablation_mode"] = os.environ["VETO_ABLATION_MODE"].lower()
AUDIT_PATH = ROOT / "results" / "api_audit.jsonl"
SOLANA_REPORT_PATH = ROOT / "results" / "solana_attack_report.json"
AUDIT_LOCK = threading.Lock()
GUARD_SERVER = None
ADDRESS_FIELDS = {
    "from", "to", "recipient", "spender", "owner", "destination",
    "fee_recipient", "approved_router", "controller", "implementation",
    "effective_implementation", "approved_implementation", "program_id",
    "mint", "pubkey", "fee_payer", "authority", "delegate",
}


def normalize_address(value):
    value = str(value or "").strip()
    if value.lower().startswith("0x") and len(value) == 42:
        return value.lower()
    # Public keys Solana usam Base58 e sao case-sensitive.
    if 32 <= len(value) <= 44 and re.fullmatch(r"[1-9A-HJ-NP-Za-km-z]+", value):
        return value
    return None


def load_blacklist(path):
    source = Path(path)
    if not source.exists():
        return set()
    data = json.loads(source.read_text(encoding="utf-8"))
    values = []
    def collect(value):
        if isinstance(value, list):
            values.extend(value)
        elif isinstance(value, dict):
            for child in value.values():
                collect(child)
    collect(data)
    if not isinstance(data, (dict, list)):
        raise ValueError("blacklist deve ser uma lista ou objeto JSON")
    return {address for item in values if (address := normalize_address(item))}


def wallet_addresses(payload):
    found = set()

    def visit(value, key=""):
        if isinstance(value, dict):
            for child_key, child in value.items():
                visit(child, str(child_key).lower())
        elif isinstance(value, list):
            for child in value:
                visit(child, key)
        elif key in ADDRESS_FIELDS:
            address = normalize_address(value)
            if address:
                found.add(address)

    visit(payload)
    return found


def result(decision, layer, reason, **extra):
    value = {"decision": decision, "layer": layer, "reason": reason}
    value.update(extra)
    return value


class VetoPipeline:
    def __init__(self, config=None, blacklist_path=None, blacklist=None):
        self.config = dict(CONF if config is None else config)
        path = blacklist_path or self.config.get("blacklist_path", "config/blacklist.json")
        path = Path(path)
        if not path.is_absolute():
            path = ROOT / path
        self.blacklist_path = path
        self.blacklist = ({address for item in blacklist if (address := normalize_address(item))}
                          if blacklist is not None else None)

    def prompt_guard(self, payload, request_id, source):
        texts = untrusted_texts(payload)
        # Na rota do agente, o prompt em si e sempre entrada nao confiavel.
        if source == "agent" and isinstance(payload.get("prompt"), str):
            texts.insert(0, payload["prompt"][:12000])
        if not texts:
            return {"label": "BENIGN", "malicious_score": 0.0, "segments": [], "skipped_empty": True}
        if GUARD_SERVER is None:
            raise RuntimeError("prompt guard nao inicializado")
        scan = GUARD_SERVER.classify(texts)
        scan.update({"request_id": request_id, "source": source})
        return scan

    def layer1(self, evidence, request_id):
        return llm_call(
            self.config["llm_url"], evidence,
            self.config.get("max_output_tokens", 450),
            self.config.get("llm_timeout_seconds", 150),
            guard_url=None, request_id=request_id,
        )

    @staticmethod
    def solana_policy(payload):
        """Comparacao deterministica antes da IA para instrucoes Solana decodificadas."""
        scenario = payload.get("scenario", {})
        intent = scenario.get("intent", {})
        instructions = scenario.get("transaction", {}).get("instructions", [])
        conflicts = []
        dangerous = {"set_authority", "approve_delegate", "close_account", "upgrade_program"}
        allowed_programs = set(intent.get("allowed_programs", []))
        for index, instruction in enumerate(instructions):
            prefix = f"scenario.transaction.instructions[{index}]"
            program = instruction.get("program_id")
            if allowed_programs and program not in allowed_programs:
                conflicts.append({"field": f"{prefix}.program_id", "reason": "PROGRAM_NOT_AUTHORIZED"})
            if instruction.get("type") in dangerous and instruction.get("type") not in intent.get("allowed_actions", []):
                conflicts.append({"field": f"{prefix}.type", "reason": "DANGEROUS_ACTION_NOT_AUTHORIZED"})
            for field in ("recipient", "mint"):
                expected = intent.get(field)
                actual = instruction.get(field)
                if expected is not None and actual is not None and expected != actual:
                    conflicts.append({"field": f"{prefix}.{field}", "reason": f"{field.upper()}_MISMATCH"})
            try:
                if int(instruction.get("amount", 0)) > int(intent["max_amount"]):
                    conflicts.append({"field": f"{prefix}.amount", "reason": "AMOUNT_EXCEEDS_LIMIT"})
            except (KeyError, TypeError, ValueError):
                pass
        return conflicts

    def simulate_anvil(self, payload):
        spec = payload.get("anvil", {})
        rpc_url = spec.get("rpc_url") or self.config.get("anvil_rpc_url")
        if not rpc_url:
            return result("REVIEW", "anvil", "ANVIL_NOT_CONFIGURED")
        method = spec.get("method", "eth_call")
        if method not in ("eth_call", "eth_estimateGas"):
            return result("BLOCK", "anvil", "ANVIL_METHOD_NOT_ALLOWED")
        params = spec.get("params")
        if params is None:
            tx = payload.get("transaction") or payload.get("scenario", {}).get("transaction")
            if not isinstance(tx, dict):
                return result("REVIEW", "anvil", "ANVIL_TRANSACTION_MISSING")
            tx = {key: tx[key] for key in ("from", "to", "gas", "gasPrice", "value", "data") if key in tx}
            params = [tx, spec.get("block", "latest")]
        started = time.perf_counter()
        try:
            response = requests.post(rpc_url, json={"jsonrpc": "2.0", "id": request_id,
                                     "method": method, "params": params}, timeout=self.config.get("anvil_timeout_seconds", 30))
            response.raise_for_status()
            body = response.json()
        except Exception as exc:
            return result("REVIEW", "anvil", "ANVIL_UNAVAILABLE", error=str(exc)[:300])
        latency = round((time.perf_counter() - started) * 1000, 2)
        if body.get("error"):
            return result("BLOCK", "anvil", "ANVIL_SIMULATION_REVERTED",
                          rpc_error=body["error"], latency_ms=latency)
        return result("ALLOW", "anvil", "ANVIL_SIMULATION_SUCCEEDED",
                      rpc_result=body.get("result"), latency_ms=latency)

    def solana_rpc_targets(self, spec, cluster):
        """aval-svm (simulacao local) primeiro quando atende o cluster; RPC publico como reserva."""
        explicit = spec.get("rpc_url") or self.config.get("solana_rpc_url")
        if explicit:
            return [("rpc", explicit)]
        targets = []
        aval_url = self.config.get("aval_svm_url")
        if aval_url and str(self.config.get("aval_svm_cluster", "devnet")).lower() == cluster:
            targets.append(("aval-svm", aval_url))
        public = self.config.get("solana_rpc_by_cluster", {}).get(cluster)
        if public:
            targets.append(("rpc", public))
        return targets

    def simulate_solana(self, payload, request_id):
        spec = payload.get("solana", {})
        cluster = str(spec.get("cluster") or self.config.get("solana_cluster", "devnet")).lower()
        targets = self.solana_rpc_targets(spec, cluster)
        if not targets:
            return result("REVIEW", "solana_simulation", "SOLANA_RPC_NOT_CONFIGURED",
                          cluster=cluster)
        serialized = spec.get("serialized_transaction")
        if not isinstance(serialized, str) or not serialized:
            return result("REVIEW", "solana_simulation", "SOLANA_TRANSACTION_MISSING")
        options = {"encoding": spec.get("encoding", "base64"), "commitment": spec.get("commitment", "confirmed"),
                   "replaceRecentBlockhash": True, "sigVerify": False, "innerInstructions": True}
        body, engine, latency, last_error = None, None, None, None
        for engine, rpc_url in targets:
            started = time.perf_counter()
            timeout = (self.config.get("aval_svm_timeout_seconds", 12) if engine == "aval-svm"
                       else self.config.get("solana_timeout_seconds", 30))
            try:
                response = requests.post(rpc_url, json={"jsonrpc": "2.0", "id": request_id,
                                         "method": "simulateTransaction", "params": [serialized, options]},
                                         timeout=timeout)
                response.raise_for_status(); body = response.json()
                if not isinstance(body, dict):
                    raise ValueError("resposta JSON-RPC invalida")
            except Exception as exc:
                # qualquer falha do aval-svm (fora do ar, lento, HTTP 5xx, resposta invalida): tenta o RPC publico
                if engine == "aval-svm" or isinstance(exc, requests.ConnectionError):
                    last_error, body = exc, None
                    continue
                return result("REVIEW", "solana_simulation", "SOLANA_RPC_UNAVAILABLE",
                              cluster=cluster, engine=engine, error=str(exc)[:300])
            latency = round((time.perf_counter() - started) * 1000, 2)
            if engine == "aval-svm":
                # aval-svm nao suporta o programa, perdeu o upstream ou falhou por dentro: tenta o RPC publico
                if (body.get("error") or {}).get("code") in (-32603, -32004, -32005):
                    last_error, body = body["error"], None
                    continue
                # resposta "de sucesso" sem o bloco aval: outro servidor na porta (ex.: solana-test-validator)
                if not body.get("error") and not (isinstance(body.get("result"), dict) and "aval" in body["result"]):
                    last_error, body = "a porta do aval-svm respondeu sem o bloco aval (outro servidor?)", None
                    continue
            break
        if body is None:
            return result("REVIEW", "solana_simulation", "SOLANA_RPC_UNAVAILABLE",
                          cluster=cluster, engine=engine, error=str(last_error)[:300])
        if body.get("error"):
            return result("BLOCK", "solana_simulation", "SOLANA_RPC_ERROR", cluster=cluster, engine=engine,
                          rpc_error=body["error"], latency_ms=latency)
        res = body.get("result", {})
        value = res.get("value", {})
        if value.get("err") is not None:
            return result("BLOCK", "solana_simulation", "SOLANA_SIMULATION_FAILED", cluster=cluster, engine=engine,
                          simulation_error=value.get("err"), logs=value.get("logs", []), latency_ms=latency,
                          aval=res.get("aval"))
        return result("ALLOW", "solana_simulation", "SOLANA_SIMULATION_SUCCEEDED",
                      cluster=cluster, engine=engine, units_consumed=value.get("unitsConsumed"),
                      logs=value.get("logs", []), latency_ms=latency, aval=res.get("aval"))

    def verify_transaction(self, payload, request_id):
        trace = []
        mode = self.config.get("ablation_mode", "full")
        if mode not in ("full", "model_only", "no_model"):
            mode = "full"
        current_blacklist = self.blacklist if self.blacklist is not None else load_blacklist(self.blacklist_path)
        matches = sorted(wallet_addresses(payload) & current_blacklist)
        check = result("ALLOW" if mode == "model_only" else ("BLOCK" if matches else "ALLOW"),
                       "blacklist", "ABLATION_SKIPPED" if mode == "model_only" else
                       ("BLACKLISTED_WALLET" if matches else "NO_BLACKLIST_MATCH"), matches=matches)
        trace.append(check)
        if check["decision"] == "BLOCK":
            return self.finish(request_id, trace, check)

        guard = ({"label": "BENIGN", "skipped_ablation": True} if mode == "model_only"
                 else self.prompt_guard(payload, request_id, "transaction"))
        if guard["label"] == "MALICIOUS":
            check = result("BLOCK", "prompt_guard", "PROMPT_INJECTION_DETECTED",
                           security_alert=True, prompt_guard=guard)
            trace.append(check)
            return self.finish(request_id, trace, check)
        trace.append(result("ALLOW", "prompt_guard", "NO_PROMPT_INJECTION", prompt_guard=guard))

        evidence = payload.get("evidence")
        if evidence is None:
            scenario = payload.get("scenario")
            evidence = {"scenario": scenario} if isinstance(scenario, dict) else payload
        conflicts = (self.solana_policy(payload)
                     if mode != "model_only" and str(payload.get("chain", "")).lower() == "solana" else [])
        if conflicts:
            check = result("BLOCK", "layer_1", "SOLANA_POLICY_CONFLICT", conflicts=conflicts)
            trace.append(check)
            return self.finish(request_id, trace, check)
        check = (result("ALLOW", "layer_1", "ABLATION_SKIPPED", decision_source="ablation")
                 if mode == "no_model" else {"layer": "layer_1", **self.layer1(evidence, request_id)})
        if mode == "model_only" and check.get("model_decision") in ("ALLOW", "BLOCK", "REVIEW"):
            check["decision"] = check["model_decision"]
            check["policy_override"] = "ABLATION_OVERRIDES_DISABLED"
        trace.append(check)
        if check.get("decision") in ("BLOCK", "REVIEW", "ERROR", "INVALID"):
            return self.finish(request_id, trace, check)
        if mode == "model_only":
            return self.finish(request_id, trace, check)

        check = (self.simulate_solana(payload, request_id)
                 if str(payload.get("chain", "")).lower() == "solana" else self.simulate_anvil(payload))
        trace.append(check)
        return self.finish(request_id, trace, check)

    def run_agent(self, payload, request_id):
        guard = self.prompt_guard(payload, request_id, "agent")
        if guard["label"] == "MALICIOUS":
            check = result("BLOCK", "prompt_guard", "PROMPT_INJECTION_DETECTED",
                           security_alert=True, message="Prompt injection detectado; chamada da IA bloqueada.",
                           prompt_guard=guard)
            return self.finish(request_id, [check], check)
        evidence = payload.get("evidence") or {"scenario": payload.get("context", {}), "agent_prompt": payload.get("prompt", "")}
        check = {"layer": "agent", **self.layer1(evidence, request_id)}
        return self.finish(request_id, [check], check)

    @staticmethod
    def finish(request_id, trace, final):
        decision = final.get("decision", "ERROR")
        return {"request_id": request_id, "decision": decision,
                "blocked": decision == "BLOCK",
                "blocked_by": final.get("layer") if decision == "BLOCK" else None,
                "security_alert": bool(final.get("security_alert")),
                "layers": trace}


PIPELINE = None


def openapi_schema():
    layer = {
        "type": "object", "additionalProperties": True,
        "properties": {
            "decision": {"type": "string", "enum": ["ALLOW", "BLOCK", "REVIEW", "ERROR", "INVALID"]},
            "layer": {"type": "string"}, "reason": {"type": "string"},
        },
    }
    response = {
        "type": "object", "required": ["request_id", "decision", "blocked", "layers"],
        "properties": {
            "request_id": {"type": "string"},
            "decision": {"type": "string"}, "blocked": {"type": "boolean"},
            "blocked_by": {"type": ["string", "null"]}, "security_alert": {"type": "boolean"},
            "layers": {"type": "array", "items": layer}, "latency_ms": {"type": "number"},
        },
    }
    return {
        "openapi": "3.1.0",
        "info": {"title": "VETO Security API", "version": "1.0.0",
                 "description": "API unica: blacklist, camada 1/IA, simulacao Anvil e protecao contra prompt injection."},
        "servers": [{"url": "http://127.0.0.1:8070"}],
        "paths": {
            "/health": {"get": {"summary": "Verifica a saude da API", "responses": {"200": {"description": "OK"}}}},
            "/v1/transactions/verify": {"post": {
                "summary": "Verifica uma transacao nas tres camadas", "operationId": "verifyTransaction",
                "requestBody": {"required": True, "content": {"application/json": {"schema": {"$ref": "#/components/schemas/TransactionRequest"}}}},
                "responses": {"200": {"description": "Decisao e trilha das camadas", "content": {"application/json": {"schema": response}}},
                              "400": {"description": "Entrada invalida"}, "503": {"description": "Backend indisponivel"}},
            }},
            "/v1/agent/run": {"post": {
                "summary": "Protege o prompt e executa o agente", "operationId": "runAgent",
                "requestBody": {"required": True, "content": {"application/json": {"schema": {"$ref": "#/components/schemas/AgentRequest"}}}},
                "responses": {"200": {"description": "Decisao do agente ou bloqueio de injection", "content": {"application/json": {"schema": response}}},
                              "400": {"description": "Entrada invalida"}, "503": {"description": "Backend indisponivel"}},
            }},
        },
        "components": {"schemas": {
            "TransactionRequest": {
                "type": "object", "required": ["scenario"],
                "properties": {
                    "chain": {"type": "string", "enum": ["solana", "ethereum", "bsc", "arbitrum", "polygon", "optimism", "base", "avalanche", "gnosis"]},
                    "scenario": {"type": "object", "additionalProperties": True,
                                 "description": "Intent, transaction, pre_state e contract_metadata."},
                    "anvil": {"type": "object", "additionalProperties": True,
                              "example": {"method": "eth_call", "params": [{"from": "0x...", "to": "0x...", "data": "0x..."}, "latest"]}},
                    "solana": {"type": "object", "additionalProperties": True,
                               "example": {"cluster": "devnet", "serialized_transaction": "BASE64_TRANSACTION", "encoding": "base64", "commitment": "confirmed"}},
                },
            },
            "AgentRequest": {
                "type": "object", "required": ["prompt"],
                "properties": {"prompt": {"type": "string", "maxLength": 12000},
                               "context": {"type": "object", "additionalProperties": True}},
            },
        }},
    }


SWAGGER_HTML = """<!doctype html>
<html><head><meta charset="utf-8"><title>VETO Security API - Swagger</title>
<link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/swagger-ui-dist@5/swagger-ui.css"></head>
<body><div id="swagger-ui"></div>
<script src="https://cdn.jsdelivr.net/npm/swagger-ui-dist@5/swagger-ui-bundle.js"></script>
<script>SwaggerUIBundle({url:'/openapi.json',dom_id:'#swagger-ui',deepLinking:true,tryItOutEnabled:true});</script>
</body></html>"""

ADMIN_HTML = """<!doctype html><html lang="pt-BR"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>VETO Admin</title><style>
:root{color-scheme:dark}*{box-sizing:border-box}body{font:14px system-ui;margin:0;background:#0b1020;color:#e8edf7}.wrap{max-width:1500px;margin:auto;padding:24px}h1,h2{margin:0 0 8px}.muted{color:#8fa0bd}.tabs{display:flex;gap:8px;margin:22px 0}.tabs button.active{background:#6386ed}.view{display:none}.view.active{display:block}.cards{display:grid;grid-template-columns:repeat(auto-fit,minmax(160px,1fr));gap:12px;margin:18px 0}.card,.panel,table{background:#121a2e;border:1px solid #26324b;border-radius:10px}.card,.panel{padding:16px}.value{font-size:26px;font-weight:700;margin-top:5px}.grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(380px,1fr));gap:14px;margin:14px 0}table{width:100%;border-collapse:collapse;overflow:hidden}th,td{text-align:left;padding:9px;border-bottom:1px solid #26324b;vertical-align:top}th{color:#9eafd0}.ALLOW{color:#5ee09a}.BLOCK{color:#ff7185}.REVIEW,.ERROR,.INVALID{color:#ffc766}button{background:#3662e3;color:white;border:0;border-radius:7px;padding:8px 12px;cursor:pointer}.bar{height:9px;background:#26324b;border-radius:6px;overflow:hidden;margin-top:5px}.bar i{display:block;height:100%;background:#6386ed}details{border-bottom:1px solid #26324b;padding:9px 0}pre{white-space:pre-wrap;word-break:break-word;color:#c7d2e8}.empty{padding:30px;text-align:center;color:#8fa0bd}
</style></head><body><div class="wrap"><h1>VETO Admin</h1><div class="muted">Monitoramento local da API e benchmark Solana</div><div class="tabs"><button id="tab-requests" class="active" onclick="show('requests')">Requisições</button><button id="tab-report" onclick="show('report')">Dashboard do relatório</button></div>
<section id="view-requests" class="view active"><div class="cards"><div class="card">Requisições recentes<div class="value" id="total">0</div></div><div class="card">Latência média<div class="value" id="avg">0 ms</div></div><div class="card">Latência máxima<div class="value" id="max">0 ms</div></div><div class="card">Bloqueadas<div class="value BLOCK" id="blocked">0</div></div></div><p><button onclick="loadRequests()">Atualizar</button> <span id="request-status" class="muted"></span></p><table><thead><tr><th>Horário</th><th>ID</th><th>Rota</th><th>Decisão</th><th>Bloqueada por</th><th>Latência</th><th>Camadas executadas</th></tr></thead><tbody id="rows"></tbody></table></section>
<section id="view-report" class="view"><p><button onclick="loadReport()">Atualizar relatório</button> <span id="report-status" class="muted"></span></p><div id="report-content"><div class="empty">Execute 09_SIMULAR_ATAQUES_SOLANA.bat para gerar o relatório.</div></div></section></div><script>
const $=id=>document.getElementById(id),esc=s=>String(s??'').replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
function show(name){for(const n of ['requests','report']){$('view-'+n).classList.toggle('active',n===name);$('tab-'+n).classList.toggle('active',n===name)}if(name==='report')loadReport()}
async function loadRequests(){try{const d=await fetch('/admin/requests?limit=200',{cache:'no-store'}).then(r=>r.json());$('total').textContent=d.summary.total;$('avg').textContent=d.summary.average_latency_ms+' ms';$('max').textContent=d.summary.max_latency_ms+' ms';$('blocked').textContent=d.summary.blocked;$('rows').innerHTML=d.requests.map(x=>`<tr><td>${esc(x.timestamp)}</td><td>${esc(x.request_id)}</td><td>${esc(x.route)}</td><td class="${esc(x.decision)}">${esc(x.decision)}</td><td>${esc(x.blocked_by||'-')}</td><td>${esc(x.latency_ms)} ms</td><td>${esc((x.layers||[]).map(y=>y.layer+':'+y.decision).join(' → '))}</td></tr>`).join('');$('request-status').textContent='Atualizado '+new Date().toLocaleTimeString()}catch(e){$('request-status').textContent='Falha: '+e}}
function card(label,value,cls=''){return `<div class="card">${esc(label)}<div class="value ${cls}">${esc(value)}</div></div>`}function tableRows(obj,total){return Object.entries(obj||{}).map(([k,v])=>`<tr><td>${esc(k)}</td><td>${esc(v)}</td><td><div class="bar"><i style="width:${total?Math.min(100,v/total*100):0}%"></i></div></td></tr>`).join('')}
async function loadReport(){try{const r=await fetch('/admin/report',{cache:'no-store'}),d=await r.json();if(!r.ok)throw Error(d.error||r.status);const f=d.funnel||{},det=d.attack_detection||{},m=d.metrics||{},raw=m.raw_model||{};$('report-content').innerHTML=`<div class="cards">${card('Casos',d.cases)}${card('Pipeline',d.accuracy==null?'-':(d.accuracy*100).toFixed(1)+'%')}${card('IA bruta',raw.accuracy==null?'-':(raw.accuracy*100).toFixed(1)+'%')}${card('Ataques bloqueados',(det.blocked||0)+' / '+(det.attacks||0),'BLOCK')}${card('Chegaram à simulação',f.reached_simulation||0)}</div><div class="grid"><div class="panel"><h2>Funil por camada</h2><table>${tableRows(f,d.cases)}</table></div><div class="panel"><h2>Atribuição por componente</h2><table>${tableRows(m.component_attribution||d.blocked_by,d.cases)}</table></div><div class="panel"><h2>Development x holdout</h2>${details(Object.entries(m.by_split||{}).map(([split,value])=>({case_id:split,...value})))}</div><div class="panel"><h2>Overrides e baselines</h2><pre>${esc(JSON.stringify({overrides:m.policy_overrides,baselines:m.baselines},null,2))}</pre></div></div><div class="panel"><h2>Resultado por tipo de ataque</h2><table><thead><tr><th>Tipo</th><th>Total</th><th>Esperado</th><th>Obtido</th><th>Bloqueado por</th></tr></thead><tbody>${Object.entries(d.by_attack_type||{}).map(([k,v])=>`<tr><td>${esc(k)}</td><td>${v.total}</td><td>${esc(JSON.stringify(v.expected))}</td><td>${esc(JSON.stringify(v.actual))}</td><td>${esc(JSON.stringify(v.blocked_by))}</td></tr>`).join('')}</tbody></table></div><div class="grid"><div class="panel"><h2>Detecções da IA/política (${(d.ai_detections||[]).length})</h2>${details(d.ai_detections)}</div><div class="panel"><h2>Detecções do Prompt Guard (${(d.prompt_guard_detections||[]).length})</h2>${details(d.prompt_guard_detections)}</div></div><div class="panel"><h2>Ataques não bloqueados</h2>${(det.missed||[]).length?`<pre>${esc(det.missed.join('\\n'))}</pre>`:'<span class="ALLOW">Nenhum</span>'}</div>`;$('report-status').textContent='Seed '+esc(d.generation?.seed||'-')+' · modo '+esc(d.generation?.ablation_mode||'full')+' · '+esc(d.generated_at||'-')}catch(e){$('report-content').innerHTML='<div class="empty">Relatório ainda indisponível. Execute 09_SIMULAR_ATAQUES_SOLANA.bat.</div>';$('report-status').textContent='Falha: '+e}}
function details(items){return (items||[]).length?(items||[]).map(x=>`<details><summary>${esc(x.case_id)} · ${esc(x.attack_type)}</summary><pre>${esc(JSON.stringify(x,null,2))}</pre></details>`).join(''):'<div class="muted">Nenhum registro.</div>'}
loadRequests();setInterval(loadRequests,2000);
</script></body></html>"""


def audit(entry):
    AUDIT_PATH.parent.mkdir(parents=True, exist_ok=True)
    with AUDIT_LOCK, AUDIT_PATH.open("a", encoding="utf-8") as stream:
        stream.write(json.dumps(entry, ensure_ascii=False, separators=(",", ":")) + "\n")


def recent_audit(limit=200):
    limit = max(1, min(int(limit), 1000))
    if not AUDIT_PATH.exists():
        rows = []
    else:
        with AUDIT_LOCK, AUDIT_PATH.open(encoding="utf-8") as stream:
            rows = [json.loads(line) for line in deque(stream, maxlen=limit) if line.strip()]
    rows.reverse()
    latencies = [float(row.get("latency_ms", 0)) for row in rows]
    decisions = Counter(row.get("decision", "UNKNOWN") for row in rows)
    return {"summary": {"total": len(rows), "blocked": decisions.get("BLOCK", 0),
                        "average_latency_ms": round(sum(latencies) / len(latencies), 2) if latencies else 0,
                        "max_latency_ms": round(max(latencies), 2) if latencies else 0,
                        "decisions": dict(decisions)}, "requests": rows}


def solana_report():
    if not SOLANA_REPORT_PATH.exists():
        return None
    try:
        return json.loads(SOLANA_REPORT_PATH.read_text(encoding="utf-8"))
    except json.JSONDecodeError:
        return None


class Handler(BaseHTTPRequestHandler):
    def reply(self, status, body):
        data = json.dumps(body, ensure_ascii=False).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        parsed_url = urlsplit(self.path)
        route = parsed_url.path
        if route == "/admin":
            data = ADMIN_HTML.encode("utf-8")
            self.send_response(200); self.send_header("Content-Type", "text/html; charset=utf-8")
            self.send_header("Content-Length", str(len(data))); self.end_headers(); self.wfile.write(data)
            return
        if route == "/admin/requests":
            try:
                limit = int(parse_qs(parsed_url.query).get("limit", ["200"])[0])
            except ValueError:
                return self.reply(400, {"error": "limit deve ser inteiro"})
            return self.reply(200, recent_audit(limit))
        if route == "/admin/report":
            report = solana_report()
            if report is None:
                return self.reply(404, {"error": "relatório Solana ainda não foi gerado"})
            return self.reply(200, report)
        if route == "/docs":
            data = SWAGGER_HTML.encode("utf-8")
            self.send_response(200); self.send_header("Content-Type", "text/html; charset=utf-8")
            self.send_header("Content-Length", str(len(data))); self.end_headers(); self.wfile.write(data)
            return
        if route == "/openapi.json":
            return self.reply(200, openapi_schema())
        if route == "/":
            self.send_response(302); self.send_header("Location", "/docs"); self.end_headers(); return
        if route == "/health":
            return self.reply(200, {"status": "ok", "service": "veto-unified-api",
                                    "swagger": "/docs", "admin": "/admin", "openapi": "/openapi.json",
                                    "llm_url": PIPELINE.config.get("llm_url") if PIPELINE else None,
                                    "ablation_mode": PIPELINE.config.get("ablation_mode", "full") if PIPELINE else None,
                                    "routes": ["/v1/transactions/verify", "/v1/agent/run"]})
        return self.reply(404, {"error": "not found"})

    def do_POST(self):
        route = urlsplit(self.path).path
        if route not in ("/v1/transactions/verify", "/v1/agent/run"):
            return self.reply(404, {"error": "not found"})
        request_id = self.headers.get("X-Request-ID") or str(uuid.uuid4())
        try:
            size = int(self.headers.get("Content-Length", "0"))
            if size <= 0 or size > 2_000_000:
                raise ValueError("body deve ter entre 1 byte e 2 MB")
            payload = json.loads(self.rfile.read(size))
            if not isinstance(payload, dict):
                raise ValueError("body deve ser um objeto JSON")
            started = time.perf_counter()
            response = (PIPELINE.verify_transaction(payload, request_id)
                        if route.endswith("/verify") else PIPELINE.run_agent(payload, request_id))
            response["latency_ms"] = round((time.perf_counter() - started) * 1000, 2)
            audit({"timestamp": datetime.now(timezone.utc).isoformat(), "route": route, **response})
            self.reply(200, response)
        except (ValueError, json.JSONDecodeError) as exc:
            self.reply(400, {"request_id": request_id, "error": str(exc)[:500]})
        except Exception as exc:
            audit({"timestamp": datetime.now(timezone.utc).isoformat(), "route": route,
                   "request_id": request_id, "decision": "ERROR", "error": str(exc)[:500]})
            self.reply(503, {"request_id": request_id, "decision": "ERROR", "error": str(exc)[:500]})

    def log_message(self, format, *args):
        pass


def main():
    global PIPELINE, GUARD_SERVER
    from . import guard_server
    parser = argparse.ArgumentParser(description="API unica do pipeline VETO")
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=8070)
    parser.add_argument("--guard-model", default=str(ROOT / "models" / "Llama-Prompt-Guard-2-86M"))
    parser.add_argument("--guard-threshold", type=float, default=0.9)
    parser.add_argument("--blacklist", default=None)
    args = parser.parse_args()
    GUARD_SERVER = guard_server
    GUARD_SERVER.THRESHOLD = args.guard_threshold
    GUARD_SERVER.TOKENIZER = GUARD_SERVER.AutoTokenizer.from_pretrained(args.guard_model, local_files_only=True)
    GUARD_SERVER.MODEL = GUARD_SERVER.AutoModelForSequenceClassification.from_pretrained(args.guard_model, local_files_only=True).eval()
    PIPELINE = VetoPipeline(blacklist_path=args.blacklist)
    print(f"VETO API pronta em http://{args.host}:{args.port}", flush=True)
    ThreadingHTTPServer((args.host, args.port), Handler).serve_forever()


if __name__ == "__main__":
    main()
