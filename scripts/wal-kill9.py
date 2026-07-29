#!/usr/bin/env python3
"""Gera um WAL cortado por um SIGKILL de verdade.

Nao e simulacao: sobe um processo filho que escreve o log com fsync a cada
registro, e o mata com SIGKILL exatamente enquanto o ultimo registro esta pela
metade. O arquivo resultante e o que o kernel deixou.

Existe para produzir a fixture do teste `recupera_a_fila_de_um_arquivo_cortado_
por_kill9_real` em src-tauri/src/core/queue_wal.rs, e para poder refaze-la
quando o formato do registro mudar.

    python3 scripts/wal-kill9.py

Imprime o caminho do WAL gerado. Resultado esperado: N registros integros e
exatamente 1 truncado.
"""
import json, os, subprocess, sys, tempfile, signal, time
d = tempfile.mkdtemp(prefix="omniget-wal-")
wal = os.path.join(d, "queue.wal")
child = f'''
import json, os, sys, time
f = open({wal!r}, "ab", buffering=0)
for i in range(200):
    rec = {{"op":"enqueued","id":i,"url":f"https://example.com/{{i}}","title":f"v{{i}}",
           "platform":"youtube","output_dir":"/downloads","position":i}}
    f.write((json.dumps(rec)+"\\n").encode()); os.fsync(f.fileno())
f.write(json.dumps({{"op":"started","id":5}}).encode()+b"\\n"); os.fsync(f.fileno())
f.write(json.dumps({{"op":"completed","id":7,"file_path":"/x.mp4"}}).encode()+b"\\n"); os.fsync(f.fileno())
sys.stdout.write("READY\\n"); sys.stdout.flush()
f.write(b'{{"op":"progress","id":5,"perc'); os.fsync(f.fileno())
time.sleep(30)
'''
p = subprocess.Popen([sys.executable,"-c",child], stdout=subprocess.PIPE)
p.stdout.readline(); time.sleep(0.3); os.kill(p.pid, signal.SIGKILL); p.wait()
print(wal)
