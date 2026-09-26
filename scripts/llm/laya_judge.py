#!/usr/bin/env python3
"""Juiz local Laya (System One aberto, wire do Jev) para o harness de missões.

Python 3.9, só stdlib. Fala com um `laya-server` local (github.com/nvkudva/laya-server),
que serve `POST /v1/systemone` no mesmo contrato do Jev (TypeSafe): um estado + perguntas
tipadas; aqui só `noul` (P(verdadeiro)) e `choice`.

Subcomandos:
    build     monta o conjunto de avaliação (saídas t08/t11 das rodadas + negativos sintéticos)
    eval      roda Laya (inglês e multilíngue) e Haiku lado a lado no conjunto
    calibrate faixa de incerteza do cascade (Laya decide fora dela, Haiku dentro)
    finetune-set  JSONL de ajuste fino (afirmações + nota do Haiku como alvo suave)
    classify  dificuldade (simples/difícil) das tarefas do harness só pelo prompt

Funções usadas por missions_eval.py: laya_verdict, haiku_verdict, cascade_verdict, classify_prompt.
"""
import argparse
import glob
import json
import os
import pathlib
import re
import shutil
import statistics
import subprocess
import sys
import time
import urllib.request

LAYA_URL = os.environ.get('LAYA_URL', 'http://127.0.0.1:8766')
LAYA_MULTI_URL = os.environ.get('LAYA_MULTI_URL', 'http://127.0.0.1:8767')
HAIKU_PASS = 7  # nota >= 7 conta como aprovado
# Faixa de incerteza do cascade, calibrada por `calibrate` no conjunto da onda 2
# (sobrescrita por --cascade-band lo,hi). Fora dela o Laya decide; dentro, Haiku.
CASCADE_BAND = (0.0, 1.0)
ONDAS = pathlib.Path(__file__).resolve().parents[2] / 'estudos/otimizacao-benchmark-2026-09-26/ondas'

SCRUB = ('ANTHROPIC_API_KEY', 'ANTHROPIC_AUTH_TOKEN', 'ANTHROPIC_BASE_URL', 'CLAUDE_CODE_OAUTH_TOKEN')


# ── Laya ───────────────────────────────────────────────────────────────

def systemone(state, questions, url=None, timeout=30):
    body = json.dumps({'model': 'laya', 'state': state, 'questions': questions}).encode()
    req = urllib.request.Request((url or LAYA_URL).rstrip('/') + '/v1/systemone', data=body,
                                 headers={'content-type': 'application/json'})
    t0 = time.time()
    with urllib.request.urlopen(req, timeout=timeout) as r:
        res = json.loads(r.read())
    return res.get('answers') or {}, (time.time() - t0) * 1000, (res.get('usage') or {}).get('input_tokens')


# Afirmações por tarefa: o Laya dá P(verdadeiro) para cada uma; o veredito é o mínimo
# (todas precisam valer). Genérico = critério do juiz como afirmação.
STATEMENTS = {
    't08': [
        'The English text is a faithful and complete translation of the Portuguese source: it says the same '
        'things, with nothing inverted, added or left out.',
        'The English text keeps every fact of the source: open source, videos and music from more than a '
        'thousand sites, works on Windows, macOS and Linux, does not collect user data, and on a failed '
        'download shows the reason and suggests what to do next.',
    ],
    't11': [
        'The summary has exactly three English bullet points and each one correctly states a user-facing '
        'change from the changelog, with nothing contradicted or invented.',
        'Every bullet of the summary is true according to the changelog.',
    ],
}


def state_for(task_prompt, files):
    return {'task': task_prompt, 'files': {k: v[:1500] for k, v in files.items()}}


def laya_verdict(task_id, task_prompt, criterion, files, url=None, agg='min'):
    """Devolve {'p', 'aprovado', 'por_pergunta', 'latencia_ms', 'input_tokens'}."""
    stmts = STATEMENTS.get(task_id[:3]) or [criterion]
    qs = {f's{i}': {'type': 'noul', 'instructions': s} for i, s in enumerate(stmts)}
    ans, ms, tok = systemone(state_for(task_prompt, files), qs, url)
    ps = [float(ans[k]['noul']) for k in qs]
    p = min(ps) if agg == 'min' else statistics.mean(ps)
    return {'p': round(p, 4), 'aprovado': p >= 0.5, 'por_pergunta': [round(x, 4) for x in ps],
            'latencia_ms': round(ms, 1), 'input_tokens': tok}


# ── Haiku (o mesmo juiz que missions_eval usava) ───────────────────────

def _env():
    return {k: v for k, v in os.environ.items()
            if k not in SCRUB and not k.startswith('CLAUDECODE') and not k.startswith('CLAUDE_CODE_')}


def haiku_verdict(task_prompt, criterion, files, model='haiku', cwd=None):
    prompt = (f'Você é um avaliador. Tarefa dada ao agente: {task_prompt}\nCritério: {criterion}\n'
              f'Arquivos do workspace após a tarefa: {json.dumps(files, ensure_ascii=False)}\n'
              'Responda só com um JSON {"nota": 0-10, "motivo": "..."}')
    t0 = time.time()
    p = subprocess.run([shutil.which('claude') or 'claude', '-p', '--output-format', 'json', '--model', model,
                        '--tools', '', '--no-session-persistence'], input=prompt, capture_output=True, text=True,
                       env=_env(), timeout=180, cwd=cwd or '/tmp')
    ms = (time.time() - t0) * 1000
    try:
        res = json.loads(p.stdout)
        m = re.search(r'\{.*\}', res.get('result', ''), re.S)
        v = json.loads(m.group(0)) if m else {}
        nota = v.get('nota')
        return {'nota': nota, 'aprovado': (nota is not None and float(nota) >= HAIKU_PASS),
                'motivo': str(v.get('motivo', ''))[:200], 'custo_usd': res.get('total_cost_usd'),
                'latencia_ms': round(ms, 1)}
    except Exception as e:  # noqa: BLE001
        return {'erro': str(e)[:200], 'latencia_ms': round(ms, 1)}


def cascade_verdict(task_id, task_prompt, criterion, files, band=None, url=None, model='haiku', cwd=None):
    lo, hi = band or CASCADE_BAND
    lv = laya_verdict(task_id, task_prompt, criterion, files, url)
    if lv['p'] < lo or lv['p'] > hi:
        return {'decidiu': 'laya', 'aprovado': lv['p'] > hi, 'laya': lv, 'custo_usd': 0.0,
                'nota': 10 if lv['p'] > hi else 0}
    hv = haiku_verdict(task_prompt, criterion, files, model, cwd)
    return {'decidiu': 'haiku', 'aprovado': hv.get('aprovado'), 'laya': lv, 'haiku': hv,
            'custo_usd': hv.get('custo_usd'), 'nota': hv.get('nota')}


# ── dificuldade ────────────────────────────────────────────────────────

DIFFICULTY_Q = {
    'dificuldade': {
        'type': 'choice',
        'instructions': 'How hard is this task for a coding agent?',
        'criteria': {
            'simple': 'A single direct step: write, convert, look up, count or translate something; '
                      'no debugging and no reasoning about code.',
            'hard': 'Needs reasoning about code: find and fix bugs, implement a function to pass tests, '
                    'or refactor across several files.',
        },
    },
}


def classify_prompt(prompt, url=None):
    ans, ms, _ = systemone({'task': prompt}, DIFFICULTY_Q, url)
    a = ans['dificuldade']
    return {'classe': a['choice'], 'p_hard': round(float(a['probabilities']['hard']), 4), 'latencia_ms': round(ms, 1)}


# ── conjunto de avaliação ──────────────────────────────────────────────

T08_PROMPT = ('Traduza o texto de pt.txt para inglês e grave a tradução em en.txt. Mantenha o nome OmniGet e os '
              'nomes dos sistemas operacionais. Só a tradução no arquivo.')
T08_CRIT = 'A tradução em inglês é fiel, fluente e completa (três frases: o que é, plataformas/privacidade, falhas)?'
T11_PROMPT = ('Leia changelog.txt e escreva summary.md em inglês com exatamente 3 tópicos (linhas começando com "- ") '
              'resumindo as mudanças mais importantes para o usuário.')
T11_CRIT = 'Os 3 tópicos em inglês resumem corretamente as mudanças mais importantes do changelog?'
PT_TEXT = ('O OmniGet é um aplicativo de código aberto que baixa vídeos e músicas de mais de mil sites. '
           'Ele funciona no Windows, no macOS e no Linux, e não coleta dados de quem usa. '
           'Quando um download falha, o aplicativo mostra o motivo e sugere o que fazer em seguida.\n')
CHANGELOG = ('0.10.1\n- Correção: downloads do TikTok voltaram a funcionar depois da mudança da API.\n'
             '- Novo: fila respeita limite de 2 downloads por site.\n- Interno: refatoração do módulo de erros.\n'
             '- Novo: legendas automáticas podem ser salvas em SRT.\n- Correção: travamento ao pausar downloads HLS.\n'
             '- Interno: atualização de dependências.\n')

EN_OK = ('OmniGet is an open-source application that downloads videos and music from more than a thousand sites. '
         'It works on Windows, macOS, and Linux, and does not collect data from its users. When a download fails, '
         'the application shows the reason and suggests what to do next.')
T08_NEG = {
    'privacidade-invertida': EN_OK.replace('does not collect data from its users', 'collects data from its users'),
    'frase-omitida': EN_OK.split(' When a download')[0],
    'so-windows': EN_OK.replace('It works on Windows, macOS, and Linux', 'It works only on Windows'),
    'codigo-fechado': EN_OK.replace('open-source', 'closed-source'),
    'falha-invertida': EN_OK.replace('shows the reason and suggests what to do next',
                                     'hides the reason and gives no suggestion'),
    'cem-sites': EN_OK.replace('more than a thousand sites', 'about a hundred sites'),
    'nao-traduzido': PT_TEXT.strip(),
    'so-primeira-frase': EN_OK.split('. ')[0] + '.',
    'espanhol': ('OmniGet es una aplicación de código abierto que descarga vídeos y música de más de mil sitios. '
                 'Funciona en Windows, macOS y Linux, y no recopila datos de quienes la usan. Cuando una descarga '
                 'falla, la aplicación muestra el motivo y sugiere qué hacer a continuación.'),
    'coleta-uso': EN_OK.replace('does not collect data from its users',
                                'collects usage data to improve downloads'),
    'frase-inventada': EN_OK + ' It also requires a paid subscription to download music.',
    'so-audio': EN_OK.replace('videos and music', 'music only'),
}
T11_NEG = {
    'internos': '- Refactored the error handling module.\n- Updated dependencies.\n- Fixed TikTok downloads after the API change.\n',
    'tiktok-invertido': ('- TikTok downloads stopped working after the API change.\n- Pausing HLS downloads no longer freezes the app.\n'
                         '- Automatic subtitles can now be saved as SRT.\n'),
    'dois-topicos': '- Fixed TikTok downloads after the API change.\n- Automatic subtitles can now be saved as SRT.\n',
    'fila-ilimitada': ('- Fixed TikTok downloads after the API change.\n- The queue now allows unlimited downloads per site.\n'
                       '- Automatic subtitles can now be saved as SRT.\n'),
    'portugues': ('- Downloads do TikTok voltaram a funcionar.\n- Legendas automáticas podem ser salvas em SRT.\n'
                  '- Corrigido travamento ao pausar downloads HLS.\n'),
    'vtt': ('- Fixed TikTok downloads after the API change.\n- Automatic subtitles can now be saved as VTT.\n'
            '- Fixed a freeze when pausing HLS downloads.\n'),
    'so-internos': '- Refactored the error module.\n- Updated dependencies.\n- Internal cleanup of the codebase.\n',
    'hls-invertido': ('- Fixed TikTok downloads after the API change.\n- Pausing HLS downloads now freezes the app.\n'
                      '- Automatic subtitles can now be saved as SRT.\n'),
    'cinco-topicos': ('- Fixed TikTok downloads.\n- Queue limit of 2 per site.\n- Refactored errors module.\n'
                      '- Subtitles saved as SRT.\n- Fixed HLS pause freeze.\n'),
    'inventado': ('- Added YouTube 8K downloads.\n- Added a dark theme.\n- Fixed TikTok downloads after the API change.\n'),
    'vago': '- Various fixes.\n- Some improvements.\n- Other updates.\n',
    'deps-no-lugar': ('- Dependencies were updated for security.\n- Automatic subtitles can now be saved as SRT.\n'
                      '- Fixed a freeze when pausing HLS downloads.\n'),
}


def build_set():
    items, seen = [], set()
    files = [ONDAS / 'onda1/missions-baseline.json'] + sorted(pathlib.Path(p) for p in glob.glob(str(ONDAS / 'onda2/missions-r*.json')))
    for f in files:
        d = json.loads(f.read_text())
        for r in d['tarefas']:
            tid = r['task'][:3]
            if tid not in ('t08', 't11'):
                continue
            out = 'en.txt' if tid == 't08' else 'summary.md'
            p = pathlib.Path(r['workspace']) / out
            if not p.exists():
                continue
            text = p.read_text(errors='replace')
            src = {'pt.txt': PT_TEXT} if tid == 't08' else {'changelog.txt': CHANGELOG}
            items.append({'id': f'{tid}-{f.stem}-r{r["rep"]}', 'task': tid, 'rotulo': 1, 'origem': f.name,
                          'duplicata': text.strip() in seen, 'files': dict(src, **{out: text})})
            seen.add(text.strip())
    for name, text in T08_NEG.items():
        items.append({'id': f't08-neg-{name}', 'task': 't08', 'rotulo': 0, 'origem': 'sintetico',
                      'files': {'pt.txt': PT_TEXT, 'en.txt': text + '\n'}})
    for name, text in T11_NEG.items():
        items.append({'id': f't11-neg-{name}', 'task': 't11', 'rotulo': 0, 'origem': 'sintetico',
                      'files': {'changelog.txt': CHANGELOG, 'summary.md': text}})
    return items


def meta(tid):
    return (T08_PROMPT, T08_CRIT) if tid == 't08' else (T11_PROMPT, T11_CRIT)


def cmd_eval(args):
    items = [json.loads(ln) for ln in open(args.set)]
    rows = []
    for it in items:
        tp, crit = meta(it['task'])
        row = {k: it[k] for k in ('id', 'task', 'rotulo', 'origem')}
        row['laya'] = laya_verdict(it['task'], tp, crit, it['files'], LAYA_URL)
        row['laya_multi'] = laya_verdict(it['task'], tp, crit, it['files'], LAYA_MULTI_URL)
        if not args.no_haiku:
            row['haiku'] = haiku_verdict(tp, crit, it['files'], args.model)
        rows.append(row)
        print(it['id'], it['rotulo'], row['laya']['p'], row['laya_multi']['p'], (row.get('haiku') or {}).get('nota'),
              flush=True)
    pathlib.Path(args.out).write_text(json.dumps(rows, ensure_ascii=False, indent=1))


def acc(pred, lab):
    return round(sum(p == l for p, l in zip(pred, lab)) / len(lab), 4)


def best_threshold(ps, labs):
    cands = sorted(set(ps)) + [1.01]
    return max(((acc([p >= t for p in ps], labs), -abs(t - 0.5), t) for t in cands))[2]


def band_for(ps, labs, target=1.0):
    """Maior cobertura do Laya com acerto >= target nos casos que ele decide.
    Laya aprova acima de hi e reprova abaixo de lo; entre eles vai ao Haiku."""
    cands = sorted(set([0.0, 1.0] + ps))
    best = (0, 0.0, 1.0)
    for lo in cands:
        for hi in cands:
            if hi < lo:
                continue
            dec = [(p > hi, l) for p, l in zip(ps, labs) if p < lo or p > hi]
            if not dec:
                continue
            a = sum(int(x) == l for x, l in dec) / len(dec)
            if a >= target and len(dec) > best[0]:
                best = (len(dec), lo, hi)
    return best[1], best[2]


def cmd_calibrate(args):
    rows = json.loads(pathlib.Path(args.results).read_text())
    rows = [r for r in rows if 'nota' in (r.get('haiku') or {})]
    labs = [r['rotulo'] for r in rows]
    out = {'n': len(rows), 'positivos': sum(labs), 'negativos': len(labs) - sum(labs)}
    hk = [int(bool(r['haiku']['aprovado'])) for r in rows]
    out['haiku'] = {'acerto': acc(hk, labs), 'custo_usd': round(sum(r['haiku'].get('custo_usd') or 0 for r in rows), 4),
                    'latencia_ms_mediana': statistics.median(r['haiku']['latencia_ms'] for r in rows)}
    for key in ('laya', 'laya_multi'):
        ps = [r[key]['p'] for r in rows]
        l05 = [int(p >= 0.5) for p in ps]
        thr = best_threshold(ps, labs)
        lt = [int(p >= thr) for p in ps]
        # validação cruzada por tarefa: limiar calibrado em t08, testado em t11 e vice-versa
        cv = []
        for tid in ('t08', 't11'):
            tr = [(r[key]['p'], r['rotulo']) for r in rows if r['task'] != tid]
            te = [(r[key]['p'], r['rotulo']) for r in rows if r['task'] == tid]
            t_ = best_threshold([a for a, _ in tr], [b for _, b in tr])
            cv += [int(p >= t_) == l for p, l in te]
        pos = [p for p, l in zip(ps, labs) if l]
        neg = [p for p, l in zip(ps, labs) if not l]
        auc = sum((a > b) + 0.5 * (a == b) for a in pos for b in neg) / (len(pos) * len(neg))
        out[key] = {'acerto_0.5': acc(l05, labs), 'concordancia_haiku_0.5': acc(l05, hk),
                    'limiar_otimo': thr, 'acerto_limiar_otimo': acc(lt, labs),
                    'concordancia_haiku_limiar_otimo': acc(lt, hk),
                    'acerto_cv_por_tarefa': round(sum(cv) / len(cv), 4), 'auc': round(auc, 4),
                    'p_media_pos': round(statistics.mean(pos), 4), 'p_media_neg': round(statistics.mean(neg), 4),
                    'latencia_ms_mediana': statistics.median(r[key]['latencia_ms'] for r in rows)}
        # cascade com rótulo verdadeiro como alvo (acerto 100% nos que o Laya decide)
        casc = {}
        for tgt in (1.0, 0.95):
            lo, hi = band_for(ps, labs, tgt)
            pred, to_h, cost = [], 0, 0.0
            for r, p in zip(rows, ps):
                if p < lo or p > hi:
                    pred.append(int(p > hi))
                else:
                    to_h += 1
                    pred.append(int(bool(r['haiku']['aprovado'])))
                    cost += r['haiku'].get('custo_usd') or 0
            casc[str(tgt)] = {'faixa': [lo, hi], 'fracao_haiku': round(to_h / len(rows), 4), 'acerto': acc(pred, labs),
                              'custo_usd': round(cost, 4)}
        out[key]['cascade'] = casc
    pathlib.Path(args.out).write_text(json.dumps(out, ensure_ascii=False, indent=1))
    print(json.dumps(out, ensure_ascii=False, indent=1))


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest='cmd', required=True)
    b = sub.add_parser('build')
    b.add_argument('--out', required=True)
    e = sub.add_parser('eval')
    e.add_argument('--set', required=True)
    e.add_argument('--out', required=True)
    e.add_argument('--model', default='haiku')
    e.add_argument('--no-haiku', action='store_true')
    c = sub.add_parser('calibrate')
    c.add_argument('--results', required=True)
    c.add_argument('--out', required=True)
    f = sub.add_parser('finetune-set', help='JSONL de ajuste fino: afirmações do juiz + nota do Haiku como alvo')
    f.add_argument('--set', required=True)
    f.add_argument('--results', required=True)
    f.add_argument('--out', required=True)
    k = sub.add_parser('classify')
    k.add_argument('--url', default=None)
    args = ap.parse_args()
    if args.cmd == 'build':
        items = build_set()
        with open(args.out, 'w') as f:
            for it in items:
                f.write(json.dumps(it, ensure_ascii=False) + '\n')
        print(f"{len(items)} itens: {sum(i['rotulo'] for i in items)} positivos, "
              f"{sum(1 - i['rotulo'] for i in items)} negativos, {sum(i.get('duplicata', False) for i in items)} duplicatas")
    elif args.cmd == 'eval':
        cmd_eval(args)
    elif args.cmd == 'calibrate':
        cmd_calibrate(args)
    elif args.cmd == 'finetune-set':
        items = {json.loads(ln)['id']: json.loads(ln) for ln in open(args.set)}
        n = 0
        with open(args.out, 'w') as fo:
            for r in json.loads(pathlib.Path(args.results).read_text()):
                nota = (r.get('haiku') or {}).get('nota')
                if nota is None:
                    continue
                it = items[r['id']]
                tp, _ = meta(it['task'])
                for s_ in STATEMENTS[it['task']]:
                    fo.write(json.dumps({'id': r['id'], 'task': it['task'], 'state': state_for(tp, it['files']),
                                         'question': s_, 'y': round(float(nota) / 10, 3), 'rotulo': it['rotulo'],
                                         'nota_haiku': nota}, ensure_ascii=False) + '\n')
                    n += 1
        print(f'{n} linhas em {args.out}')
    elif args.cmd == 'classify':
        sys.path.insert(0, str(pathlib.Path(__file__).parent))
        import missions_eval  # noqa: E402
        for t in missions_eval.TASKS:
            print(json.dumps(dict(task=t.id, kind=t.kind, **classify_prompt(t.prompt, args.url)), ensure_ascii=False))
    return 0


if __name__ == '__main__':
    sys.exit(main())
