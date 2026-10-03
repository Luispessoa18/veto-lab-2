import argparse, json, os, threading, time, uuid
from datetime import datetime, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from types import SimpleNamespace

import torch
from transformers import AutoModelForSequenceClassification, AutoTokenizer

ROOT=Path(__file__).resolve().parents[1]
LOG_PATH=ROOT/'results'/'prompt_guard.jsonl'
DEFAULT_MODELS=[ROOT/'models'/'Llama-Prompt-Guard-2-86M',ROOT/'models'/'PIGuard']
GUARDS=[]      # detectores carregados: {'name','model','tokenizer','positive'}
MODEL=None     # primeiro detector (compatibilidade com código antigo)
TOKENIZER=None
MODEL_LOCK=threading.Lock()
LOG_LOCK=threading.Lock()
THRESHOLD=0.9
POSITIVE_LABELS=('injection','malicious','jailbreak','unsafe')


class PIGuardClassifier(torch.nn.Module):
    """Reimplementação local do PIGuard (DeBERTa-v3 + linear no token [CLS]), para não executar
    o modeling_piguard.py baixado do Hub (trust_remote_code)."""

    def __init__(self,config):
        super().__init__()
        from transformers import DebertaV2Model
        self.deberta=DebertaV2Model(config)
        self.classifier=torch.nn.Linear(config.hidden_size,config.num_labels)

    def forward(self,input_ids,attention_mask,**kwargs):
        hidden=self.deberta(input_ids=input_ids,attention_mask=attention_mask).last_hidden_state
        return SimpleNamespace(logits=self.classifier(hidden[:,0,:]))


def _load_piguard(path,config):
    from safetensors.torch import load_file
    from transformers import DebertaV2Config
    ignored=('architectures','auto_map','model_type','torch_dtype','transformers_version')
    module=PIGuardClassifier(DebertaV2Config(**{k:v for k,v in config.items() if k not in ignored}))
    state={k:v for k,v in load_file(str(path/'model.safetensors')).items() if not k.startswith('pooler.')}
    missing,unexpected=module.load_state_dict(state,strict=False)
    missing=[k for k in missing if not k.endswith('position_ids')]
    if missing or unexpected:
        raise RuntimeError(f'pesos do PIGuard incompatíveis: faltando={missing[:5]} sobrando={unexpected[:5]}')
    return module


def _load_one(path,device):
    path=Path(path)
    config=json.loads((path/'config.json').read_text(encoding='utf-8'))
    tokenizer=AutoTokenizer.from_pretrained(path,local_files_only=True)
    if config.get('model_type')=='piguard':
        model=_load_piguard(path,config)
    else:
        model=AutoModelForSequenceClassification.from_pretrained(path,local_files_only=True)
    labels={int(k):str(v).lower() for k,v in (config.get('id2label') or {}).items()}
    positive=next((index for index,label in labels.items() if label in POSITIVE_LABELS),1)
    return {'name':path.name,'model':model.eval().to(device),'tokenizer':tokenizer,'positive':positive}


def load_models(paths):
    """Carrega os detectores na GPU (CUDA) quando disponível; ignora os que não foram baixados."""
    global GUARDS,MODEL,TOKENIZER
    device='cuda' if torch.cuda.is_available() else 'cpu'
    found=[Path(p) if Path(p).is_absolute() else ROOT/p for p in paths]
    GUARDS=[_load_one(p,device) for p in found if (p/'config.json').exists()]
    if not GUARDS:
        raise RuntimeError(f'nenhum detector encontrado em {[str(p) for p in found]}')
    MODEL,TOKENIZER=GUARDS[0]['model'],GUARDS[0]['tokenizer']
    return device,[guard['name'] for guard in GUARDS]


def load_model(path):
    return load_models([path])[0]


def segments(text,max_tokens=500,guard=None):
    tokenizer=(guard or GUARDS[0])['tokenizer'] if GUARDS else TOKENIZER
    ids=tokenizer.encode(str(text),add_special_tokens=False)
    if not ids:return ['']
    return [tokenizer.decode(ids[i:i+max_tokens],skip_special_tokens=True) for i in range(0,len(ids),max_tokens)]


def score_chunks(chunks,batch_size=16,guard=None):
    guard=guard or GUARDS[0]
    model,tokenizer,positive=guard['model'],guard['tokenizer'],guard['positive']
    scores=[]
    with MODEL_LOCK,torch.inference_mode():
        for start in range(0,len(chunks),batch_size):
            inputs=tokenizer(chunks[start:start+batch_size],return_tensors='pt',truncation=True,max_length=512,padding=True)
            inputs={k:v.to(next(model.parameters()).device) for k,v in inputs.items()}
            probs=torch.softmax(model(**inputs).logits,dim=-1)
            scores+=[float(p[positive].item()) for p in probs]
    return scores


def text_scores(texts,guard=None):
    """Maior score de injection de cada texto, para um detector (um texto longo vira vários segmentos)."""
    guard=guard or GUARDS[0]
    owners,chunks=[],[]
    for index,text in enumerate(texts):
        for chunk in segments(text,guard=guard):
            owners.append(index);chunks.append(chunk)
    best=[0.0]*len(texts)
    for owner,value in zip(owners,score_chunks(chunks,guard=guard)):
        best[owner]=max(best[owner],value)
    return best


def text_scores_by_model(texts):
    """{nome do detector: [score de cada texto]} para todos os detectores carregados."""
    return {guard['name']:text_scores(texts,guard) for guard in GUARDS}


def classify(texts):
    """Rota /scan e compatibilidade: score de cada texto = maior entre os detectores."""
    by_model=text_scores_by_model(texts)
    results=[]
    for index,text in enumerate(texts):
        scores={name:round(values[index],6) for name,values in by_model.items()}
        top=max(scores.values(),default=0.0)
        results.append({'label':'MALICIOUS' if top>=THRESHOLD else 'BENIGN','malicious_score':top,
                        'scores':scores,'text_preview':str(text)[:240]})
    score=max((x['malicious_score'] for x in results),default=0.0)
    return {'label':'MALICIOUS' if score>=THRESHOLD else 'BENIGN',
            'malicious_score':score,'threshold':THRESHOLD,'segments':results}

def audit(entry):
    LOG_PATH.parent.mkdir(parents=True,exist_ok=True)
    with LOG_LOCK,LOG_PATH.open('a',encoding='utf-8') as f:
        f.write(json.dumps(entry,ensure_ascii=False,separators=(',',':'))+'\n')

class Handler(BaseHTTPRequestHandler):
    def reply(self,status,obj):
        data=json.dumps(obj,ensure_ascii=False).encode('utf-8')
        self.send_response(status);self.send_header('Content-Type','application/json; charset=utf-8')
        self.send_header('Content-Length',str(len(data)));self.end_headers();self.wfile.write(data)
    def do_GET(self):
        if self.path=='/health':return self.reply(200,{'status':'ok','model':'Llama-Prompt-Guard-2-86M'})
        self.reply(404,{'error':'not found'})
    def do_POST(self):
        if self.path!='/scan':return self.reply(404,{'error':'not found'})
        try:
            size=int(self.headers.get('Content-Length','0'))
            body=json.loads(self.rfile.read(size) or b'{}')
            texts=body.get('texts',[])
            if isinstance(texts,str):texts=[texts]
            if not isinstance(texts,list) or len(texts)>128:raise ValueError('texts must be a list with at most 128 items')
            started=time.perf_counter();result=classify([str(x) for x in texts])
            result.update({'request_id':body.get('request_id') or str(uuid.uuid4()),
                           'source':body.get('source','unknown'),
                           'latency_ms':round((time.perf_counter()-started)*1000,2)})
            audit({'timestamp':datetime.now(timezone.utc).isoformat(),**result})
            self.reply(200,result)
        except Exception as exc:self.reply(400,{'error':str(exc)[:500]})
    def log_message(self,format,*args):pass

def main():
    global MODEL,TOKENIZER,THRESHOLD
    p=argparse.ArgumentParser();p.add_argument('--model',action='append',help='pasta de um detector; repita para usar vários')
    p.add_argument('--host',default='127.0.0.1');p.add_argument('--port',type=int,default=8090)
    p.add_argument('--threshold',type=float,default=0.9);args=p.parse_args();THRESHOLD=args.threshold
    torch.set_num_threads(max(1,min(4,(os.cpu_count() or 4))))
    device,names=load_models(args.model or DEFAULT_MODELS)
    print(f'Prompt Guard pronto em http://{args.host}:{args.port} threshold={THRESHOLD} device={device} detectores={names}',flush=True)
    ThreadingHTTPServer((args.host,args.port),Handler).serve_forever()

if __name__=='__main__':
    main()
