#!/usr/bin/env python3
"""Harness de avaliação das missões LLM do OmniGet (Python 3.9, só stdlib).

Cada tarefa cria o próprio workspace-semente, roda UMA missão pelo executor
Claude e confere o resultado com uma rubrica automática (testes passam, arquivo
com conteúdo X, JSON válido e igual ao esperado). Opcionalmente um juiz por
modelo dá nota às tarefas abertas (tradução, resumo).

Modo de execução (`--mode cli`, o único implementado hoje)
---------------------------------------------------------
Reproduz o argv que `src-tauri/omniget-core/src/core/llm/cli_runtime/claude.rs`
(`argv`) monta em `cli_runtime/mod.rs::plan_with` para um job de projeto com
conta de escrita (SandboxMode::Write):

    claude -p --output-format stream-json --verbose
           --permission-prompts none --permission-mode acceptEdits
           --include-partial-messages [--model M] [--allowedTools ...]

e, com `--argv lean` (padrão desde a onda 2), o argv enxuto de job de projeto:
`--effort low --append-system-prompt <PROJECT_BATCHING> --tools <PROJECT_TOOLS>
--strict-mcp-config --exclude-dynamic-system-prompt-sections`.

prompt no stdin (`render_prompt` => "User: <prompt>"), cwd = workspace da
tarefa, variáveis de SCRUB_ENV removidas, conta = perfil padrão do CLI
(config_dir vazio, o login do terminal). Uso/custo vêm do evento `result`
(`usage`, `modelUsage`, `total_cost_usd`, `duration_ms`), o mesmo que
`result_usage` lê.

Diferenças em relação a um job real do app (documentadas no JSON de saída):
  * sem coordinator: sem mensagens de sistema do app (skills injetadas,
    contexto do workspace, `--append-system-prompt` do agente);
  * sem projeção MCP (`--mcp-config` + `--permission-prompt-tool`): no app os
    prompts de permissão vão para o OmniGet; aqui, `--allowedTools` emula as
    regras "Sempre" já aprovadas para os comandos de teste (python3/node);
  * `--max-budget-usd` por tarefa como trava de custo (o app passa None);
  * também removemos CLAUDECODE/CLAUDE_CODE_* herdados desta sessão (o app,
    aberto pelo Finder, não os tem).
O modo `app` (binário real + OMNIGET_TEST_DRIVER + POST /v1/debug/eval ->
llm_job_submit/llm_job_get) exige uma conta CLI logada dentro do perfil
isolado e o Vite :1420; ver `--mode app` abaixo (não implementado).

Uso:
    python3 scripts/llm/missions_eval.py --list
    python3 scripts/llm/missions_eval.py --out resultados.json [--model sonnet]
        [--tasks t01,t03] [--reps 1] [--judge [haiku|laya|cascade]] [--work DIR] [--budget 0.6]
        [--effort-router laya]   # Laya classifica o prompt: simples -> low, difícil -> esforço padrão
Juiz `laya`/`cascade` e roteador precisam de um laya-server local (wire do Jev), ver scripts/llm/laya_judge.py.
"""
import argparse
import csv
import io
import json
import os
import pathlib
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import time

SCRUB_ENV = ['ANTHROPIC_API_KEY', 'ANTHROPIC_AUTH_TOKEN', 'ANTHROPIC_BASE_URL', 'CLAUDE_CODE_OAUTH_TOKEN',
             'CLAUDE_CODE_USE_BEDROCK', 'CLAUDE_CODE_USE_VERTEX', 'OPENAI_API_KEY', 'OPENAI_BASE_URL']
# Emula as regras "Sempre" (ex.: `node *`) que o dono aprova no app.
ALLOWED_TEST_TOOLS = ['Bash(python3 *)', 'Bash(node *)', 'Bash(ls *)', 'Bash(cat *)', 'Bash(grep *)']
OWN_PIDS = set()
# Espelham cli_runtime/claude.rs: PROJECT_TOOLS e PROJECT_BATCHING (argv enxuto de job de projeto).
# ToolSearch fica na lista: sem ele o CLI carrega todo schema de MCP de uma vez (82k tokens medidos).
PROJECT_TOOLS = 'Bash,Read,Edit,Write,Glob,Grep,WebFetch,WebSearch,ToolSearch'
PROJECT_BATCHING = ('Work in as few turns as possible: batch independent tool calls in one message '
                    '(read every file you need at once), make all the edits, then run the tests once at the end. '
                    'Do not re-read files you just wrote or re-run checks that already passed. '
                    'Stop as soon as the result is verified.')
# Espelha cli_runtime/claude.rs::PROJECT_EFFORT (onda 2 r2: mediana 152,8 s -> 125,6 s, custo igual, 33/33).
PROJECT_EFFORT = 'low'


def w(root, rel, text):
    p = root / rel
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(text, encoding='utf-8')


def run_quiet(cmd, cwd, timeout=60):
    try:
        p = subprocess.run(cmd, cwd=str(cwd), capture_output=True, text=True, timeout=timeout)
        return p.returncode, (p.stdout + p.stderr)[-600:]
    except Exception as e:  # noqa: BLE001
        return 99, str(e)


def read(root, rel):
    p = root / rel
    return p.read_text(encoding='utf-8', errors='replace') if p.exists() else None


# ── tarefas ────────────────────────────────────────────────────────────
# Cada tarefa: id, kind, prompt, seed(root), check(root) -> (ok, detalhe),
# judge (opcional): critério em texto para o juiz por modelo.

TASKS = []


def task(tid, kind, prompt, judge=None):
    def deco(cls):
        cls.id, cls.kind, cls.prompt, cls.judge = tid, kind, prompt, judge
        TASKS.append(cls)
        return cls
    return deco


@task('t01-arquivo', 'arquivo',
      'Crie o arquivo notes/hello.txt contendo exatamente a linha: OmniGet mission ok (sem nada mais além de uma quebra de linha final opcional).')
class T01:
    @staticmethod
    def seed(r):
        w(r, 'README.txt', 'workspace vazio\n')

    @staticmethod
    def check(r):
        t = read(r, 'notes/hello.txt')
        return (t is not None and t.rstrip('\n') == 'OmniGet mission ok'), repr((t or '')[:60])


CSV_ROWS = [('id', 'nome', 'preco', 'estoque'), ('1', 'Cabo USB-C', '19.90', '42'), ('2', 'Fonte 65W', '149.00', '7'),
            ('3', 'Hub 4 portas', '89.50', '0'), ('4', 'Mouse sem fio', '59.99', '15'), ('5', 'SSD 1TB', '399.00', '3')]


@task('t02-csv-json', 'arquivo',
      'Converta produtos.csv em produtos.json: um array JSON de objetos com as chaves id (inteiro), nome (string), '
      'preco (número) e estoque (inteiro), na mesma ordem do CSV. Inclua só os produtos com estoque maior que zero.')
class T02:
    @staticmethod
    def seed(r):
        buf = io.StringIO()
        csv.writer(buf, lineterminator='\n').writerows(CSV_ROWS)
        w(r, 'produtos.csv', buf.getvalue())

    @staticmethod
    def check(r):
        t = read(r, 'produtos.json')
        want = [{'id': int(a), 'nome': b, 'preco': float(c), 'estoque': int(d)} for a, b, c, d in CSV_ROWS[1:] if int(d) > 0]
        try:
            got = json.loads(t or '')
        except ValueError as e:
            return False, f'json inválido: {e}'
        ok = isinstance(got, list) and len(got) == len(want) and all(
            g.get('id') == x['id'] and g.get('nome') == x['nome'] and abs(float(g.get('preco', -1)) - x['preco']) < 1e-9
            and g.get('estoque') == x['estoque'] and isinstance(g.get('id'), int) for g, x in zip(got, want))
        return ok, f'{len(got) if isinstance(got, list) else "?"} itens'


@task('t03-bugfix-py', 'codigo',
      'Os testes em test_stats.py falham. Corrija stats.py (não altere os testes) e rode `python3 -m unittest -q` para confirmar.')
class T03:
    @staticmethod
    def seed(r):
        w(r, 'stats.py', '''def mean(xs):
    return sum(xs) / len(xs) + 1


def median(xs):
    s = sorted(xs)
    n = len(s)
    return s[n // 2]


def mode(xs):
    counts = {}
    for x in xs:
        counts[x] = counts.get(x, 0) + 1
    return min(counts, key=counts.get)
''')
        w(r, 'test_stats.py', '''import unittest
from stats import mean, median, mode


class T(unittest.TestCase):
    def test_mean(self):
        self.assertEqual(mean([1, 2, 3, 4]), 2.5)

    def test_median_odd(self):
        self.assertEqual(median([3, 1, 2]), 2)

    def test_median_even(self):
        self.assertEqual(median([4, 1, 3, 2]), 2.5)

    def test_mode(self):
        self.assertEqual(mode([1, 2, 2, 3, 3, 3]), 3)

    def test_mean_empty(self):
        with self.assertRaises(ValueError):
            mean([])


if __name__ == "__main__":
    unittest.main()
''')

    @staticmethod
    def check(r):
        orig = T03_TEST_HASH
        if hash_file(r / 'test_stats.py') != orig:
            return False, 'testes alterados'
        code, out = run_quiet([sys.executable, '-m', 'unittest', '-q'], r)
        return code == 0, out.strip().splitlines()[-1] if out.strip() else ''


@task('t04-feature-py', 'codigo',
      'Implemente a função slugify(texto) em slug.py para que `python3 -m unittest -q` passe. Não altere test_slug.py.')
class T04:
    @staticmethod
    def seed(r):
        w(r, 'slug.py', 'def slugify(texto):\n    raise NotImplementedError\n')
        w(r, 'test_slug.py', '''import unittest
from slug import slugify


class T(unittest.TestCase):
    def test_basic(self):
        self.assertEqual(slugify("Olá Mundo"), "ola-mundo")

    def test_spaces_and_symbols(self):
        self.assertEqual(slugify("  Ação & Reação!!  "), "acao-reacao")

    def test_numbers(self):
        self.assertEqual(slugify("Versão 0.10.1 do OmniGet"), "versao-0-10-1-do-omniget")

    def test_collapse(self):
        self.assertEqual(slugify("a---b___c"), "a-b-c")

    def test_empty(self):
        self.assertEqual(slugify("!!!"), "")


if __name__ == "__main__":
    unittest.main()
''')

    @staticmethod
    def check(r):
        if hash_file(r / 'test_slug.py') != T04_TEST_HASH:
            return False, 'testes alterados'
        code, out = run_quiet([sys.executable, '-m', 'unittest', '-q'], r)
        return code == 0, out.strip().splitlines()[-1] if out.strip() else ''


@task('t05-bugfix-js', 'codigo',
      'Os testes de `node --test` falham. Corrija src/queue.js sem mexer em test/queue.test.js e confirme rodando `node --test`.')
class T05:
    @staticmethod
    def seed(r):
        w(r, 'package.json', '{"name":"q","version":"1.0.0","type":"commonjs"}\n')
        w(r, 'src/queue.js', '''// Fila com prioridade: menor número sai primeiro; empate sai na ordem de chegada.
class PriorityQueue {
  constructor() { this.items = []; }
  push(value, priority) { this.items.push({ value, priority }); }
  pop() {
    if (this.items.length === 0) return undefined;
    let best = 0;
    for (let i = 1; i < this.items.length; i++) {
      if (this.items[i].priority <= this.items[best].priority) best = i;
    }
    return this.items.splice(best, 1)[0];
  }
  get size() { return this.items.length - 1; }
}
module.exports = { PriorityQueue };
''')
        w(r, 'test/queue.test.js', '''const test = require('node:test');
const assert = require('node:assert');
const { PriorityQueue } = require('../src/queue.js');

test('menor prioridade primeiro', () => {
  const q = new PriorityQueue();
  q.push('b', 2); q.push('a', 1); q.push('c', 3);
  assert.strictEqual(q.pop(), 'a');
  assert.strictEqual(q.pop(), 'b');
});

test('empate respeita chegada', () => {
  const q = new PriorityQueue();
  q.push('x', 1); q.push('y', 1); q.push('z', 1);
  assert.deepStrictEqual([q.pop(), q.pop(), q.pop()], ['x', 'y', 'z']);
});

test('size e vazio', () => {
  const q = new PriorityQueue();
  assert.strictEqual(q.size, 0);
  q.push('a', 1);
  assert.strictEqual(q.size, 1);
  q.pop();
  assert.strictEqual(q.pop(), undefined);
});
''')

    @staticmethod
    def check(r):
        if hash_file(r / 'test/queue.test.js') != T05_TEST_HASH:
            return False, 'testes alterados'
        node = shutil.which('node')
        if not node:
            return False, 'node ausente'
        code, out = run_quiet([node, '--test'], r)
        m = re.search(r'# fail (\d+)', out)
        return code == 0, (m.group(0) if m else out.strip()[-80:])


SECRET = 'ORBIT-7731'


@task('t06-busca-local', 'pesquisa',
      'Em algum arquivo dentro de corpus/ existe o código de ativação que começa com "ORBIT-". Escreva em answer.txt '
      'apenas o caminho relativo (a partir da raiz do workspace) do arquivo que o contém, numa linha.')
class T06:
    @staticmethod
    def seed(r):
        words = 'alfa beta gama delta epsilon zeta eta teta iota kapa lambda mi ni xi omicron pi ro sigma tau'.split()
        for i in range(40):
            sub = ['docs', 'src', 'notas', 'arquivo/antigo'][i % 4]
            body = '\n'.join(' '.join(words[(i + j + k) % len(words)] for k in range(12)) for j in range(30))
            if i == 27:
                body += f'\n\ncódigo de ativação: {SECRET}\n'
            if i == 11:
                body += '\n\ncódigo de ativação expirado: ORBIX-0000 (não é este)\n'
            w(r, f'corpus/{sub}/arquivo_{i:02d}.txt', body)

    @staticmethod
    def check(r):
        t = (read(r, 'answer.txt') or '').strip().lstrip('./')
        return t == 'corpus/arquivo/antigo/arquivo_27.txt', repr(t[:80])  # 27 % 4 == 3


@task('t07-contagem', 'pesquisa',
      'Conte quantas linhas em arquivos .py dentro de app/ (recursivo) contêm a marca "TODO(" (exatamente assim, com o '
      'parêntese). Ignore outros tipos de arquivo. Escreva só o número em count.txt.')
class T07:
    @staticmethod
    def seed(r):
        total = 0
        for i in range(15):
            lines = []
            for j in range(20):
                if (i * 7 + j) % 9 == 0:
                    lines.append(f'x = {j}  # TODO(tonho): revisar {j}')
                    total += 1
                elif (i + j) % 11 == 0:
                    lines.append(f'y = {j}  # TODO sem parêntese')
                else:
                    lines.append(f'z_{j} = {i * j}')
            w(r, f'app/mod{i // 5}/f{i:02d}.py', '\n'.join(lines) + '\n')
            w(r, f'app/mod{i // 5}/f{i:02d}.md', '# TODO(docs): isto não conta\n')
        expected_path(r).write_text(str(total))

    @staticmethod
    def check(r):
        want = expected_path(r).read_text()
        t = (read(r, 'count.txt') or '').strip()
        return t == want, f'got={t!r} want={want}'


PT_TEXT = ('O OmniGet é um aplicativo de código aberto que baixa vídeos e músicas de mais de mil sites. '
           'Ele funciona no Windows, no macOS e no Linux, e não coleta dados de quem usa. '
           'Quando um download falha, o aplicativo mostra o motivo e sugere o que fazer em seguida.')


@task('t08-traducao', 'traducao',
      'Traduza o texto de pt.txt para inglês e grave a tradução em en.txt. Mantenha o nome OmniGet e os nomes dos '
      'sistemas operacionais. Só a tradução no arquivo.',
      judge='A tradução em inglês é fiel, fluente e completa (três frases: o que é, plataformas/privacidade, falhas)?')
class T08:
    @staticmethod
    def seed(r):
        w(r, 'pt.txt', PT_TEXT + '\n')

    @staticmethod
    def check(r):
        t = read(r, 'en.txt') or ''
        low = t.lower()
        need = ['omniget', 'open', 'windows', 'macos', 'linux', 'download', 'data']
        miss = [k for k in need if k not in low]
        pt_left = [k for k in (' não ', ' que ', ' aplicativo', ' baixa ', ' quando ') if k in f' {low} ']
        ok = bool(t.strip()) and not miss and not pt_left and 150 < len(t) < 600
        return ok, f'faltando={miss} pt={pt_left} len={len(t)}'


@task('t09-rename', 'codigo',
      'Renomeie a função calc_total para compute_order_total em todo o pacote shop/ (definição e todos os usos), '
      'sem mudar o comportamento. Depois rode `python3 -m unittest -q`.')
class T09:
    @staticmethod
    def seed(r):
        w(r, 'shop/__init__.py', '')
        w(r, 'shop/pricing.py', 'def calc_total(items, discount=0.0):\n'
                                '    subtotal = sum(q * p for q, p in items)\n'
                                '    return round(subtotal * (1 - discount), 2)\n')
        w(r, 'shop/cart.py', 'from shop.pricing import calc_total\n\n\n'
                             'class Cart:\n    def __init__(self):\n        self.items = []\n\n'
                             '    def add(self, qty, price):\n        self.items.append((qty, price))\n\n'
                             '    def total(self):\n        return calc_total(self.items)\n')
        w(r, 'shop/checkout.py', 'from shop import pricing\n\n\n'
                                 'def checkout(cart, coupon=None):\n'
                                 '    discount = 0.1 if coupon == "OMNI10" else 0.0\n'
                                 '    return pricing.calc_total(cart.items, discount)\n')
        w(r, 'test_shop.py', 'import unittest\nfrom shop.cart import Cart\nfrom shop.checkout import checkout\n'
                             'from shop.pricing import compute_order_total\n\n\n'
                             'class T(unittest.TestCase):\n'
                             '    def test_total(self):\n        c = Cart(); c.add(2, 10.0); c.add(1, 5.5)\n'
                             '        self.assertEqual(c.total(), 25.5)\n'
                             '        self.assertEqual(checkout(c, "OMNI10"), 22.95)\n'
                             '        self.assertEqual(compute_order_total([(1, 1.0)]), 1.0)\n\n\n'
                             'if __name__ == "__main__":\n    unittest.main()\n')

    @staticmethod
    def check(r):
        left = [p.name for p in (r / 'shop').glob('*.py') if 'calc_total' in p.read_text()]
        code, out = run_quiet([sys.executable, '-m', 'unittest', '-q'], r)
        return code == 0 and not left, f'exit={code} restos={left}'


LOG_LEVELS = ['INFO', 'WARN', 'ERROR', 'DEBUG']


@task('t10-log-json', 'arquivo',
      'Leia app.log e grave report.json com um objeto {"levels": {NIVEL: quantidade}, "top_error": "<mensagem de ERROR '
      'mais frequente>"}. Os níveis aparecem entre colchetes em cada linha. Inclua só níveis que aparecem.')
class T10:
    @staticmethod
    def seed(r):
        errs = ['timeout ao baixar fragmento', 'HTTP 403 no manifesto', 'timeout ao baixar fragmento', 'disco cheio']
        lines = []
        for i in range(120):
            lvl = LOG_LEVELS[(i * 5 + i // 7) % 4] if i % 13 else 'ERROR'
            msg = errs[i % 4] if lvl == 'ERROR' else f'evento {i}'
            lines.append(f'2026-09-26T10:{i // 60:02d}:{i % 60:02d}Z [{lvl}] {msg}')
        w(r, 'app.log', '\n'.join(lines) + '\n')
        levels, top = {}, {}
        for ln in lines:
            lvl = re.search(r'\[(\w+)\]', ln).group(1)
            levels[lvl] = levels.get(lvl, 0) + 1
            if lvl == 'ERROR':
                m = ln.split('] ', 1)[1]
                top[m] = top.get(m, 0) + 1
        expected_path(r).write_text(json.dumps({'levels': levels, 'top_error': max(top, key=top.get)}))

    @staticmethod
    def check(r):
        want = json.loads(expected_path(r).read_text())
        try:
            got = json.loads(read(r, 'report.json') or '')
        except ValueError as e:
            return False, f'json inválido: {e}'
        return got.get('levels') == want['levels'] and got.get('top_error') == want['top_error'], \
            f"levels={got.get('levels')} top={got.get('top_error')!r}"


@task('t11-resumo', 'traducao',
      'Leia changelog.txt e escreva summary.md em inglês com exatamente 3 tópicos (linhas começando com "- ") '
      'resumindo as mudanças mais importantes para o usuário.',
      judge='Os 3 tópicos em inglês resumem corretamente as mudanças mais importantes do changelog?')
class T11:
    @staticmethod
    def seed(r):
        w(r, 'changelog.txt', '0.10.1\n- Correção: downloads do TikTok voltaram a funcionar depois da mudança da API.\n'
                              '- Novo: fila respeita limite de 2 downloads por site.\n- Interno: refatoração do módulo de erros.\n'
                              '- Novo: legendas automáticas podem ser salvas em SRT.\n- Correção: travamento ao pausar downloads HLS.\n'
                              '- Interno: atualização de dependências.\n')

    @staticmethod
    def check(r):
        t = read(r, 'summary.md') or ''
        bullets = [ln for ln in t.splitlines() if ln.startswith('- ')]
        low = t.lower()
        hits = sum(k in low for k in ('tiktok', 'srt', 'subtitle', 'hls', 'per site', 'queue'))
        return len(bullets) == 3 and hits >= 2, f'bullets={len(bullets)} hits={hits}'


def expected_path(r):
    """Gabarito fora do workspace, para o agente não ler."""
    return r.parent / (r.name + '.expected')


def hash_file(p):
    import hashlib
    return hashlib.sha256(p.read_bytes()).hexdigest() if p.exists() else None


def _seed_hash(cls, rel):
    d = pathlib.Path(tempfile.mkdtemp(prefix='mseed-'))
    cls.seed(d)
    h = hash_file(d / rel)
    shutil.rmtree(d, ignore_errors=True)
    return h


T03_TEST_HASH = _seed_hash(T03, 'test_stats.py')
T04_TEST_HASH = _seed_hash(T04, 'test_slug.py')
T05_TEST_HASH = _seed_hash(T05, 'test/queue.test.js')


# ── execução ───────────────────────────────────────────────────────────

def child_env():
    env = {k: v for k, v in os.environ.items()
           if k not in SCRUB_ENV and not k.startswith('CLAUDECODE') and not k.startswith('CLAUDE_CODE_')}
    return env


def claude_argv(args, model=None, effort=None):
    """Espelha claude::argv para SandboxMode::Write sem projeção.

    `--argv lean` (padrão) = job de projeto da onda 2 (plan_with com lean_project);
    `--argv baseline` = argv da onda 1, para comparar."""
    lean = getattr(args, 'argv', 'lean') == 'lean'
    out = ['-p', '--output-format', 'stream-json', '--verbose',
           '--permission-prompts', 'none', '--permission-mode', 'acceptEdits',
           '--include-partial-messages']
    model = args.model if model is None else model
    if model:
        out += ['--model', model]
    if effort is None:
        effort = getattr(args, 'effort', None)
    if effort is None:  # espelha plan_with: job de projeto enxuto sem reasoning_effort -> PROJECT_EFFORT
        effort = PROJECT_EFFORT if lean else ''
    if effort and effort != 'default':
        out += ['--effort', effort]
    if lean and getattr(args, 'batching', True):
        out += ['--append-system-prompt', PROJECT_BATCHING]
    if args.budget:
        out += ['--max-budget-usd', str(args.budget)]
    if lean:
        out += ['--tools', PROJECT_TOOLS, '--strict-mcp-config', '--exclude-dynamic-system-prompt-sections']
    if args.allow_tests:
        out += ['--allowedTools', ','.join(ALLOWED_TEST_TOOLS)]
    return out


def effort_of(argv):
    return argv[argv.index('--effort') + 1] if '--effort' in argv else '(padrão)'


def model_for(args, t):
    """`--simple-model` roteia tarefas de tipo simples (arquivo/pesquisa/tradução) para outro modelo."""
    if getattr(args, 'simple_model', '') and t.kind in SIMPLE_KINDS:
        return args.simple_model
    return args.model


SIMPLE_KINDS = ('arquivo', 'pesquisa', 'traducao')
ROUTES = {}


def effort_for(args, t):
    """`--effort-router laya`: o Laya classifica o prompt (simples/difícil); simples -> low,
    difícil -> esforço padrão do CLI (sem --effort). Sem roteador: (None, None) = política atual."""
    if getattr(args, 'effort_router', '') != 'laya':
        return None, None
    if t.id not in ROUTES:
        import laya_judge as lj
        c = lj.classify_prompt(t.prompt, args.laya_url)
        c['hard'] = c['p_hard'] >= args.router_threshold
        ROUTES[t.id] = c
    c = ROUTES[t.id]
    return ('default' if c['hard'] else 'low'), c


def run_claude(args, prompt, cwd, timeout, model=None, effort=None):
    """Uma missão: devolve (result_event, tool_calls, pid, stderr_tail, timed_out)."""
    binary = shutil.which('claude') or os.path.expanduser('~/.local/bin/claude')
    stdin = 'User: ' + prompt  # claude::render_prompt de uma única mensagem de usuário
    p = subprocess.Popen([binary] + claude_argv(args, model, effort), cwd=str(cwd), env=child_env(), stdin=subprocess.PIPE,
                         stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, start_new_session=True)
    OWN_PIDS.add(p.pid)
    timed_out = False
    try:
        out, err = p.communicate(stdin, timeout=timeout)
    except subprocess.TimeoutExpired:
        timed_out = True
        try:
            os.killpg(p.pid, signal.SIGTERM)  # grupo próprio (start_new_session): só filhos desta missão
        except OSError:
            pass
        try:
            out, err = p.communicate(timeout=10)
        except subprocess.TimeoutExpired:
            os.killpg(p.pid, signal.SIGKILL)
            out, err = p.communicate()
    OWN_PIDS.discard(p.pid)
    result, tools, first_text_ms = None, [], None
    for line in (out or '').splitlines():
        try:
            ev = json.loads(line)
        except ValueError:
            continue
        if ev.get('type') == 'result':
            result = ev
        elif ev.get('type') == 'assistant':
            for b in (ev.get('message') or {}).get('content') or []:
                if b.get('type') == 'tool_use':
                    tools.append(b.get('name'))
    return result, tools, p.pid, (err or '')[-400:], timed_out


def usage_of(result):
    u = {'input': 0, 'output': 0, 'cache_read': 0, 'cache_write': 0}
    if not result:
        return u, None, {}
    mu = result.get('modelUsage') or {}
    if mu:
        for m in mu.values():
            u['input'] += int(m.get('inputTokens', 0))
            u['output'] += int(m.get('outputTokens', 0))
            u['cache_read'] += int(m.get('cacheReadInputTokens', 0))
            u['cache_write'] += int(m.get('cacheCreationInputTokens', 0))
    else:
        us = result.get('usage') or {}
        u = {'input': us.get('input_tokens', 0), 'output': us.get('output_tokens', 0),
             'cache_read': us.get('cache_read_input_tokens', 0), 'cache_write': us.get('cache_creation_input_tokens', 0)}
    models = {k: round(v.get('costUSD', 0), 5) for k, v in mu.items()}
    return u, result.get('total_cost_usd'), models


def judge(args, t, root):
    files = {p.name: p.read_text(errors='replace')[:3000] for p in root.iterdir() if p.is_file() and not p.name.startswith('.')}
    if args.judge in ('laya', 'cascade'):
        import laya_judge as lj
        try:
            if args.judge == 'laya':
                v = lj.laya_verdict(t.id, t.prompt, t.judge, files, args.laya_url)
                return {'juiz': 'laya', 'nota': round(v['p'] * 10, 1), 'aprovado': v['aprovado'], 'p': v['p'],
                        'latencia_ms': v['latencia_ms'], 'custo_usd': 0.0}
            band = tuple(float(x) for x in args.cascade_band.split(',')) if args.cascade_band else None
            v = lj.cascade_verdict(t.id, t.prompt, t.judge, files, band, args.laya_url, args.judge_model, str(root))
            return {'juiz': 'cascade', 'decidiu': v['decidiu'], 'nota': v.get('nota'), 'aprovado': v['aprovado'],
                    'p_laya': v['laya']['p'], 'custo_usd': v.get('custo_usd') or 0.0}
        except Exception as e:  # noqa: BLE001  (laya-server fora do ar)
            return {'erro': f'laya: {e}'[:200]}
    prompt = (f'Você é um avaliador. Tarefa dada ao agente: {t.prompt}\nCritério: {t.judge}\n'
              f'Arquivos do workspace após a tarefa: {json.dumps(files, ensure_ascii=False)}\n'
              'Responda só com um JSON {"nota": 0-10, "motivo": "..."}')
    p = subprocess.run([shutil.which('claude') or 'claude', '-p', '--output-format', 'json', '--model', args.judge_model,
                        '--tools', '', '--no-session-persistence'], input=prompt, capture_output=True, text=True,
                       env=child_env(), timeout=180, cwd=str(root))
    try:
        res = json.loads(p.stdout)
        m = re.search(r'\{.*\}', res.get('result', ''), re.S)
        verdict = json.loads(m.group(0)) if m else {}
        return {'nota': verdict.get('nota'), 'motivo': str(verdict.get('motivo', ''))[:200],
                'custo_usd': res.get('total_cost_usd')}
    except Exception as e:  # noqa: BLE001
        return {'erro': str(e)[:200]}


def run_task(args, t, rep):
    root = pathlib.Path(args.work) / f'{t.id}-r{rep}'
    if root.exists():
        shutil.rmtree(root)
    root.mkdir(parents=True)
    t.seed(root)
    t0 = time.time()
    model = model_for(args, t)
    effort, rota = effort_for(args, t)
    result, tools, pid, err, timed_out = run_claude(args, t.prompt, root, args.timeout, model, effort)
    wall = time.time() - t0
    ok, detail = t.check(root)
    tok, cost, models = usage_of(result)
    row = {'task': t.id, 'kind': t.kind, 'rep': rep, 'model': model or '(padrão)', 'sucesso': bool(ok), 'detalhe': detail,
           'tempo_s': round(wall, 2), 'duration_ms_cli': (result or {}).get('duration_ms'),
           'num_turns': (result or {}).get('num_turns'), 'tokens': tok, 'tokens_total': sum(tok.values()),
           'custo_usd': cost, 'custo_por_modelo': models, 'tool_calls': len(tools),
           'tools': sorted(set(t_ for t_ in tools if t_)), 'pid': pid, 'timeout': timed_out,
           'subtype': (result or {}).get('subtype'), 'is_error': (result or {}).get('is_error'),
           'denials': len((result or {}).get('permission_denials') or []), 'workspace': str(root)}
    row['effort'] = effort_of(claude_argv(args, model, effort))
    if rota:
        row['roteador'] = rota
    if not result:
        row['stderr'] = err
    if args.judge and t.judge:
        row['juiz'] = judge(args, t, root)
    return row


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--mode', default='cli', choices=['cli', 'app'])
    ap.add_argument('--out', help='arquivo JSON de resultados')
    ap.add_argument('--tasks', help='ids separados por vírgula (prefixo basta, ex.: t01,t03)')
    ap.add_argument('--reps', type=int, default=1)
    ap.add_argument('--model', default='sonnet', help="modelo do CLI ('' = padrão da conta, como o app sem política)")
    ap.add_argument('--budget', type=float, default=0.75, help='--max-budget-usd por missão (0 = sem trava)')
    ap.add_argument('--timeout', type=int, default=300)
    ap.add_argument('--no-allow-tests', dest='allow_tests', action='store_false',
                    help='não emular regras Sempre para python3/node (Bash negado, como --permission-prompts none puro)')
    ap.add_argument('--judge', nargs='?', const='haiku', default=None, choices=['haiku', 'laya', 'cascade'],
                    help='juiz nas tarefas abertas: haiku (CLI), laya (laya-server local) ou cascade '
                         '(Laya decide fora da faixa de incerteza, senão Haiku); só --judge = haiku')
    ap.add_argument('--laya-url', default=os.environ.get('LAYA_URL', 'http://127.0.0.1:8766'))
    ap.add_argument('--cascade-band', default='', help='lo,hi da faixa de incerteza do Laya (padrão: laya_judge.CASCADE_BAND)')
    ap.add_argument('--effort-router', default='', choices=['', 'laya'],
                    help='laya = classifica o prompt: simples -> --effort low, difícil -> sem --effort')
    ap.add_argument('--router-threshold', type=float, default=0.5, help='p(difícil) mínima para esforço padrão')
    ap.add_argument('--judge-model', default='haiku')
    ap.add_argument('--work', default=os.path.join(tempfile.gettempdir(), 'omniget-missions-eval'))
    ap.add_argument('--argv', default='lean', choices=['lean', 'baseline'],
                    help='lean = argv enxuto de job de projeto (onda 2); baseline = argv da onda 1')
    ap.add_argument('--effort', default=None, choices=['default', 'low', 'medium', 'high', 'xhigh', 'max'],
                    help='--effort do CLI; omitido = como o app (lean -> PROJECT_EFFORT=low); default = sem a flag')
    ap.add_argument('--simple-model', default='',
                    help='modelo para tarefas simples (arquivo/pesquisa/tradução), ex.: haiku')
    ap.add_argument('--no-batching', dest='batching', action='store_false',
                    help='sem o --append-system-prompt PROJECT_BATCHING (mede o efeito da instrução de lote)')
    ap.add_argument('--list', action='store_true')
    args = ap.parse_args()
    if args.list:
        for t in TASKS:
            print(f'{t.id:18} {t.kind:9} {t.prompt[:90]}')
        return 0
    if args.mode == 'app':
        print('modo app não implementado: precisa de conta CLI logada no perfil isolado (llm_accounts_create cria '
              'config_dir vazio) + Vite :1420 para o binário debug. Use --mode cli.', file=sys.stderr)
        return 2
    sel = TASKS
    if args.tasks:
        pref = [x.strip() for x in args.tasks.split(',') if x.strip()]
        sel = [t for t in TASKS if any(t.id.startswith(p) for p in pref)]
    pathlib.Path(args.work).mkdir(parents=True, exist_ok=True)
    rows = []
    t_all = time.time()
    try:
        for rep in range(args.reps):
            for t in sel:
                row = run_task(args, t, rep)
                rows.append(row)
                print(f"{row['task']:18} {'OK ' if row['sucesso'] else 'FAIL'} {row['tempo_s']:7.1f}s "
                      f"tok={row['tokens_total']:>8} ${row['custo_usd'] or 0:.4f} tools={row['tool_calls']} {row['detalhe']}",
                      flush=True)
    finally:
        for pid in list(OWN_PIDS):  # só os PIDs que este processo criou
            try:
                os.killpg(pid, signal.SIGTERM)
            except OSError:
                pass
    n = len(rows)
    total = {
        'tarefas': n, 'sucesso': sum(r['sucesso'] for r in rows), 'taxa_sucesso': round(sum(r['sucesso'] for r in rows) / n, 3) if n else 0,
        'tokens': {k: sum(r['tokens'][k] for r in rows) for k in ('input', 'output', 'cache_read', 'cache_write')},
        'tokens_total': sum(r['tokens_total'] for r in rows),
        'custo_usd': round(sum(r['custo_usd'] or 0 for r in rows), 4),
        'custo_juiz_usd': round(sum((r.get('juiz') or {}).get('custo_usd') or 0 for r in rows), 4),
        'tempo_s': round(sum(r['tempo_s'] for r in rows), 1), 'wall_s': round(time.time() - t_all, 1),
    }
    ver = subprocess.run([shutil.which('claude') or 'claude', '--version'], capture_output=True, text=True).stdout.strip()
    report = {
        'harness': 'scripts/llm/missions_eval.py', 'mode': args.mode, 'claude_version': ver,
        'model': args.model or '(padrão da conta)', 'effort': effort_of(claude_argv(args)),
        'simple_model': args.simple_model or None, 'judge': args.judge, 'effort_router': args.effort_router or None,
        'rotas': ROUTES or None, 'batching': args.batching, 'argv_mode': args.argv, 'argv': ['claude'] + claude_argv(args),
        'budget_por_missao_usd': args.budget, 'reps': args.reps, 'quando': time.strftime('%Y-%m-%dT%H:%M:%S%z'),
        'diferencas_vs_app': [
            'sem coordinator: sem mensagens de sistema do app (skills, contexto) nem --append-system-prompt do agente',
            'sem projeção MCP/--permission-prompt-tool; --allowedTools emula regras Sempre de python3/node',
            '--max-budget-usd por missão como trava (o app passa None)',
            'CLAUDECODE/CLAUDE_CODE_* removidos do ambiente (herdados da sessão de quem roda)',
        ],
        'total': total, 'tarefas': rows,
    }
    if args.out:
        pathlib.Path(args.out).parent.mkdir(parents=True, exist_ok=True)
        pathlib.Path(args.out).write_text(json.dumps(report, ensure_ascii=False, indent=2))
    print(json.dumps(total, ensure_ascii=False))
    return 0 if total['sucesso'] == n else 1


if __name__ == '__main__':
    sys.exit(main())
