# Ferrum Studio  Windows > Linux > MacOS


DAW (Digital Audio Workstation) nativa desenvolvida em **Rust**, com **Slint** para a interface gráfica.

## ⚠️ REGRA PRINCIPAL DE DESENVOLVIMENTO

### A DAW será construída primeiro pela interface.

O desenvolvimento inicial deve ser focado **exclusivamente na UI/UX e no design visual da DAW**.

A ordem intencional do projeto é:

```text
FASE 1
UI / UX
   ↓
FASE 2
Integração com o modelo de dados
   ↓
FASE 3
Audio Engine
   ↓
FASE 4
MIDI / Piano Roll
   ↓
FASE 5
Plugin Host
   ↓
FASE 6
Recursos avançados
```

### Durante a Fase 1

O agente deve **priorizar o design da interface**, incluindo:

- layout geral
- Arrange View
- Timeline
- Track Headers
- Mixer
- Piano Roll
- Browser
- Inspector
- Transport
- Faders
- Knobs
- Buttons
- Meters
- Menus
- Toolbars
- Panels
- Splitters
- seleção
- drag & drop visual
- zoom
- scroll
- estados visuais
- animações necessárias
- feedback visual
- responsividade dos componentes

A interface deve ser construída com **dados mockados** quando necessário.

Exemplo:

```text
Track 1 → Drums
Track 2 → Bass
Track 3 → Vocal
```

Os clips podem ser objetos fictícios:

```text
Drums
████████████

Bass
      ███████████

Vocal
             █████████
```

O Piano Roll pode utilizar notas fictícias:

```text
C5       █████
B4           ████
A4      █████████
G4                █████
```

O Mixer pode utilizar valores fictícios de volume, pan e meters.

---

## O que NÃO fazer durante a Fase 1

Não implementar prematuramente:

- processamento real de áudio
- gravação de áudio
- reprodução real
- drivers de áudio
- VST3
- CLAP
- MIDI real
- comunicação com dispositivos MIDI
- persistência definitiva do projeto
- decoder de áudio
- Audio Graph
- processamento DSP
- otimizações de real-time audio

Esses sistemas serão implementados posteriormente.

### Exceção

Pode existir **lógica mínima necessária para a própria interação da UI**.

Exemplos:

- arrastar um clip visualmente
- redimensionar um clip
- selecionar uma track
- alterar zoom
- mover a timeline
- abrir/fechar painel
- alterar um fader visualmente
- selecionar notas no Piano Roll
- alterar valores exibidos

Essa lógica deve existir apenas para tornar o protótipo da interface funcional.

---

# Objetivo da Fase 1

O objetivo é conseguir abrir a aplicação e visualizar uma **DAW visualmente completa**, mesmo que os dados ainda sejam fictícios.

Exemplo:

```text
┌──────────────────────────────────────────────────────────────┐
│ TRANSPORT                                                    │
├──────────────┬───────────────────────────────────────────────┤
│ TRACKS       │ ARRANGE / TIMELINE                            │
│              │                                               │
│ Drums        │ ███████████                                   │
│ Bass         │       ███████████                             │
│ Vocal        │              █████████████                    │
│ Synth        │                    █████████                  │
│              │                                               │
├──────────────┴───────────────────────────────────────────────┤
│ MIXER                                                        │
│                                                              │
│ Drums      Bass       Vocal       Synth       Master         │
│   │          │          │           │           │            │
│   │          │          │           │           │            │
│   ▼          ▼          ▼           ▼           ▼            │
└──────────────────────────────────────────────────────────────┘
```

O visual deve ser tratado como um **protótipo real da DAW**, não como uma tela descartável.

---

# Arquitetura futura

Mesmo começando pela UI, a arquitetura final deverá separar:

```text
                    ┌──────────────────┐
                    │     Slint UI     │
                    └────────┬─────────┘
                             │
                       Commands / Events
                             │
                             ▼
                    ┌──────────────────┐
                    │    DAW Core      │
                    └────────┬─────────┘
                             │
                    ┌────────┴─────────┐
                    ▼                  ▼
             ┌──────────────┐   ┌──────────────┐
             │ Audio Engine │   │ Plugin Host  │
             └──────────────┘   └──────────────┘
```

Porém, **essa arquitetura será implementada progressivamente**.

Não antecipar a implementação do backend apenas porque a arquitetura futura já está definida.

---

# Regra para o agente

Quando uma tarefa envolver a interface durante a Fase 1:

> **Construa primeiro a experiência visual. Não substitua o trabalho de UI por implementação de backend.**

Se uma funcionalidade normalmente dependeria de áudio, MIDI ou plugins, use dados mockados para representar o comportamento visual.

Exemplo:

```text
Pedido:
"Adicionar medidor de volume"

Fase 1:
→ criar o meter visual
→ usar valores simulados
→ criar animação visual
→ testar aparência e comportamento

Não fazer:
→ implementar processamento real de áudio
→ conectar ao dispositivo de áudio
```

Outro exemplo:

```text
Pedido:
"Criar Piano Roll"

Fase 1:
→ desenhar teclado
→ grid
→ notas
→ seleção
→ drag
→ resize
→ velocity
→ zoom
→ scroll

Não fazer:
→ MIDI real
→ MIDI input
→ MIDI output
```

---

# Critério para sair da Fase 1

A Fase 1 termina quando a interface principal da DAW estiver visualmente definida e funcional como protótipo.

Somente então começar a conectar:

```text
UI
 ↓
DAW Core
 ↓
Audio Engine
 ↓
MIDI
 ↓
Plugin Host
```

A arquitetura deve permitir que a UI desenvolvida na Fase 1 seja mantida enquanto a lógica real é gradualmente conectada.


# RustDAW

DAW (Digital Audio Workstation) nativa desenvolvida em **Rust**, com **Slint** para a interface gráfica.

O objetivo é construir uma DAW modular, performática e extensível, com arquitetura semelhante às DAWs tradicionais, mas com um núcleo moderno e independente da interface.

---

## 1. Princípios do projeto

### 1.1 Separação rígida entre UI, Core e Audio Engine

A arquitetura deve seguir:

```text
                    ┌──────────────────┐
                    │     Slint UI     │
                    └────────┬─────────┘
                             │
                       Commands / Events
                             │
                             ▼
                    ┌──────────────────┐
                    │    DAW Core      │
                    │                  │
                    │ Project Model    │
                    │ Commands         │
                    │ Undo / Redo      │
                    │ Timeline         │
                    │ MIDI             │
                    │ Automation       │
                    └────────┬─────────┘
                             │
                    ┌────────┴─────────┐
                    ▼                  ▼
             ┌──────────────┐   ┌──────────────┐
             │ Audio Engine │   │ Plugin Host  │
             └──────┬───────┘   └──────────────┘
                    │
                    ▼
              Audio Device
```

**Regras obrigatórias:**

- O Core não pode depender de Slint.
- O Audio Engine não pode depender de Slint.
- O Plugin Host não pode depender de Slint.
- A UI não deve implementar regras de negócio.
- A UI deve enviar comandos e receber estado.
- O callback de áudio nunca deve depender da UI.
- O thread de áudio deve ser real-time safe.
- Não fazer I/O de disco dentro do callback de áudio.
- Evitar alocações, locks pesados e operações imprevisíveis no callback de áudio.

---

# 2. Workspace

O projeto deve ser organizado como um Cargo Workspace.

```text
rust-daw/
│
├── Cargo.toml
│
├── crates/
│   │
│   ├── daw-core/
│   ├── daw-audio/
│   ├── daw-media/
│   ├── daw-midi/
│   ├── daw-plugins/
│   ├── daw-project/
│   └── daw-ui/
│
├── app/
│   └── src/
│       └── main.rs
│
├── ui/
│   ├── main.slint
│   ├── arrange/
│   ├── piano_roll/
│   ├── mixer/
│   ├── browser/
│   └── components/
│
└── README.md
```

A estrutura pode evoluir, mas o agente deve preservar a separação de responsabilidades.

Não criar um `main.rs` monolítico.

Não colocar toda a lógica dentro dos arquivos `.slint`.

---

# 3. DAW Core

O `daw-core` é o núcleo independente da interface.

Ele contém o modelo conceitual da DAW.

```text
daw-core/
├── project/
├── track/
├── clip/
├── timeline/
├── tempo/
├── automation/
├── commands/
├── selection/
└── history/
```

---

# 4. Project Model

O projeto representa tudo que deve ser salvo em um projeto da DAW.

```text
Project
├── metadata
├── tempo_map
├── time_signature
├── tracks
├── buses
├── markers
├── routing
└── settings
```

O projeto não deve conhecer:

- Slint
- widgets
- janelas
- pixels
- eventos de mouse
- teclado da UI

---

# 5. Tracks

Uma Track representa um canal da DAW.

Uma track pode conter:

```text
Track
├── identity
├── type
├── clips
├── volume
├── pan
├── mute
├── solo
├── inserts
├── sends
├── routing
└── automation
```

Tipos possíveis:

```text
Audio
MIDI
Instrument
Bus
Aux
Master
```

A implementação pode começar apenas com Audio e MIDI.

---

# 6. Clips

Um clip representa um objeto colocado na timeline.

Existem principalmente:

```text
AudioClip
MidiClip
```

### AudioClip

```text
AudioClip
├── source
├── timeline_position
├── duration
├── source_offset
├── gain
├── fade_in
├── fade_out
└── playback_mode
```

### MidiClip

```text
MidiClip
├── timeline_position
├── duration
└── events
```

---

# 7. MIDI

MIDI e Piano Roll não são a mesma coisa.

MIDI é o sistema de eventos.

O Piano Roll é um editor visual desses eventos.

```text
MIDI
├── Note
├── Control Change
├── Pitch Bend
├── Aftertouch
├── Program Change
└── outros eventos
```

Uma nota deve possuir, no mínimo:

```text
Note
├── pitch
├── velocity
├── start
└── duration
```

O Piano Roll deve apenas editar e visualizar esses dados.

---

# 8. Timeline

A DAW deve possuir um sistema centralizado de tempo.

Não usar segundos como única representação de posição.

A timeline precisa permitir conversões entre:

```text
Bars / Beats
       ↕
Musical Time
       ↕
Seconds
       ↕
Samples
```

O sistema deve possuir:

```text
Position
Duration
Tempo
TempoMap
TimeSignature
```

Isso será utilizado por:

- MIDI
- Audio
- Automation
- Quantization
- Loop
- Recording
- Playback
- Grid
- Snap

O tempo musical deve continuar correto mesmo quando o BPM mudar.

---

# 9. Transport

O Transport controla o estado global de reprodução.

```text
Transport
├── play
├── stop
├── pause
├── record
├── position
├── loop
├── bpm
├── time_signature
└── playback_state
```

Estados:

```text
Stopped
Playing
Paused
Recording
```

---

# 10. Commands

Alterações no projeto devem ocorrer através de Commands.

Exemplos:

```text
CreateTrack
DeleteTrack
MoveTrack
RenameTrack

CreateAudioClip
DeleteAudioClip
MoveAudioClip
SplitAudioClip

CreateMidiClip
AddMidiNote
DeleteMidiNote
MoveMidiNote
ResizeMidiNote

SetTrackVolume
SetTrackPan
SetMute
SetSolo

AddPlugin
RemovePlugin

AddAutomation
ModifyAutomation
```

A UI não deve modificar diretamente estruturas internas do projeto quando a alteração precisar ser persistida ou desfeita.

Fluxo:

```text
UI
 ↓
Command
 ↓
DAW Core
 ↓
Project
 ↓
History
```

---

# 11. Undo / Redo

Todo Command que modifica o projeto deve ser capaz de ser desfeito.

```text
Command
├── execute()
└── undo()
```

Histórico:

```text
Command
Command
Command
Command
   ↑
 current
```

Implementar:

```text
Undo
Redo
```

desde cedo.

Não deixar Undo/Redo para o final do projeto.

---

# 12. Audio Engine

O Audio Engine é independente da UI.

Responsabilidades:

```text
Audio Engine
├── Audio Device
├── Audio Buffer
├── Transport synchronization
├── Audio Graph
├── Track processing
├── Mixer
├── Bus processing
├── Sends
├── Master
└── Plugin processing
```

Fluxo:

```text
Audio Clip
     ↓
Clip Player
     ↓
Track Inserts
     ↓
Track Volume / Pan
     ↓
Bus
     ↓
Master
     ↓
Output Device
```

---

# 13. Real-Time Audio

O callback de áudio é uma área crítica.

Dentro do callback:

### Evitar

```text
Heap allocation
File I/O
Logging pesado
Mutex bloqueante
Network
UI operations
Slint operations
Operações imprevisíveis
```

O callback deve trabalhar com dados previamente preparados.

Comunicação entre UI/Core e Audio Engine deve utilizar mecanismos apropriados para real-time audio, como:

```text
lock-free queues
atomics
preallocated buffers
```

quando necessário.

A estabilidade do áudio tem prioridade sobre atualizações visuais.

---

# 14. Audio Graph

O processamento deve ser baseado em um grafo de áudio.

Exemplo:

```text
Track 1 ────────┐
                │
Track 2 ────────┼──→ Bus ──→ Master ──→ Output
                │
Track 3 ────────┘
```

Uma track pode possuir plugins:

```text
Audio
 ↓
EQ
 ↓
Compressor
 ↓
Saturation
 ↓
Volume
 ↓
Bus
```

O sistema deve permitir futuramente:

- Inserts
- Sends
- Returns
- Buses
- Sidechain
- Parallel processing
- Master chain

---

# 15. Plugin Host

O sistema de plugins deve ser independente da UI.

Objetivos:

```text
VST3
CLAP
```

Arquitetura:

```text
Plugin Host
├── Scanner
├── Plugin Metadata
├── Plugin Instance
├── Parameters
├── State
├── Audio Processing
└── Editor
```

O host deve conseguir processar plugins mesmo que a UI do plugin não esteja aberta.

---

# 16. Media System

Responsável pelos arquivos de mídia.

```text
daw-media/
├── decoder
├── audio_file
├── waveform
├── cache
└── sample_manager
```

Responsabilidades:

- carregar arquivos de áudio
- decodificar áudio
- gerar waveform
- cache de waveform
- gerenciar referências aos arquivos
- detectar arquivos ausentes

O áudio usado durante o processamento deve ser preparado fora do callback real-time.

---

# 17. Project Format

O projeto deve possuir formato próprio.

Exemplo:

```text
MySong/
├── project.daw
├── Audio/
├── Waveforms/
└── Cache/
```

`project.daw` deve armazenar o estado do projeto.

O projeto deve referenciar arquivos de áudio, em vez de necessariamente copiar todos os arquivos para dentro do projeto.

Futuramente pode existir:

```text
Save
Save As
Autosave
Backup
Collect and Save
```

---

# 18. UI / Slint

Slint é responsável exclusivamente pela apresentação e interação.

Principais áreas:

```text
MainWindow
├── Transport
├── Browser
├── ArrangeView
├── Mixer
├── Inspector
└── StatusBar
```

Editor:

```text
ArrangeView
├── Timeline Ruler
├── Track Headers
├── Audio Clips
├── MIDI Clips
├── Automation
└── Playhead
```

Piano Roll:

```text
PianoRoll
├── Piano Keyboard
├── Grid
├── Notes
├── Velocity
├── Controllers
└── Playhead
```

Mixer:

```text
Mixer
├── Channel Strips
├── Inserts
├── Sends
├── Fader
├── Pan
├── Meter
├── Mute
└── Solo
```

---

# 19. UI State

Diferenciar:

```text
Project State
```

de:

```text
UI State
```

Project State:

```text
Tracks
Clips
MIDI
Automation
Mixer
Routing
```

UI State:

```text
Selected Track
Selected Clip
Zoom
Scroll
Open Panel
Focused Editor
Browser State
```

UI State não deve ser salvo no projeto, exceto quando houver uma razão explícita para isso.

---

# 20. Comunicação

A comunicação deve seguir:

```text
Slint
  │
  │ Command
  ▼
Application
  │
  ▼
Core
  │
  ├──── Project
  │
  └──── Audio Engine
```

Eventos do Core podem retornar para a UI:

```text
ProjectChanged
TransportChanged
SelectionChanged
TrackChanged
PluginChanged
AudioMeterUpdated
```

A UI não deve acessar diretamente estruturas internas do Audio Engine.

---

# 21. Performance

Prioridades:

```text
1. Audio stability
2. Correctness
3. Deterministic behavior
4. Low latency
5. UI responsiveness
6. Visual effects
```

Não sacrificar estabilidade do áudio para atualizar a UI a cada sample.

Meters e visualizações podem usar uma taxa menor, por exemplo:

```text
30–60 FPS
```

enquanto o áudio roda em sua própria taxa de processamento.

---

# 22. Desenvolvimento incremental

Não implementar tudo de uma vez.

Ordem recomendada:

### Fase 1 — Core

```text
Project
Track
Clip
Timeline
Transport
Commands
Undo/Redo
```

### Fase 2 — Audio

```text
Audio Device
Audio Buffer
Audio File
Track Playback
Master Output
```

### Fase 3 — UI

```text
Main Window
Transport
Track List
Timeline
Audio Clips
Playhead
```

### Fase 4 — MIDI

```text
MIDI Events
MIDI Clip
Piano Roll
MIDI Recording
MIDI Playback
```

### Fase 5 — Mixer

```text
Volume
Pan
Mute
Solo
Meters
Bus
Sends
```

### Fase 6 — Plugins

```text
Plugin Scanner
VST3
CLAP
Plugin Parameters
Plugin State
Plugin Editor
```

### Fase 7 — Automation

```text
Automation Lane
Automation Points
Read
Write
Touch
Latch
```

### Fase 8 — Produção

```text
Recording
Comping
Fades
Crossfades
Time Stretch
Freeze
Bounce
Export
```

---

# 23. Regras para o agente

Antes de implementar qualquer funcionalidade:

1. Verificar em qual camada ela pertence.
2. Não colocar lógica de negócio na UI.
3. Não criar dependências circulares entre crates.
4. Não duplicar estado entre UI e Core sem necessidade.
5. Não colocar processamento de áudio no thread da UI.
6. Não bloquear o thread de áudio.
7. Preferir tipos fortes em vez de `f32`/`String` espalhados pelo sistema.
8. Criar testes para lógica do Core.
9. Manter APIs pequenas e explícitas.
10. Não adicionar abstrações complexas sem necessidade.
11. Não implementar funcionalidades futuras prematuramente.
12. Não quebrar a arquitetura existente para resolver rapidamente um problema local.
13. Antes de criar um novo módulo, verificar se a responsabilidade já existe.
14. Manter o projeto compilando após cada etapa significativa.

---

# 24. Filosofia

A DAW deve ser construída como um **motor de áudio com uma interface**, e não como uma interface que toca áudio.

A arquitetura deve permitir trocar a UI futuramente sem reescrever o Core.

Também deve ser possível testar:

```text
daw-core
```

sem iniciar Slint ou dispositivo de áudio.

E testar:

```text
audio-engine
```

sem iniciar a interface gráfica.

O objetivo é ter uma base sólida antes de adicionar funcionalidades.

---

# 25. Estado inicial desejado

A primeira versão funcional deve conseguir:

```text
Abrir DAW
   ↓
Criar Track
   ↓
Importar WAV
   ↓
Colocar AudioClip na Timeline
   ↓
Pressionar Play
   ↓
Ouvir áudio
   ↓
Mover Clip
   ↓
Salvar projeto
   ↓
Reabrir projeto
```

Depois:

```text
Criar MIDI Track
   ↓
Criar MIDI Clip
   ↓
Abrir Piano Roll
   ↓
Criar notas
   ↓
Reproduzir MIDI
```

Depois:

```text
Track
 ↓
Plugin
 ↓
Mixer
 ↓
Master
 ↓
Output
```

Esse é o núcleo da DAW.

**Não começar pelo visual. Começar pelo modelo de dados, timeline e engine.**