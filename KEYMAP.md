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
| `Ctrl+M` | **Alterna Musical/Linear** do clip selecionado (timewarp). Em *Musical* o áudio acompanha o tempo do projeto — esticado com pitch preservado (o motor toca em varispeed enquanto o render com keylock não chega e depois troca para o buffer pronto). Em *Linear* o áudio volta ao tempo absoluto e o span do clip decide quanto do arquivo é ouvido. Clips de áudio importados nascem **Musical**. Um `Ctrl+Z` desfaz. |
| `Ctrl+Z` | Desfaz (undo). `Shift+Ctrl+Z`/`Ctrl+Y` refaz (redo). |
| `Delete` / `Backspace` | Apaga o clip selecionado; com multi-seleção (marquee), apaga todos os selecionados. |
| `Ctrl+Y` | Refaz (redo). |

## Timeline

| Ação | Descrição |
| --- | --- |
| `Shift + clique` (em área vazia) | **Cola** o conteúdo do clipboard na posição clicada (track + beat). |
| `Ctrl + clique-arraste` | **Marquee / lasso**: seleção múltipla. Funciona começando em cima de um clip ou em área vazia. |
| `Alt + arraste` (na régua) | **Time Warp** (estilo Cubase): agarra a linha de compasso/batida mais próxima e a move para onde você soltar. O projeto ganha a rampa de tempo que faz aquela posição musical soar no ponto escolhido — os clips em modo musical são reesticados e continuam casando com a grade. O alvo não é arredondado (o gesto serve para encaixar a grade num evento do áudio). Um gesto = um `Ctrl+Z`. |
| Clique em área vazia | Deseleciona a seleção única (qualquer ferramenta). |
| Arraste de clip (mouse) | Move o clip; arrastar pela borda redimensiona. |
| Slice/Glue/faca | Ferramentas que dividem (`/`) ou unem (`U`) clips adjacentes (via barra de ferramentas). |
| Dropar item do **Clip Pool** | Cria um clip na timeline. Padrões do Beat Sequencer caem com a **duração correspondente do padrão** (passos/4). |

## Navegação

| Atalho | Ação |
| --- | --- |
| `Espaço` | Toca / pausa, **roteado pelo cursor**: sobre a popup do **Beat Sequencer** aberta comanda o sequenciador (o pattern soa e a timeline fica parada — nenhum clip toca, a agulha não anda); em qualquer outro lugar comanda a **timeline** (os clips tocam e a agulha anda, com o step sequencer mudo). Os dois nunca tocam juntos; pedir play no outro modo troca o que está em cena. |
| `F` | Centraliza e faz **zoom no clip selecionado**; `F` de novo restaura o viewport ao contexto geral. |
| Rodinha do mouse | Zoom horizontal e rolagem vertical da timeline. |

## Beat Sequencer

| Atalho / Ação | Descrição |
| --- | --- |
| Botão ▶/⏸ do header do popup | Play/stop do sequenciencer. É o mesmo transporte do `Espaço` com o cursor sobre a popup, então nunca mexe na timeline. |
| Clique em um passo do grid | Liga/desliga a nota (`Ctrl+clique` para estender/caret). **Ligar também pinta a nota na timeline**: o passo não fica preso ao sequenciador, ele vira nota MIDI no **compasso em que o playhead está**, dentro de um clip com o nome do padrão. É o que faz a bateria soar no play da timeline (que silencia o step sequencer). Desligar o passo remove a nota. O clip resultante é um MIDI normal e **independente** do pattern: dá pra arrastar, cortar, abrir no piano roll e apagar, e mexer no swing/resolução depois não reescreve o que já foi gravado. Um clique = um `Ctrl+Z`. |
| Clique num padrão na seção **BEAT PATTERNS** do Clip Pool | Seleciona o padrão no sequencer (song mode). |
| Arraste um padrão do Clip Pool para a timeline | Cria um clip MIDI regular com a duração do padrão. Equivale a pintar o pattern inteiro de uma vez — as duas rotas geram as mesmas notas. |
| **Botões do popup**: `Copy` / `Duplicate` / `Clear` / `Delete` | Copiam/duplicam/limpam/apagam padrões, trilhas ou linhas. |

## Referência rápida

| Tecla | Um toque |
| --- | --- |
| `Ctrl+C` | Copiar clip |
| `Ctrl+D` | Duplicar clip |
| `Ctrl+M` | Alternar Musical/Linear do clip (timewarp) |
| `Ctrl+Z` / `Ctrl+Y` / `Ctrl+Shift+Z` | Undo / Redo |
| `Delete` / `Backspace` | Apagar clip(s) |
| `Space` | Play/Stop (roteado pelo cursor: popup do beat → sequenciencer; resto → timeline) |
| `F` | Focar/zoom no clip selecionado |

---

*Mantido em sincronia com o código: `ui/main.slint` (handler global de
teclas + `toggle-play-routed`), `app/src/host.rs` (rota do `Espaço` do
winit + contexto do ponteiro), `app/src/playback.rs` (troca de modo do
transporte), `ui/timeline/timeline.slint` (clique fora, Shift+clique,
marquee), `ui/clip_pool.slint` (seção BEAT PATTERNS).*
