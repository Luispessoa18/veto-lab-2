"""Real upstream case discovery, optional read-only RPC observations, optional isolated Foundry replay.
No case is tagged as a verified on-chain attack merely from a filename or hash occurrence.
"""
import json, os, re, shutil, subprocess, time
from pathlib import Path
import requests
from .core import addr

REPO_URL = 'https://github.com/SunWeb3Sec/DeFiHackLabs.git'
CHAIN_ALIASES = {"ethereum": ["mainnet", "ethereum", "eth"], "bsc": ["bsc", "binance"],
                 "arbitrum": ["arbitrum"], "polygon": ["polygon", "matic"],
                 "optimism": ["optimism"], "base": ["base"], "avalanche": ["avalanche"],
                 "gnosis": ["gnosis"], "fantom": ["fantom"], "celo": ["celo"],
                 "linea": ["linea"], "mantle": ["mantle"], "moonriver": ["moonriver"]}
HEX64 = re.compile(r'(?<![a-fA-F0-9])0x[a-fA-F0-9]{64}(?![a-fA-F0-9])')
BLOCK = re.compile(r'(?i)(?:block(?:number)?\s*[:=]\s*|createSelectFork\s*\([^,]+,\s*)(\d{5,10})')
RPC = re.compile(r'(?i)createSelectFork\s*\(\s*(?:vm\.)?(?:envString\s*\(\s*)?["\'](mainnet|ethereum|bsc|arbitrum|polygon|optimism|base|avalanche|gnosis|fantom|celo|linea|mantle|moonriver)["\']')


def clone_repo(folder):
    folder = Path(folder)
    if (folder / '.git').exists(): return folder
    if not shutil.which('git'): raise RuntimeError('Git não encontrado. Instale Git for Windows.')
    folder.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(['git', 'clone', '--depth', '1', REPO_URL, str(folder)], check=True, timeout=240)
    return folder


def infer_chain(text, rel):
    match = RPC.search(text)
    if match: return 'ethereum' if match.group(1) == 'mainnet' else match.group(1).lower()
    snippet = '\n'.join(text.splitlines()[:65]).lower()
    for name, aliases in CHAIN_ALIASES.items():
        if any(re.search(r'\b'+re.escape(a)+r'\b', snippet) for a in aliases): return name
    return None


def discover(repo, max_cases=80):
    repo = Path(repo)
    files = sorted(set(repo.glob('src/test/**/*_exp.sol')) | set(repo.glob('src/test/**/*Exp.sol')))
    if not files: files = sorted(repo.glob('src/test/**/*.sol'))
    out = []
    for p in files:
        if len(out) >= max_cases: break
        src = p.read_text(encoding='utf-8', errors='replace')
        rel = p.relative_to(repo).as_posix()
        hashes = list(dict.fromkeys(HEX64.findall(src)))[:8]
        chain = infer_chain(src, rel)
        if not chain: continue  # only cases with a defensible chain hint
        head = '\n'.join(line.strip().lstrip('/').strip() for line in src.splitlines()[:45] if line.strip().startswith('//'))[:1800]
        out.append({'id': f'HIST-{len(out):04d}', 'kind':'historical_catalog', 'chain':chain,
                    'source_url':'https://github.com/SunWeb3Sec/DeFiHackLabs/blob/main/'+rel,
                    'source_file':rel, 'source_commit':None, 'tx_hash_candidates':hashes,
                    'block_hint': (BLOCK.search(src).group(1) if BLOCK.search(src) else None),
                    'notes_untrusted':head, 'ground_truth':{'label':'UNVERIFIED', 'rationale':'catalogued exploit test; no verified per-transaction label'},
                    'provenance':{'origin':'DeFiHackLabs source file', 'onchain_verified':False, 'anvil_replayed':False}})
    return out


def rpc_request(url, method, params):
    r=requests.post(url, json={'jsonrpc':'2.0','id':1,'method':method,'params':params},timeout=22)
    r.raise_for_status(); d=r.json()
    if d.get('error'): raise RuntimeError(str(d['error']))
    return d.get('result')


def fetch_receipts(cases, rpc_by_chain):
    """Only hash candidates; fetching a receipt doesn't prove the receipt is the exploited transaction."""
    for case in cases:
        url=rpc_by_chain.get(case['chain'])
        if not url: continue
        observed=[]
        for h in case['tx_hash_candidates'][:2]:
            try:
                tx=rpc_request(url, 'eth_getTransactionByHash', [h])
                rec=rpc_request(url, 'eth_getTransactionReceipt', [h])
                if not tx or not rec: continue
                observed.append({'hash':h, 'tx':{k:tx.get(k) for k in ('from','to','input','value','gas','blockNumber','transactionIndex')},
                                 'receipt':{k:rec.get(k) for k in ('status','gasUsed','blockNumber','contractAddress','logs')},
                                 'classification':'UNVERIFIED_HASH_CANDIDATE',
                                 'note':'RPC read-only observation; not proven to be attack transaction'})
            except Exception as e: observed.append({'hash':h,'error':str(e)[:220]})
        if observed: case['rpc_observations']=observed
    return cases


def foundry_replay(repo, case, timeout=160):
    """Run unmodified upstream test locally. Fork RPC endpoints are configured externally.
    NEVER conflate forge PASSED with verified financial effect/intent mismatch.
    """
    if not shutil.which('forge'): return {'status':'UNAVAILABLE', 'message':'forge absent'}
    try:
        p=subprocess.run(['forge','test','--match-path',case['source_file'],'-vvv'],cwd=str(repo),
                         capture_output=True,text=True,timeout=timeout)
        return {'status':'TEST_PASSED' if p.returncode==0 else 'TEST_FAILED',
                'return_code':p.returncode,'log_tail':(p.stdout+'\n'+p.stderr)[-9000:],
                'financial_effects_verified':False, 'anvil_replayed':False,
                'note':'Foundry runs a local EVM/fork per upstream test; this does not start or prove an Anvil replay'}
    except subprocess.TimeoutExpired: return {'status':'TIMEOUT','financial_effects_verified':False,'anvil_replayed':False}
    except Exception as e: return {'status':'ERROR','message':str(e)[:300],'financial_effects_verified':False,'anvil_replayed':False}
