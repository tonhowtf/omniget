"""Canonical app+core libtest preparation. Prepared children never invoke Cargo."""
import hashlib
import json
import os
import pathlib
import re
import shutil
import subprocess
import time

ROOT = pathlib.Path(__file__).resolve().parents[2]
TAURI = ROOT / 'src-tauri'
PACKAGES = ['omniget', 'omniget-core']

def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def source_files():
    excluded = {'target', 'binaries', '__pycache__', 'node_modules', '.git'}
    configured = pathlib.Path(os.environ.get('CARGO_TARGET_DIR', str(TAURI / 'target')))
    if not configured.is_absolute():
        configured = TAURI / configured
    configured = configured.resolve()
    for base in ['src-tauri', 'src', 'static', 'browser-extension', 'scripts/llm', 'rust-toolchain.toml', 'package.json', 'pnpm-lock.yaml', 'LICENSE']:
        root = ROOT / base
        if root.is_file():
            yield root
        elif root.is_dir():
            for directory, dirs, files in os.walk(root):
                dirs[:] = sorted(d for d in dirs if d not in excluded and (pathlib.Path(directory) / d).resolve() != configured)
                for name in sorted(files):
                    yield pathlib.Path(directory) / name

def sources():
    h = hashlib.sha256()
    for p in source_files():
        h.update(str(p.relative_to(ROOT)).encode()); h.update(b'\0'); h.update(p.read_bytes())
    return h.hexdigest()

def source_stamp():
    # Also invalidate edit-and-revert during compilation, even if content matches.
    return [(str(p), p.stat().st_mtime_ns, p.stat().st_size) for p in source_files()]

def prepare(directory):
    directory = pathlib.Path(directory).resolve()
    directory.mkdir(parents=True, exist_ok=True)
    if (directory / 'prepared.json').exists():
        raise RuntimeError('ALREADY_PREPARED: use the existing manifest or a new output directory')
    lock = directory / '.preparing'
    fd = os.open(lock, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
    os.close(fd)
    try:
        return _prepare(directory)
    finally:
        lock.unlink()

def _prepare(directory):
    directory = pathlib.Path(directory).resolve()
    directory.mkdir(parents=True, exist_ok=True)
    before = sources()
    stamp = source_stamp()
    command = ['cargo', 'test', '--no-run', '--lib', '-p', 'omniget', '-p', 'omniget-core', '--message-format=json']
    started = time.monotonic()
    p = subprocess.run(command, cwd=TAURI, capture_output=True, text=True)
    (directory / 'cargo.log').write_text(p.stdout + p.stderr)
    if p.returncode:
        raise RuntimeError('PREPARATION_FAILED: ' + str(directory / 'cargo.log'))
    if sources() != before or source_stamp() != stamp:
        raise RuntimeError('SOURCES_CHANGED_DURING_BUILD')
    artifacts = {}
    for line in p.stdout.splitlines():
        try:
            v = json.loads(line)
        except ValueError:
            continue
        if v.get('reason') != 'compiler-artifact' or not v.get('executable') or not v['profile']['test']:
            continue
        name = {'omniget_lib': 'omniget', 'omniget_core': 'omniget-core'}.get(v['target']['name'])
        if name:
            source = pathlib.Path(v['executable'])
            dest = directory / (name + source.suffix)
            shutil.copy2(source, dest)
            artifacts[name] = {'path': str(dest), 'digest': digest(dest), 'deps': str(source.parent),
                               'cwd': str(TAURI if name == 'omniget' else TAURI / name)}
    if set(artifacts) != set(PACKAGES):
        raise RuntimeError('MISSING_TEST_ARTIFACT')
    toolchain = subprocess.check_output(['rustc', '-vV'], cwd=ROOT, text=True)
    sysroot = subprocess.check_output(['rustc', '--print', 'sysroot'], cwd=ROOT, text=True).strip()
    host = re.search(r'^host: (.+)$', toolchain, re.M)[1]
    manifest = {'schemaVersion': 1, 'root': str(ROOT), 'sourceDigest': before, 'artifacts': artifacts,
                'command': command, 'profile': 'test', 'target': 'implicit host', 'toolchain': toolchain,
                'rustLib': str(pathlib.Path(sysroot) / 'lib/rustlib' / host / 'lib'),
                'configuration': {k: os.environ.get(k) for k in ['RUSTFLAGS','CARGO_ENCODED_RUSTFLAGS','RUSTC_WRAPPER','CARGO_TARGET_DIR']},
                'seconds': time.monotonic()-started}
    manifest['buildId'] = hashlib.sha256(json.dumps(manifest, sort_keys=True).encode()).hexdigest()
    out = directory / 'prepared.json'
    out.write_text(json.dumps(manifest, indent=2))
    return out, manifest['seconds']

def run_tests(crate, filter_, env):
    prepared = env.get('OMNIGET_PREPARED_TESTS')
    if not prepared:
        cmd = ['cargo','test','--lib','-p','omniget','-p','omniget-core',filter_,'--','--test-threads=1']
        return cmd, subprocess.run(cmd, cwd=TAURI, env=env, capture_output=True, text=True)
    manifest = json.loads(pathlib.Path(prepared).read_text())
    if manifest['root'] != str(ROOT) or manifest['sourceDigest'] != sources():
        raise RuntimeError('STALE_PREPARATION')
    for k,v in manifest['configuration'].items():
        if env.get(k) != v:
            raise RuntimeError('PREPARED_CONFIGURATION_CHANGED: ' + k)
    artifact = manifest['artifacts'][crate]
    if digest(pathlib.Path(artifact['path'])) != artifact['digest']:
        raise RuntimeError('ARTIFACT_CHANGED')
    runtime = dict(env, CARGO_MANIFEST_DIR=artifact['cwd'])
    deps = pathlib.Path(artifact['deps'])
    dirs = [str(deps),str(deps.parent),manifest['rustLib']]
    dirs += [str(p) for p in (deps.parent/'build').glob('*/out') if p.is_dir()]
    for key in ['DYLD_FALLBACK_LIBRARY_PATH','LD_LIBRARY_PATH','PATH']:
        runtime[key] = os.pathsep.join(dirs + [env.get(key,'')])
    cmd = [artifact['path'],filter_,'--test-threads=1']
    p = subprocess.run(cmd,cwd=artifact['cwd'],env=runtime,capture_output=True,text=True)
    if sources() != manifest['sourceDigest']:
        raise RuntimeError('SOURCES_CHANGED_DURING_RUN')
    if p.returncode == 0 and not re.search(r'test result: ok\. [1-9]\d* passed;',p.stdout):
        raise RuntimeError('ZERO_TESTS_EXECUTED: ' + filter_)
    return cmd,p

if __name__ == '__main__':
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', default=str(ROOT / 'estudos/bench' / str(time.time_ns()) / 'prepared'))
    args = parser.parse_args()
    manifest, seconds = prepare(args.out)
    print(json.dumps({'prepared': str(manifest), 'seconds': seconds}))
