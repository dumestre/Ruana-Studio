# Build, Testes e Verificação

> Como compilar, testar e verificar o Ruana Studio. Comandos reais, contagem
> real de testes e os pitfalls de build que já custaram tempo.

**Última verificação:** 2026-10-13 · **Fontes:** `Cargo.toml`, `app/Cargo.toml`,
`app/build.rs`, execução real de `cargo check`

---

## 1. Workspace

```toml
[workspace]
resolver = "3"
members = [
    "app",
    "crates/daw-core",
    "crates/daw-audio",
    "crates/daw-media",
    "crates/daw-midi",
    "crates/daw-plugins",
    "crates/daw-project",
    "timeline-render",
    "ruana-hub",
]

[workspace.package]
version = "0.1.0"
edition = "2024"
```

---

## 2. Comandos essenciais

```bash
# Build do workspace
cargo build

# Crate específico
cargo build -p daw-audio
cargo build -p timeline-render

# Check rápido (sem codegen)
cargo check -p daw-core -p daw-midi

# Testes
cargo test -p daw-core
cargo test -p daw-audio
cargo test

# Formatação (gate de revisão)
cargo fmt --check
```

---

## 3. Perfil de build

```toml
[profile.dev]
opt-level = 0                  # nosso código: compila rápido, debugável
debug = "line-tables-only"
incremental = true

[profile.dev.package."*"]
opt-level = 2                  # dependências: rápidas em runtime

[profile.dev.package.daw-audio]
opt-level = 2

[profile.dev.package.daw-media]
opt-level = 2

[profile.release]
lto = "thin"
codegen-units = 16
debug = false
```

**Por que:** o nosso código em `opt-level = 0` (compilação incremental rápida,
debug limpo), mas as dependências em `2` — sem isso, DSP e decodificação em
debug ficam lentos demais para testar áudio de verdade.

---

## 4. Testes — contagem real

**228 anotações `#[test]`** em `crates/` + `app/src/`. Distribuição por arquivo:

| Testes | Arquivo |
|---|---|
| 47 | `crates/daw-core/src/project.rs` |
| 19 | `crates/daw-audio/src/stretch.rs` |
| 16 | `app/src/meters.rs` |
| 15 | `crates/daw-media/src/waveform_image.rs` |
| 14 | `crates/daw-core/src/timeline.rs` |
| 13 | `app/src/beat.rs` |
| 11 | `crates/daw-midi/src/pattern.rs` |
| 11 | `crates/daw-midi/src/notes.rs` |
| 11 | `crates/daw-audio/src/mixer.rs` |
| 11 | `crates/daw-audio/src/engine.rs` |
| 9 | `crates/daw-core/src/warp.rs` |
| 8 | `crates/daw-audio/src/resample.rs` |
| 7 | `crates/daw-media/src/waveform.rs` |
| 6 | `crates/daw-core/src/transport.rs` |
| 6 | `crates/daw-audio/src/meter.rs` |
| 6 | `crates/daw-audio/src/buffer.rs` |
| 5 | `crates/daw-core/src/mixer.rs` |
| 5 | `crates/daw-core/src/clip.rs` |
| 4 | `app/src/models.rs` |
| 2 | `crates/daw-core/src/track.rs` |
| 2 | `app/src/timeline_slint.rs` |

Onde está a cobertura mais densa: **modelo de documento** (`project.rs`),
**time-stretch** (`stretch.rs`), **medidores** (`meters.rs`) e **waveform**
(`waveform_image.rs`). Ver [[MIDI e Sequenciador]], [[Timeline e Transporte]],
[[Projeto e Persistência]] para o que cada suíte cobre.

---

## 5. build.rs do app (faz muito mais que compilar UI)

`app/build.rs` roda **três** tarefas:

### 5.1 Compila a UI Slint

```rust
let ui_file = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../ui/main.slint");
slint_build::compile(ui_file).expect("failed to compile Slint UI");
```

O arquivo compilado é `ui/main.slint` — **não** `app/ui/...`. A pasta `ui/` fica
na **raiz do repositório**.

### 5.2 Gera os cursores `.cur` a partir de SVG

```rust
let cursors = [
    ("selectcursor.svg",  "select.cur",  (7, 8)),
    ("drawcursor.svg",    "draw.cur",    (16, 16)),
    ("erasecursor.svg",   "erase.cur",   (16, 16)),
    ("gluecursor.svg",    "glue.cur",    (16, 16)),
    ("slicecursor.svg",   "slice.cur",   (16, 16)),
    ("stretchcursor.svg", "stretch.cur", (16, 16)),
    ("gain.svg",          "gain.cur",    (16, 16)),
    ("hand.svg",          "hand.cur",    (16, 16)),
];
```

Para **cada** SVG o build gera **dois** arquivos: o claro (`X.cur`) e o escuro
(`X-dark.cur`, com as cores invertidas antes do premultiply do alpha).

**Pontos que valem saber:**

- `app/cursors/` é **gerado**, não versionado à mão. A fonte é
  `app/cursors/svgcursors/`.
- Os SVGs são rasterizados em **32×32** com escala **0.55** (a arte ocupa 55%
  do quadro).
- O **hotspot** é por cursor: `select` usa `(7, 8)` (a ponta da seta), os
  outros `(16, 16)` (o centro).
- `gain.cur` e `hand.cur` **não** estão no `ToolCursor` — existem no build mas
  não têm ferramenta mapeada em `app/src/cursors.rs`.
- Depois de gerar, `copy_cursors_to_target()` copia tudo para
  `target/<profile>/cursors/` — é o diretório que `cursor_directory()` procura
  em runtime (ver [[Cursores e Ferramentas]] §4).

O formato `.cur` é escrito **à mão** (`write_cur_file`): ICONDIR (type = 2),
BITMAPINFOHEADER com altura dobrada (XOR + AND mask), pixels BGRA
**bottom-up** com alpha **premultiplicado**, e máscara AND de 1 bit/pixel com
stride de 4 bytes.

### 5.3 Gera o ícone do app

Renderiza `assets/RuanaStudio.svg` em PNGs (16/24/32/48/64/128/256) e monta um
`.ico` multi-size com entradas BMP clássicas (não PNG embutido — o comentário
explica: *"formato aceito por todo o shell do Windows, sem depender de
decodificação de PNG embutido"*).

**Override do usuário tem prioridade:**

| Arquivo | Efeito |
|---|---|
| `assets/ruana-studio.ico` | embutido no `.exe` |
| `assets/app_icon_64.png` | ícone de runtime das janelas |

O `.ico` só é gerado/embutido no Windows, e a falha ao embutir é **graciosa**
(`cargo:warning=Icone nao embutido no .exe (…)`) — o ícone de runtime continua
funcionando se `rc.exe`/`windres` não existirem.

---

## 6. Pitfalls de build

### 6.1 Erro de sintaxe `.slint` aparece como **panic no build script**

Um erro de sintaxe num `.slint` não falha o crate de forma limpa: **panica
dentro do `build.rs`**, e a linha reportada **frequentemente NÃO é a linha real
do erro**.

Procedimento que funciona: **contar chaves antes de investigar.**

```bash
grep -o '{' ui/pasta/arquivo.slint | wc -l
grep -o '}' ui/pasta/arquivo.slint | wc -l
```

Os números têm de bater. Um `{` sobrando fecha o componente cedo e o parser
reclama dezenas de linhas depois.

### 6.2 O caminho da UI é relativo à raiz

`build.rs` monta `CARGO_MANIFEST_DIR/../ui/main.slint`. Mover a pasta `ui/`
quebra o build mesmo que os arquivos existam.

### 6.3 Cursores ausentes não quebram o build

`copy_cursors_to_target()` usa `.ok()` e ignora o erro. Se `app/cursors/` não
existir, o build passa e o app roda **sem cursores customizados** (com logs
`[cursor]` no stderr).

### 6.4 Aviso de hard link (F:)

```
warning: hard linking files in the incremental compilation cache failed.
consider moving the cache directory to a file system which supports hard linking
```

Acontece quando o `target/` está num filesystem sem suporte a hard link. É
**ruído de performance**, não erro — a compilação faz cópia em vez de link.

---

## 7. Verificação antes de entregar uma alteração

Checklist derivado do processo de revisão do projeto:

1. `cargo fmt --check` — formatação.
2. `cargo check` — compila sem codegen.
3. `cargo test` (ou `-p <crate>` afetado) — suíte.
4. Build do alvo afetado: `cargo build -p daw-audio` /
   `-p timeline-render` / `cargo build` (app).
5. Se mexeu em **geometria de timeline ou shader**: aplicar em
   `timeline-render/src/main.rs` **e** `app/src/timeline_canvas.rs` (ver
   [[wgpu Timeline Rendering]] §"Sincronização Standalone ↔ App").
6. Se mexeu em **áudio**: teste de escuta, não só `cargo test`.
7. Se mudou **arquitetura, contratos ou procedimento**: atualizar o cofre.

---

## 8. Estado verificado em 2026-10-13

| Verificação | Resultado |
|---|---|
| `cargo check -p daw-core -p daw-midi` | **OK** — `Finished dev profile in 6.44s` |
| Warnings de `daw-core` / `daw-midi` | 1 warning cada (não bloqueante) |
| Chaves em `ui/mixer/mixer_channel.slint` | 45 `{` / 45 `}` — **balanceado** |
| Testes declarados no workspace | 228 |

> Nota histórica: houve um período em que `ui/mixer/mixer_channel.slint` estava
> desbalanceado (51 `{` vs 52 `}`) e quebrava o `cargo build` do app. **Isso já
> foi corrigido** — o arquivo está balanceado hoje. O pitfall §6.1 continua
> válido como técnica de diagnóstico.

---

## Links

- [[Workspace Rust e Crates]]
- [[Cursores e Ferramentas]]
- [[wgpu Timeline Rendering]]
- [[Estado Atual e Roadmap]]
