import argparse, json, os
from pathlib import Path
import requests
from .core import DATASET_VERSION,fixtures,jsonl_read,jsonl_write
from .historical import clone_repo,discover,fetch_receipts,foundry_replay
from .eval import run_benchmark,report
ROOT=Path(__file__).resolve().parents[1]
CONF=json.loads((ROOT/'config/settings.json').read_text(encoding='utf-8'))
OUT=ROOT/'results'; OUT.mkdir(exist_ok=True)

def main():
    p=argparse.ArgumentParser(description='VETO LAB: avaliação cega antes/depois, casos sintéticos + catálogo histórico')
    sub=p.add_subparsers(dest='cmd',required=True)
    g=sub.add_parser('generate');g.add_argument('--count',type=int,default=CONF['synthetic_count']);g.add_argument('--catalog',action='store_true')
    c=sub.add_parser('catalog');c.add_argument('--fetch-rpc',action='store_true');c.add_argument('--forge',action='store_true');c.add_argument('--max-replays',type=int,default=3)
    r=sub.add_parser('run');r.add_argument('--limit',type=int,default=None)
    sub.add_parser('report')
    a=p.parse_args()
    if a.cmd=='generate':
        cs=fixtures(a.count,CONF['seed']);jsonl_write(OUT/'synthetic_cases.jsonl',cs)
        (OUT/'metrics.json').write_text(json.dumps({'status':'STALE','reason':'cases regenerated; rerun benchmark',
                                                    'dataset_version':DATASET_VERSION},indent=2),encoding='utf-8')
        print(f'Gerados {len(cs)} cenários sintéticos PAREADOS; NÃO são transações reais nem simulações Anvil.')
        if a.catalog:do_catalog(False,False,0)
    if a.cmd=='catalog':do_catalog(a.fetch_rpc,a.forge,a.max_replays)
    if a.cmd=='run':
        cases=list(jsonl_read(OUT/'synthetic_cases.jsonl'))
        url=CONF['llm_url'];health=url.split('/v1/')[0]+'/health'
        try: print('Servidor:',requests.get(health,timeout=10).json())
        except Exception as e:raise SystemExit(f'Servidor llama.cpp indisponível em {health}: {e}')
        guard_url=CONF.get('prompt_guard_url')
        if guard_url:
            guard_health=guard_url.rsplit('/',1)[0]+'/health'
            try:print('Prompt Guard:',requests.get(guard_health,timeout=10).json())
            except Exception as e:raise SystemExit(f'Prompt Guard indisponível em {guard_health}; execute 07_PROMPT_GUARD.bat: {e}')
        run_benchmark(cases,url,OUT/'predictions.jsonl',CONF['max_output_tokens'],
                      CONF['llm_timeout_seconds'],a.limit,CONF.get('benchmark_workers',2),guard_url)
        do_report()
    if a.cmd=='report':do_report()

def do_catalog(fetch,forge,n):
    repo=clone_repo(ROOT/'external/DeFiHackLabs')
    cs=discover(repo,CONF['max_historical'])
    if fetch:cs=fetch_receipts(cs,CONF['rpc_by_chain'])
    if forge:
        for c in cs[:n]:
            c['foundry_replay']=foundry_replay(repo,c)
            print(c['id'],c['source_file'],c['foundry_replay']['status'])
    jsonl_write(OUT/'historical_catalog.jsonl',cs)
    print(f'Catalogados {len(cs)} arquivos históricos com indicação de rede; SEM rótulo verificado por transação.')
    print('Veja results/historical_catalog.jsonl; preencha RPC em config/settings.json para buscar receipts (read-only).')

def do_report():
    f=OUT/'predictions.jsonl'
    if not f.exists():raise SystemExit('Execute 04_RODAR_BENCHMARK.bat antes.')
    rows=list(jsonl_read(f))
    versions={x.get('dataset_version') for x in rows}
    if versions!={DATASET_VERSION}:
        raise SystemExit(f'Predições obsoletas para {versions}; execute 04_RODAR_BENCHMARK.bat no dataset {DATASET_VERSION}.')
    result=report(rows)
    (OUT/'metrics.json').write_text(json.dumps(result,indent=2,ensure_ascii=False),encoding='utf-8')
    print(json.dumps(result,indent=2,ensure_ascii=False))

if __name__=='__main__':main()
