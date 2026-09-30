#!/usr/bin/env bash
# Compila o skin-tokens.cpp (auto-rig da cidade 3D) num commit fixado e
# empacota o `skintokens-cli` com as bibliotecas do ggml ao lado, num
# `.tar.gz` plano que o app extrai em `<app_data>/city3d/skintokens/bin/`.
#
# Usado pelo `.github/workflows/skintokens.yml` e para validar no Mac:
#
#   scripts/skintokens/build.sh <plataforma> <pasta-de-saida> [checkout]
#   scripts/skintokens/build.sh --commit        # imprime o commit fixado
#
# Plataformas: macos-arm64 (Metal + CPU), linux-x64 (CPU + Vulkan),
# windows-x64 (CPU + Vulkan). Sem `checkout`, clona o repo no commit fixado.
# Saida: `skintokens-cli-<plataforma>.tar.gz` e o `.sha256` dele.
#
# Ao trocar o commit, rode o workflow de novo e cole os sha256 novos em
# `src-tauri/omniget-asset3d/src/providers/skintokens.rs` (CLI_RELEASE).
set -euo pipefail

SKINTOKENS_REPO="https://github.com/localai-org/skin-tokens.cpp"
SKINTOKENS_COMMIT="37b28284d0015c4e61e2657072f3b0f5166af207"
# nlohmann-json e dependencia de build; vem do release oficial em todo sistema.
JSON_URL="https://github.com/nlohmann/json/releases/download/v3.11.3/json.tar.xz"
JSON_SHA256="d6c65aca6b1ed68e7a182f4757257b107ae403032760ed6ef121c9d55e81757d"

if [[ "${1:-}" == "--commit" ]]; then
  echo "$SKINTOKENS_COMMIT"
  exit 0
fi

PLATFORM="${1:?plataforma: macos-arm64 | linux-x64 | windows-x64}"
OUT="${2:?pasta de saida}"
SRC="${3:-}"

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  else
    shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

mkdir -p "$OUT"
OUT="$(cd "$OUT" && pwd)"
WORK="$OUT/work-$PLATFORM"
mkdir -p "$WORK"

if [[ -z "$SRC" ]]; then
  SRC="$WORK/skin-tokens.cpp"
  if [[ ! -d "$SRC/.git" ]]; then
    git clone --filter=blob:none "$SKINTOKENS_REPO" "$SRC"
  fi
  git -C "$SRC" fetch --depth 1 origin "$SKINTOKENS_COMMIT" || true
  git -C "$SRC" checkout --quiet "$SKINTOKENS_COMMIT"
  git -C "$SRC" submodule update --init --recursive
fi
SRC="$(cd "$SRC" && pwd)"
HEAD="$(git -C "$SRC" rev-parse HEAD)"
if [[ "$HEAD" != "$SKINTOKENS_COMMIT" ]]; then
  echo "checkout em $HEAD, esperado $SKINTOKENS_COMMIT" >&2
  exit 1
fi

# nlohmann-json num prefixo local.
JSON_PREFIX="$WORK/json-prefix"
if [[ ! -f "$JSON_PREFIX/share/cmake/nlohmann_json/nlohmann_jsonConfig.cmake" ]]; then
  curl -fsSL "$JSON_URL" -o "$WORK/json.tar.xz"
  got="$(sha256_of "$WORK/json.tar.xz")"
  if [[ "$got" != "$JSON_SHA256" ]]; then
    echo "json.tar.xz: sha256 $got, esperado $JSON_SHA256" >&2
    exit 1
  fi
  rm -rf "$WORK/json"
  tar -xf "$WORK/json.tar.xz" -C "$WORK"
  cmake -S "$WORK/json" -B "$WORK/json-build" -DJSON_BuildTests=OFF \
    -DCMAKE_INSTALL_PREFIX="$JSON_PREFIX" >/dev/null
  cmake --install "$WORK/json-build" >/dev/null
fi

# Tudo instalado plano em `bin/`: o CLI, a libskintokens, o ggml e os backends
# dinamicos (que o ggml carrega de `GGML_BACKEND_PATH`, a pasta do binario).
# No Linux o upstream poe `$ORIGIN/../<libdir>` no rpath; com libdir = bin isso
# vira a propria pasta. No macOS o rpath e `@loader_path`.
CONFIG=(
  -DCMAKE_BUILD_TYPE=Release
  -DCMAKE_PREFIX_PATH="$JSON_PREFIX"
  -DCMAKE_INSTALL_BINDIR=bin
  -DCMAKE_INSTALL_LIBDIR=bin
  -DSKINTOKENS_BUILD_TESTS=OFF
  -DSKINTOKENS_DYNAMIC_BACKENDS=ON
)
GEN=(-G Ninja)
case "$PLATFORM" in
  macos-arm64)
    CONFIG+=(
      -DSKINTOKENS_ENABLE_VULKAN=OFF
      -DSKINTOKENS_CPU_ALL_VARIANTS=ON
      -DCMAKE_OSX_ARCHITECTURES=arm64
      -DCMAKE_OSX_DEPLOYMENT_TARGET=13.3
      -DCMAKE_INSTALL_RPATH=@loader_path
    )
    ;;
  linux-x64)
    CONFIG+=(
      -DSKINTOKENS_ENABLE_VULKAN=ON
      -DSKINTOKENS_CPU_ALL_VARIANTS=ON
      '-DCMAKE_INSTALL_RPATH=$ORIGIN'
    )
    ;;
  windows-x64)
    # Gerador do Visual Studio (MSVC). As variantes de CPU do ggml pedem
    # GCC/Clang, entao o Windows leva um backend de CPU so.
    GEN=(-G "Visual Studio 17 2022" -A x64)
    CONFIG+=(
      -DSKINTOKENS_ENABLE_VULKAN=ON
      -DSKINTOKENS_CPU_ALL_VARIANTS=OFF
    )
    ;;
  *)
    echo "plataforma desconhecida: $PLATFORM" >&2
    exit 1
    ;;
esac

BUILD="$WORK/build"
STAGE="$WORK/stage"
cmake -S "$SRC" -B "$BUILD" "${GEN[@]}" "${CONFIG[@]}"
cmake --build "$BUILD" --config Release --parallel
rm -rf "$STAGE"
cmake --install "$BUILD" --config Release --prefix "$STAGE"

PKG="$WORK/pkg"
rm -rf "$PKG"
mkdir -p "$PKG"
# So o que roda: binario e bibliotecas (sem headers, .lib de import, cmake).
find "$STAGE/bin" -maxdepth 1 \( -type f -o -type l \) \
  \( -name 'skintokens-cli*' -o -name '*.dll' -o -name '*.dylib' -o -name '*.so' -o -name '*.so.*' \) \
  -exec cp -P {} "$PKG/" \;
cp "$SRC/LICENSE" "$PKG/LICENSE-skin-tokens.cpp.txt"
[[ -f "$SRC/NOTICE" ]] && cp "$SRC/NOTICE" "$PKG/NOTICE-skin-tokens.cpp.txt"
cp "$SRC/ggml/LICENSE" "$PKG/LICENSE-ggml.txt"
printf 'skin-tokens.cpp %s (%s)\nplatform %s\n' \
  "$SKINTOKENS_COMMIT" "$SKINTOKENS_REPO" "$PLATFORM" > "$PKG/BUILD-INFO.txt"

EXE="$PKG/skintokens-cli"
[[ "$PLATFORM" == windows-x64 ]] && EXE="$PKG/skintokens-cli.exe"
if [[ ! -f "$EXE" ]]; then
  echo "o install nao gerou $EXE" >&2
  exit 1
fi

# O pacote tem que rodar fora da pasta de build.
case "$PLATFORM" in
  macos-arm64)
    # Linhas de dependencia (com tab) e rpaths; o cabecalho do otool e o
    # proprio arquivo e contem a pasta de trabalho.
    if { otool -L "$EXE" "$PKG"/*.dylib "$PKG"/*.so | grep $'^\t'
         otool -l "$EXE" "$PKG"/*.dylib "$PKG"/*.so | grep -A2 LC_RPATH; } | grep -F "$WORK"; then
      echo "biblioteca aponta para a pasta de build" >&2
      exit 1
    fi
    ;;
  linux-x64)
    if LD_LIBRARY_PATH= ldd "$EXE" | grep 'not found'; then
      echo "biblioteca faltando ao lado do binario" >&2
      exit 1
    fi
    ;;
esac
# Sem argumentos o CLI imprime o uso (codigo 2); isso ja prova que ele acha
# a libskintokens e o ggml ao lado. Os backends carregam no `inspect`.
usage="$(GGML_BACKEND_PATH="$PKG" "$EXE" 2>&1 || true)"
if [[ "$usage" != *"usage:"* ]]; then
  echo "$usage" >&2
  echo "$EXE nao roda fora da pasta de build" >&2
  exit 1
fi

ARCHIVE="$OUT/skintokens-cli-$PLATFORM.tar.gz"
rm -f "$ARCHIVE"
# COPYFILE_DISABLE: sem os `._*` do macOS dentro do pacote.
( cd "$PKG" && COPYFILE_DISABLE=1 tar -czf "$ARCHIVE" * )
SUM="$(sha256_of "$ARCHIVE")"
echo "$SUM  $(basename "$ARCHIVE")" > "$ARCHIVE.sha256"
echo "pacote: $ARCHIVE"
echo "sha256: $SUM"
echo "bytes:  $(wc -c < "$ARCHIVE" | tr -d ' ')"
