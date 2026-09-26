#!/usr/bin/env python3
"""Opt-in bounded real-media benchmark. Never imports personal cookies/config.
Usage: python3 scripts/mcp/benchmark.py --cases cases.json --output report-dir
Engine results are explicitly NOT MCP interoperability evidence.
"""
import argparse,csv,datetime,hashlib,json,pathlib,re,subprocess,time

def redact(s):
    s=re.sub(r'(?im)(authorization|cookie|set-cookie):[^\r\n]*',r'\1: [REDACTED]',s)
    s=re.sub(r'https?://[^\s]+',lambda m:m[0].split('?')[0],s)
    s=re.sub(r'(/Users/|/home/)[^/\s]+',r'~',s)
    return s[-8000:]
def run(args,timeout):
    try:
        p=subprocess.run(args,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,timeout=timeout)
        return p.returncode,p.stdout,p.stderr
    except subprocess.TimeoutExpired as e:
        return 124,'',str(e.stderr.decode(errors='replace') if isinstance(e.stderr,bytes) else e.stderr or '')+'\nBENCHMARK_TIMEOUT'
def main():
    parser=argparse.ArgumentParser();parser.add_argument('--cases',required=True);parser.add_argument('--output',required=True);parser.add_argument('--timeout',type=int,default=90);a=parser.parse_args()
    dest=pathlib.Path(a.output);dest.mkdir(parents=True,exist_ok=True)
    cases=json.loads(pathlib.Path(a.cases).read_text());version=run(['yt-dlp','--version'],10)[1].strip();results=[];start=time.monotonic()
    # Round-robin smoke before repeating platforms.
    cases=sorted(cases,key=lambda c:(int(c['caseId'].rsplit('-',1)[1]),c['platform']))
    for c in cases:
        if time.monotonic()-start>2700:break
        folder=dest/'media'/c['caseId'];folder.mkdir(parents=True,exist_ok=True)
        before=time.monotonic();utc=datetime.datetime.now(datetime.timezone.utc).isoformat()
        code,stdout,stderr=run(['yt-dlp','--ignore-config','--no-playlist','--playlist-end','1','--no-progress','--no-warnings','--socket-timeout','10','--retries','0','--fragment-retries','0','--max-filesize','150M','--max-downloads','1','-f','bv*[height<=720]+ba/b[height<=720]','--restrict-filenames','-o',str(folder/'%(id)s.%(ext)s'),'--print','after_move:filepath',c['url']],a.timeout)
        artifacts=[]
        for file in folder.iterdir():
            if file.suffix in ('.part','.ytdl'):continue
            status,out,err=run(['ffprobe','-v','error','-show_streams','-show_format','-of','json',str(file)],15)
            try:probe=json.loads(out)
            except ValueError:probe={}
            valid=status==0 and file.stat().st_size>0 and bool(probe.get('streams'))
            decode=run(['ffmpeg','-v','error','-i',str(file),'-t','2','-f','null','-'],15)[0] if valid else None
            artifacts.append({'name':file.name,'bytes':file.stat().st_size,'valid':valid and decode==0,'probe':probe,'sampleDecodeExitCode':decode})
        success=any(x['valid'] for x in artifacts)
        row={**c,'path':'engine_reference','engine':'yt-dlp','engineVersion':version,'authClass':'anonymous_no_imported_config','startedAt':utc,'totalSeconds':time.monotonic()-before,'exitCode':code,'outcome':'success' if success else 'failed','artifactValidation':artifacts,'evidence':redact(stderr),'options':'single item; <=720p preference; 150MiB per file; no credentials; max 90s','independence':'OmniGet may use the same engine; this is not an independent downloader comparison'}
        results.append(row)
        with (dest/'results.jsonl').open('a') as f:f.write(json.dumps(row)+'\n')
        print(c['caseId'],row['outcome'],round(row['totalSeconds'],2),flush=True)
        if sum(x['bytes'] for r in results for x in r['artifactValidation'])>2*1024**3:break
    with (dest/'summary.csv').open('w') as f:
        w=csv.DictWriter(f,fieldnames=['caseId','platform','path','outcome','totalSeconds','exitCode']);w.writeheader();w.writerows({k:r[k] for k in w.fieldnames} for r in results)
    succeeded=sum(r['outcome']=='success' for r in results)
    (dest/'REPORT.md').write_text(f'# Real engine benchmark\n\n{succeeded}/{len(results)} attempted cases produced validated media.\n\nThis run tests the reference engine only, not MCP, the UI or remote clients. No p95 reported for this small sample. Failures remain in the denominator. No personal credentials were imported. See results.jsonl for evidence and artifact validation.\n')
if __name__=='__main__':main()
