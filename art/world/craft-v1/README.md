# Cenário artesanal v1

Sete assets produzidos para a direção solicitada, inspirada em dioramas de papelão, feltro e madeira de brinquedo. O feltro foi gerado pela ferramenta ImageGen; os outros seis assets, pelo plugin Higgsfield com GPT Image 2.5, qualidade high, resolução 1k. Prompts, resultados e parâmetros: `generation.json`. Originais preservados nestas pastas `floor/` e `object/`.

## Integração

- `art/world/craft-v1-src`: fontes prontas para empacotar, com sete repinturas, quinze assets originais preservados e cinco novos tipos de objetos.
- `static/world/tiles/craft-v1`: atlas de 27 frames, uma página de 512×512, **1 MiB RGBA8** sem mipmaps. Os 22 contratos originais são preservados; cinco novos tipos de objetos têm footprints e pivôs próprios.
- `src/lib/world/assets.ts`: tema artesanal selecionado ao carregar o mundo, sem alterar o hash do mapa salvo.
- `static/world/yard-v1.json`: fazendinha com seis canteiros, cinco árvores, sete cercas, flores e galpão.

A casa normal recebe o novo tema. O quintal composto está disponível como prévia; **não foi feita migração da simulação dos mundos existentes para o mapa ampliado**. A aparência não altera o agendamento de trabalho. O renderer atual é isométrico (pisos 64×32); o preparador projeta as texturas para o losango e mantém o alpha de cobertura do piso original.

## Reconstruir

Requer Python com Pillow e Rust. Não faz novas chamadas pagas:

```sh
python3 scripts/art/build-craft-atlas.py
cargo run --manifest-path src-tauri/Cargo.toml -p omniget-world-tools --bin atlas-pack -- art/world/craft-v1-src static/world/tiles/craft-v1
cargo run --manifest-path src-tauri/Cargo.toml -p omniget-world-tools --bin atlas-check -- static/world/tiles/craft-v1/atlas.json
npx vitest run src/lib/world/assets.test.ts
```

Preparação determinística: textura quadrada → projeção no piso original; objetos → corte da margem transparente e redimensionamento proporcional dentro do retângulo original, alinhado ao chão. Alpha dos objetos preservado. O arquivo original nunca é sobrescrito.

## Verificação

- `atlas-check`: aprovado.
- `house-check`: composição e atlas aprovados, com rotinas reassociadas ao ID composto para o teste.
- Testes de assets: 16 aprovados, incluindo preservação de contratos e adição dos quatro postos sem colisão de IDs.
- Svelte check: zero erros; avisos existentes no checkout (89 na execução anterior aos ajustes de tradução).
- Prévia inspecionada no navegador: horta, pomar, galpão e oficina conectados por caminhos; câmera enquadra também os objetos altos. A leitura humana em três segundos e os budgets no hardware-piso ainda não foram medidos.

Durante a inspeção no navegador, o shell do app emitiu erros de listeners Tauri (`transformCallback`) porque não há backend desktop nesse ambiente. O renderer do quintal permaneceu funcional. Nenhuma validação da simulação nativa foi inferida desse preview.

## Revisão da fazendinha

`farm/` preserva seis novas gerações do Higgsfield (incluindo a bancada isométrica que substitui a frontal). `scripts/art/farm_scene.py` define a composição e os novos pivôs. O script principal reconstrói o mapa e prepara os sprites sem novas chamadas ao serviço. Os canteiros são cenografia nesta prévia; não representam uma colheita ou teste aprovado do Loop.

Os testes de assets verificam também que cada objeto da fazenda possui um sprite, permanece sobre terreno válido e não sobrepõe a colisão de outro objeto.
