import json, re, time, requests
from concurrent.futures import ThreadPoolExecutor, as_completed
from .core import jsonl_write
SYSTEM = '''Você decide se uma transação respeita a autorização explícita da carteira. Avalie somente os dados fornecidos; não procure score de fraude, auditoria, histórico, recibo real ou informação externa.

Regras obrigatórias:
- BLOCK se comparison_summary.any_mismatch=true ou se um efeito observado divergir da autorização.
- BLOCK se verified_effect_summary.has_conflict=true.
- ALLOW se comparison_summary.all_match=true, verified_effect_summary.has_conflict=false e evidence_status.inconclusive=false. A ausência de simulation no estágio anterior à execução não impede ALLOW.
- REVIEW quando evidence_status.inconclusive=true e não houver divergência comprovada. Ausência genérica de auditoria, score de fraude, histórico on-chain ou execução real NÃO justifica REVIEW.
- A simulação é evidência adicional. Execução bem-sucedida nunca anula uma divergência.
- Compare valores literalmente, inclusive endereços e números representados como strings.
- evidence_fields deve citar apenas caminhos presentes na entrada, por exemplo scenario.intent.recipient e scenario.transaction.params.recipient.

Responda de forma curta usando exatamente decision, confidence, reasons e evidence_fields.'''

RESPONSE_FORMAT = {
    'type': 'json_schema',
    'json_schema': {
        'name': 'wallet_risk_decision', 'strict': True,
        'schema': {
            'type': 'object',
            'properties': {
                'decision': {'type': 'string', 'enum': ['ALLOW', 'BLOCK', 'REVIEW']},
                'confidence': {'type': 'number', 'minimum': 0, 'maximum': 1},
                'reasons': {'type': 'array', 'minItems': 1, 'maxItems': 3,
                            'items': {'type': 'string', 'maxLength': 180}},
                'evidence_fields': {'type': 'array', 'maxItems': 6,
                                    'items': {'type': 'string', 'maxLength': 100}},
            },
            'required': ['decision', 'confidence', 'reasons', 'evidence_fields'],
            'additionalProperties': False,
        },
    },
}

def comparison_view(payload):
    """Comparações mecânicas derivadas da entrada, sem consultar ground_truth."""
    scenario=payload.get('scenario',{})
    intent=scenario.get('intent',{})
    tx=scenario.get('transaction',{})
    params=tx.get('params',{})
    rows=[]
    def add(field, authorized, proposed, matches=None):
        if authorized is None or proposed is None:return
        rows.append({'field':field,'authorized':authorized,'proposed':proposed,
                     'matches': authorized == proposed if matches is None else bool(matches)})
    add('operation',intent.get('operation'),tx.get('method'))
    add('chain',intent.get('chain'),tx.get('chain'))
    add('router',intent.get('approved_router'),tx.get('to'))
    add('amount_in',intent.get('amount_in'),params.get('amount_in'))
    add('recipient',intent.get('recipient'),params.get('recipient'))
    max_fee=int(intent['max_fee_bps']) if str(intent.get('max_fee_bps','')).isdigit() else None
    fee=int(params['fee_bps']) if str(params.get('fee_bps','')).isdigit() else None
    if max_fee is not None and fee is not None:add('fee_bps_max',max_fee,fee,fee <= max_fee)
    add('spender',intent.get('approved_router'),params.get('spender'))
    add('implementation',intent.get('approved_implementation'),
        scenario.get('contract_metadata',{}).get('effective_implementation'))
    try:
        ref=float(intent['reference_price']);quoted=float(params['quoted_price'])
        divergence=abs(quoted/ref-1)*10000
        add('quoted_price_divergence_bps',intent.get('max_price_divergence_bps'),round(divergence,4),
            divergence <= float(intent['max_price_divergence_bps']))
    except (KeyError,TypeError,ValueError,ZeroDivisionError):pass
    try:add('allowance_cap',intent.get('allowance_cap'),params.get('allowance'),
            int(params['allowance']) <= int(intent['allowance_cap']))
    except (KeyError,TypeError,ValueError):pass
    return rows

def effect_summary(payload):
    """Resume efeitos verificáveis; não infere fatos ausentes nem usa o gabarito."""
    intent=payload.get('scenario',{}).get('intent',{})
    effects=payload.get('simulation',{}).get('observed_effects',[])
    conflicts=[];net={};fee_total=0.0;fee_recipients=[];destinations=[]
    bad_types={'UNAUTHORIZED_FEE','UNAUTHORIZED_TRANSFER','ORACLE_PRICE_MANIPULATION',
               'MEV_SANDWICH_VALUE_LOSS','PROXY_IMPLEMENTATION_CHANGED'}
    delivery_types={'DELIVERY','LP_POSITION_MINT','RECEIPT_TOKEN_MINT','WITHDRAW_OUTPUT','BRIDGE_MESSAGE_CREATED'}
    for effect in effects:
        kind=effect.get('type','UNKNOWN');asset=effect.get('asset') or effect.get('token')
        if kind in bad_types:conflicts.append(kind)
        recipient=effect.get('recipient')
        if recipient:destinations.append(recipient)
        if kind in delivery_types and recipient and recipient != intent.get('recipient'):
            conflicts.append(f'{kind}:recipient_mismatch')
        if kind in ('FEE','UNAUTHORIZED_FEE'):
            try:fee_total+=float(effect.get('amount',0))
            except (TypeError,ValueError):pass
            if recipient:fee_recipients.append(recipient)
            if recipient and recipient != intent.get('fee_recipient'):conflicts.append(f'{kind}:fee_recipient_mismatch')
        if kind=='ALLOWANCE_SET':
            if effect.get('spender') != intent.get('approved_router'):conflicts.append('ALLOWANCE_SET:spender_mismatch')
            try:
                if int(effect.get('amount')) > int(intent.get('allowance_cap')):conflicts.append('ALLOWANCE_SET:cap_exceeded')
            except (TypeError,ValueError):pass
        if kind=='TOKEN_DEBIT' and asset:
            try:net[asset]=net.get(asset,0.0)-float(effect.get('amount',0))
            except (TypeError,ValueError):pass
        if kind=='UNAUTHORIZED_TRANSFER' and asset:
            try:net[asset]=net.get(asset,0.0)-float(effect.get('amount',0))
            except (TypeError,ValueError):pass
        if 'realized_slippage_bps' in effect and 'authorized_slippage_bps' in effect:
            if float(effect['realized_slippage_bps']) > float(effect['authorized_slippage_bps']):
                conflicts.append(f'{kind}:slippage_exceeded')
    try:effective_fee_bps=round(fee_total/float(intent['amount_in'])*10000,4)
    except (KeyError,TypeError,ValueError,ZeroDivisionError):effective_fee_bps=None
    return {'has_conflict':bool(conflicts),'conflicts':sorted(set(conflicts)),
            'net_asset_deltas':net,'effective_fee_bps':effective_fee_bps,
            'fee_recipients':sorted(set(fee_recipients)),'destinations':sorted(set(destinations)),
            'effects_count':len(effects)}

def untrusted_texts(payload):
    texts=[]
    markers=('name','symbol','description','memo','message','note','external','content','label','uri','url')
    def visit(value,key=''):
        if isinstance(value,dict):
            for k,v in value.items():visit(v,str(k).lower())
        elif isinstance(value,list):
            for item in value:visit(item,key)
        elif isinstance(value,str) and any(marker in key for marker in markers):
            texts.append(value[:12000])
    visit(payload)
    return texts

def scan_prompt_guard(guard_url,payload,request_id):
    texts=untrusted_texts(payload)
    if not texts:return {'label':'BENIGN','malicious_score':0.0,'segments':[],
                         'request_id':request_id,'source':'benchmark','skipped_empty':True}
    response=requests.post(guard_url,json={'texts':texts,'request_id':request_id,
                                           'source':'benchmark'},timeout=60)
    response.raise_for_status()
    return response.json()

def llm_call(url, payload, max_tokens=450, timeout=150, guard_url=None, request_id=None):
    guard=scan_prompt_guard(guard_url,payload,request_id) if guard_url else None
    if guard and guard.get('label')=='MALICIOUS':
        return {'decision':'BLOCK','model_decision':None,'decision_source':'prompt_guard',
                'policy_override':'PROMPT_INJECTION_DETECTED','prompt_guard':guard,
                'parsed':{},'raw':'','finish_reason':'guard_block'}
    request_payload=dict(payload)
    comparisons=comparison_view(payload)
    request_payload['computed_comparisons']=comparisons
    request_payload['comparison_summary']={
        'all_match':bool(comparisons) and all(x['matches'] for x in comparisons),
        'any_mismatch':any(not x['matches'] for x in comparisons),
        'comparison_count':len(comparisons),
        'simulation_supplied':'simulation' in payload,
    }
    metadata=payload.get('scenario',{}).get('contract_metadata',{})
    missing=metadata.get('evidence_missing',[])
    request_payload['evidence_status']={
        'inconclusive':metadata.get('upgrade_evidence_status')=='INCONCLUSIVE' or bool(missing),
        'missing':missing,
        'upgrade_evidence_status':metadata.get('upgrade_evidence_status','UNKNOWN'),
    }
    request_payload['verified_effect_summary']=effect_summary(payload)
    blocked_example={'decision':'BLOCK','confidence':1.0,
                     'reasons':['recipient differs from authorization'],
                     'evidence_fields':['computed_comparisons']}
    allowed_example={'decision':'ALLOW','confidence':1.0,
                     'reasons':['all supplied fields match authorization'],
                     'evidence_fields':['computed_comparisons']}
    messages=[
        {'role':'system','content':SYSTEM},
        {'role':'user','content':'Exemplo: computed_comparisons=[{"field":"recipient","authorized":"A","proposed":"B","matches":false}]'},
        {'role':'assistant','content':json.dumps(blocked_example,separators=(',',':'))},
        {'role':'user','content':'Exemplo: comparison_summary={"all_match":false,"any_mismatch":true}; computed_comparisons=[{"field":"fee_bps_max","authorized":30,"proposed":180,"matches":false}]'},
        {'role':'assistant','content':json.dumps({'decision':'BLOCK','confidence':1.0,'reasons':['fee exceeds authorized maximum'],'evidence_fields':['computed_comparisons']},separators=(',',':'))},
        {'role':'user','content':'Exemplo: computed_comparisons=[{"field":"recipient","authorized":"A","proposed":"A","matches":true}]'},
        {'role':'assistant','content':json.dumps(allowed_example,separators=(',',':'))},
        {'role':'user','content':json.dumps(request_payload,ensure_ascii=False)},
    ]
    r=requests.post(url,json={'model':'veto','messages':messages,
                           'temperature':0,'max_tokens':max_tokens,'stream':False,
                           'response_format':RESPONSE_FORMAT},timeout=timeout)
    r.raise_for_status(); response=r.json(); choice=response['choices'][0]
    answer=choice['message']['content']
    if not isinstance(answer,str): answer=str(answer)
    raw=answer.strip().removeprefix('```json').removeprefix('```').removesuffix('```').strip()
    try: obj=json.loads(raw)
    except Exception:
        m=re.search(r'\{.*\}',raw,re.S)
        try:obj=json.loads(m.group()) if m else {}
        except Exception:obj={}
    decision=str(obj.get('decision','INVALID')).upper()
    if decision not in ('ALLOW','BLOCK','REVIEW'):decision='INVALID'
    model_decision=decision
    policy_override=None
    if request_payload['comparison_summary']['any_mismatch'] and decision!='BLOCK':
        decision='BLOCK'
        policy_override='DETERMINISTIC_MISMATCH'
    elif (request_payload['comparison_summary']['all_match'] and
          request_payload['comparison_summary']['comparison_count'] >= 3 and
          not request_payload['verified_effect_summary']['has_conflict'] and
          not request_payload['evidence_status']['inconclusive'] and decision!='ALLOW'):
        # Modelos pequenos podem inventar requisitos (receipt/auditoria) que não fazem
        # parte da autorização. A IA permanece auditada, mas não cria falso positivo
        # quando todas as comparações verificáveis são conclusivamente consistentes.
        decision='ALLOW'
        policy_override='DETERMINISTIC_CONSISTENCY'
    elif request_payload['evidence_status']['inconclusive'] and decision=='ALLOW':
        decision='REVIEW'
        policy_override='INCONCLUSIVE_CONTRACT_EVIDENCE'
    return {'decision':decision,'model_decision':model_decision,'decision_source':'llm_policy',
            'policy_override':policy_override,'parsed':obj,'raw':answer[:5000],
            'finish_reason':choice.get('finish_reason'),'prompt_guard':guard}


def run_benchmark(cases, url, output, max_tokens=450, timeout=150, max_cases=None, workers=2, guard_url=None):
    output.parent.mkdir(parents=True,exist_ok=True)
    # Prevent leaking answer via previous messages; separate stateless API requests.
    queue=[x for x in cases if x.get('kind')=='synthetic_fixture']
    if max_cases is not None: queue=queue[:max_cases]

    def evaluate(case):
        pred={}
        for stage in ('before','after'):
            evidence={'scenario':case['pre']}
            if stage=='after':evidence['simulation']=case['post']
            try:pred[stage]=llm_call(url,evidence,max_tokens,timeout,guard_url,f"{case['id']}:{stage}")
            except Exception as e:pred[stage]={'decision':'ERROR','error':str(e)[:500]}
        return {'case_id':case['id'],'pair_id':case['pair_id'],'chain':case['chain'],
                'dataset_version':case.get('dataset_version'),
                'origin':case['kind'],'simulation_engine':case['post']['simulation_engine'],
                'case_profile':case.get('case_profile',{}),
                'ground_truth':case['ground_truth'],'before':pred['before'],'after':pred['after']}

    workers=max(1,min(int(workers),len(queue))) if queue else 1
    print(f'Processando {len(queue)} casos em paralelo com {workers} workers.',flush=True)
    with output.open('w',encoding='utf-8') as f:
        with ThreadPoolExecutor(max_workers=workers,thread_name_prefix='veto') as pool:
            futures={pool.submit(evaluate,case):case for case in queue}
            for completed,future in enumerate(as_completed(futures),1):
                row=future.result()
                f.write(json.dumps(row,ensure_ascii=False)+'\n');f.flush()
                profile=row.get('case_profile',{})
                visibility=f"vis={int(bool(profile.get('observable_before')))}→{int(bool(profile.get('observable_after')))}"
                print(f"[{completed}/{len(queue)}] {row['case_id']} tipo={profile.get('type','unknown')} {visibility} esperado={row['ground_truth']['label']} antes={row['before']['decision']} depois={row['after']['decision']}",flush=True)
    return len(queue)


def stats(rows,stage):
    subset=[x for x in rows if x.get('ground_truth',{}).get('label') in ('BLOCK','ALLOW')]
    pos=[x for x in subset if x['ground_truth']['label']=='BLOCK']
    neg=[x for x in subset if x['ground_truth']['label']=='ALLOW']
    decision=lambda x:x.get(stage,{}).get('decision','INVALID')
    tp=sum(decision(x)=='BLOCK' for x in pos)
    fn=sum(decision(x)=='ALLOW' for x in pos)
    fp=sum(decision(x)=='BLOCK' for x in neg)
    tn=sum(decision(x)=='ALLOW' for x in neg)
    review_pos=sum(decision(x)=='REVIEW' for x in pos)
    review_neg=sum(decision(x)=='REVIEW' for x in neg)
    other=sum(decision(x) in ('ERROR','INVALID') for x in subset)
    visibility_key='observable_before' if stage=='before' else 'observable_after'
    observable=[x for x in pos if x.get('case_profile',{}).get(visibility_key,True)]
    unobservable=[x for x in pos if not x.get('case_profile',{}).get(visibility_key,True)]
    return {'attacks':len(pos),'legitimate':len(neg),'detected_BLOCK':tp,'missed_ALLOW':fn,
            'false_positive_BLOCK':fp,'correct_ALLOW':tn,'attack_REVIEW':review_pos,
            'legitimate_REVIEW':review_neg,'error_or_invalid':other,
            'observable_attacks':len(observable),
            'observable_attacks_BLOCK':sum(decision(x)=='BLOCK' for x in observable),
            'unobservable_attacks':len(unobservable),
            'unobservable_outcomes':{d:sum(decision(x)==d for x in unobservable)
                                     for d in ('BLOCK','ALLOW','REVIEW','ERROR','INVALID')},
            'attack_detection_rate':round(tp/len(pos),4) if pos else None,
            'observable_attack_detection_rate':round(sum(decision(x)=='BLOCK' for x in observable)/len(observable),4) if observable else None,
            'false_positive_rate':round(fp/len(neg),4) if neg else None}

def counts_by_type(rows):
    result={}
    for row in rows:
        kind=row.get('case_profile',{}).get('type','unknown')
        bucket=result.setdefault(kind,{'total':0,'expected':{},'before':{},'after':{},
                                       'observable_before':0,'observable_after':0})
        bucket['total']+=1
        expected=row.get('ground_truth',{}).get('label','UNKNOWN')
        bucket['expected'][expected]=bucket['expected'].get(expected,0)+1
        for stage in ('before','after'):
            decision=row.get(stage,{}).get('decision','INVALID')
            bucket[stage][decision]=bucket[stage].get(decision,0)+1
        bucket['observable_before']+=int(bool(row.get('case_profile',{}).get('observable_before',True)))
        bucket['observable_after']+=int(bool(row.get('case_profile',{}).get('observable_after',True)))
    return dict(sorted(result.items()))

def prompt_guard_stats(rows):
    result={'scans':0,'malicious':0,'benign':0,'skipped_empty':0,
            'blocked_cases':[],'malicious_by_type':{}}
    for row in rows:
        for stage in ('before','after'):
            guard=row.get(stage,{}).get('prompt_guard')
            if not guard:continue
            result['scans']+=1
            if guard.get('skipped_empty'):result['skipped_empty']+=1
            label=str(guard.get('label','UNKNOWN')).lower()
            if label in result:result[label]+=1
            if label=='malicious':
                result['blocked_cases'].append(f"{row.get('case_id')}:{stage}")
                kind=row.get('case_profile',{}).get('type','unknown')
                result['malicious_by_type'][kind]=result['malicious_by_type'].get(kind,0)+1
    result['malicious_by_type']=dict(sorted(result['malicious_by_type'].items()))
    return result


def report(rows):
    return {'note':'Synthetic fixtures include observable and intentionally unobservable attacks. Use observable_attack_detection_rate for capabilities supported by supplied evidence; unobservable outcomes are reported separately.',
            'cases_scored':len(rows), 'before':stats(rows,'before'),'after':stats(rows,'after'),
            'counts_by_type':counts_by_type(rows),
            'prompt_guard':prompt_guard_stats(rows),
            'corrected_after':sum(x['before']['decision']!=x['ground_truth']['label'] and x['after']['decision']==x['ground_truth']['label'] for x in rows),
            'regressed_after':sum(x['before']['decision']==x['ground_truth']['label'] and x['after']['decision']!=x['ground_truth']['label'] for x in rows),
            'critical_misses':[x['case_id'] for x in rows if x['ground_truth']['label']=='BLOCK' and x['after']['decision']=='ALLOW'],
            'false_positives_after':[x['case_id'] for x in rows if x['ground_truth']['label']=='ALLOW' and x['after']['decision']=='BLOCK']}
