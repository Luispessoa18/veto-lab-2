import argparse, json, os, threading, time, uuid
from datetime import datetime, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import torch
from transformers import AutoModelForSequenceClassification, AutoTokenizer

ROOT=Path(__file__).resolve().parents[1]
LOG_PATH=ROOT/'results'/'prompt_guard.jsonl'
MODEL=None
TOKENIZER=None
MODEL_LOCK=threading.Lock()
LOG_LOCK=threading.Lock()
THRESHOLD=0.9

def segments(text, max_tokens=500):
    ids=TOKENIZER.encode(str(text),add_special_tokens=False)
    if not ids:return ['']
    return [TOKENIZER.decode(ids[i:i+max_tokens],skip_special_tokens=True) for i in range(0,len(ids),max_tokens)]

def classify(texts):
    chunks=[chunk for text in texts for chunk in segments(text)]
    results=[]
    with MODEL_LOCK,torch.inference_mode():
        for chunk in chunks:
            inputs=TOKENIZER(chunk,return_tensors='pt',truncation=True,max_length=512)
            probs=torch.softmax(MODEL(**inputs).logits,dim=-1)[0]
            malicious=float(probs[1].item())
            results.append({'label':'MALICIOUS' if malicious>=THRESHOLD else 'BENIGN',
                            'malicious_score':round(malicious,6),'text_preview':chunk[:240]})
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
    p=argparse.ArgumentParser();p.add_argument('--model',default=str(ROOT/'models'/'Llama-Prompt-Guard-2-86M'))
    p.add_argument('--host',default='127.0.0.1');p.add_argument('--port',type=int,default=8090)
    p.add_argument('--threshold',type=float,default=0.9);args=p.parse_args();THRESHOLD=args.threshold
    torch.set_num_threads(max(1,min(4,(os.cpu_count() or 4))))
    TOKENIZER=AutoTokenizer.from_pretrained(args.model,local_files_only=True)
    MODEL=AutoModelForSequenceClassification.from_pretrained(args.model,local_files_only=True).eval()
    print(f'Prompt Guard pronto em http://{args.host}:{args.port} threshold={THRESHOLD}',flush=True)
    ThreadingHTTPServer((args.host,args.port),Handler).serve_forever()

if __name__=='__main__':
    main()
