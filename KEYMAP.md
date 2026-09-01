# RuanaStudio — Atalhos de Teclado (KEYMAP)

Referência dos atalhos atuais da DAW. Os atalhos **globais** vivem no
`FocusScope` da janela principal (`ui/main.slint`); gestos de mouse
(timeline, piano roll, Clip Pool) estão espalhados nos componentes.

> Convenção: `A+B` = segurar `A` e pressionar `B`. `→` = disparo
> independente do foco global (a timeline tem prioridade e consome).

---

## Edição

| Atalho | Ação |
| --- | --- |
| `Ctrl+C` | Copia o clip selecionado para o **clipboard interno** (não é o clipboard do SO). Sustenta colar múltiplas vezes com `Shift+clique`. |
| `Ctrl+D` | Duplica o clip selecionado (cópia imediatamente à frente, herdando cor/override e audio). |
| `Ctrl+Z` | Desfaz (undo). `Shift+Ctrl+Z`/`Ctrl+Y` refaz (redo). |
| `Delete` / `Backspace` | Apaga o clip selecionado; com multi-seleção (marquee), apaga todos os selecionados. |
| `Ctrl+Y` | Refaz (redo). |

## Timeline

| Ação | Descrição |
| --- | --- |
| `Shift + clique` (em área vazia) | **Cola** o conteúdo do clipboard na posição clicada (track + beat). |
| `Ctrl + clique-arraste` | **Marquee / lasso**: seleção múltipla. Funciona começando em cima de um clip ou em área vazia. |
| Clique em área vazia | Deseleciona a seleção única (qualquer ferramenta). |
| Arraste de clip (mouse) | Move o clip; arrastar pela borda redimensiona. |
| Slice/Glue/faca | Ferramentas que dividem (`/`) ou unem (`U`) clips adjacentes (via barra de ferramentas). |
| Dropar item do **Clip Pool** | Cria um clip na timeline. Padrões do Beat Sequencer caem com a **duração correspondente do padrão** (passos/4). |

## Navegação

| Atalho | Ação |
| --- | --- |
| `Espaço` | Toca / pausa o transporte (play/stop). |
| `F` | Centraliza e faz **zoom no clip selecionado**; `F` de novo restaura o viewport ao contexto geral. |
| Rodinha do mouse | Zoom horizontal e rolagem vertical da timeline. |

## Beat Sequencer

| Atalho / Ação | Descrição |
| --- | --- |
| Clique em um passo do grid | Liga/desliga a nota (`Ctrl+clique` para estender/caret). |
| Clique num padrão na seção **BEAT PATTERNS** do Clip Pool | Seleciona o padrão no sequencer (song mode). |
| Arraste um padrão do Clip Pool para a timeline | Cria um clip MIDI regular com a duração do padrão. |
| **Botões do popup**: `Copy` / `Duplicate` / `Clear` / `Delete` | Copiam/duplicam/limpam/apagam padrões, trilhas ou linhas. |

## Referência rápida

| Tecla | Um toque |
| --- | --- |
| `Ctrl+C` | Copiar clip |
| `Ctrl+D` | Duplicar clip |
| `Ctrl+Z` / `Ctrl+Y` / `Ctrl+Shift+Z` | Undo / Redo |
| `Delete` / `Backspace` | Apagar clip(s) |
| `Space` | Play/Stop |
| `F` | Focar/zoom no clip selecionado |

---

*Mantido em sincronia com o código: `ui/main.slint` (handler global de
teclas), `ui/timeline/timeline.slint` (clique fora, Shift+clique,
marquee), `ui/clip_pool.slint` (seção BEAT PATTERNS).*
