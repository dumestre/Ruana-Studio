# MOC — Mapa do Conhecimento

> Índice mestre do cofre RuanaStudio. Organizado por assunto.

**Caminho real do cofre:**
`F:\Projetos Dev Temp\RuanaStudio\ruana-studio-docs\Ruana Brain`

> ⚠️ O `AGENTS.md` do orquestrador aponta para
> `F:\Projetos Dev Temp\ruana-studio\ruana-studio-docs\ruana-brain` — esse
> caminho **não existe**. O correto é o de cima (note `RuanaStudio`, com
> `Ruana Brain` separado por espaço).

---

## Navegação rápida

### Arquitetura e visão geral
- [[Arquitetura Geral]] — visão de cima do sistema
- [[Workspace Rust e Crates]] — estrutura do Cargo workspace
- [[Decisões Arquiteturais]] — ADRs (o **porquê** de cada escolha)
- [[Estado Atual e Roadmap]] — implementado × placeholder × pendências

### Interface (Slint)
- [[Referência Slint 1.18]] — DSL + API Rust + backend Skia
- [[Patterns de UI Slint para DAW]] — padrões de interação
- [[Sistema de Tema]] — 7 paletas, escada de superfícies, troca em runtime
- [[Componentes e Tokens]] — catálogo dos componentes e convenções
- [[Arquitetura de Janelas e Painéis]] — Workspace, splitters, janelas flutuantes
- [[Fluxo de Interface]] — winit → roteamento → Slint/engine → core
- [[Cursores e Ferramentas]] — cursores Win32, variante clara/escura

### Renderer
- [[Referência wgpu 30]] — renderer GPU + shaders WGSL
- [[wgpu Timeline Rendering]] — renderização de timeline

### Áudio e MIDI
- [[Engine de Áudio]] — pipeline completo: stream, workers, playback, timewarp
- [[Referência cpal 0.18]] — I/O de áudio de baixo nível
- [[Referência symphonia 0.6]] — decodificação de áudio
- [[Referência timestretch 0.15]] — time-stretching EDM
- [[MIDI e Sequenciador]] — notas, piano roll, beat patterns

### Timeline e documento
- [[Timeline e Transporte]] — mapa de tempo, timewarp, playhead
- [[Projeto e Persistência]] — Project, Clip, Track, Mixer, undo/redo

### Infra
- [[Referência Rust]] — Rust 2024 edition (base do projeto)
- [[Referência winit 0.30]] — janelas e eventos
- [[Build, Testes e Verificação]] — comandos, 228 testes, pitfalls de build
- [[Plugins e Extensibilidade]] — VST3/CLAP (fase futura)
- [[KEYMAP]] — atalhos de teclado

---

## Índice por pasta

| Pasta | Conteúdo |
|---|---|
| `00 - Índice e Arquitetura` | [[MOC - Mapa do Conhecimento]], [[Arquitetura Geral]], [[Workspace Rust e Crates]] |
| `01 - Rust e Workspace` | [[Referência Rust]], [[Workspace Rust e Crates]], [[Referência winit 0.30]] |
| `02 - Slint e Interface` | [[Referência Slint 1.18]], [[Patterns de UI Slint para DAW]], [[Sistema de Tema]], [[Componentes e Tokens]], [[Arquitetura de Janelas e Painéis]], [[Fluxo de Interface]], [[Cursores e Ferramentas]] |
| `03 - wgpu 30 e Renderer` | [[Referência wgpu 30]], [[wgpu Timeline Rendering]] |
| `04 - Engine de Áudio` | [[Engine de Áudio]], [[Referência cpal 0.18]], [[Referência symphonia 0.6]], [[Referência timestretch 0.15]] |
| `05 - MIDI e Sequenciador` | [[MIDI e Sequenciador]] |
| `06 - Timeline e Transporte` | [[Timeline e Transporte]] |
| `07 - Projeto e Persistência` | [[Projeto e Persistência]] |
| `08 - Plugins e Extensibilidade` | [[Plugins e Extensibilidade]] |
| `09 - Testes e Qualidade` | [[Build, Testes e Verificação]] |
| `10 - Decisões Arquiteturais` | [[Decisões Arquiteturais]] |
| `11 - Roadmap e Tarefas` | [[Estado Atual e Roadmap]] |
| `12 - Histórico de Mudanças` | *(a preencher)* |

---

## Por onde começar

**Se você vai mexer na interface:** [[Sistema de Tema]] →
[[Componentes e Tokens]] → [[Fluxo de Interface]].

**Se você vai mexer em áudio:** [[Engine de Áudio]] →
[[Timeline e Transporte]] → [[Build, Testes e Verificação]].

**Se você vai mexer no renderer:** [[wgpu Timeline Rendering]] →
[[Decisões Arquiteturais]] (ADR-001, ADR-002).

**Antes de entregar qualquer alteração:** [[Build, Testes e Verificação]] §7.

---

## Convenções deste cofre

- Cada documento técnico indica: objetivo, arquivos/crates relacionados, APIs
  importantes, fluxo de dados, dependências, limitações, estratégia de testes,
  data da última verificação e links internos.
- **Fatos confirmados** são separados de **hipóteses** e **trabalho futuro**.
- Nada é registrado como implementado se existir apenas no roadmap.
- Os documentos citam `caminho/arquivo.rs:linha` sempre que possível.
