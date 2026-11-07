# Sistema de Tema

> Como o tema do Ruana Studio funciona de ponta a ponta: 7 paletas, a
> "escada" de superfícies, tokens, troca em runtime, persistência e cursores.

**Última verificação:** 2026-10-13 · **Fontes:** `ui/theme.slint`,
`ui/components/theme_picker.slint`, `app/src/theme.rs`, `app/src/cursors.rs`,
`ui/main.slint`, `app/src/main.rs`

---

## 1. Princípio

Existe **um único global de tema** (`Theme`, em `ui/theme.slint`) e **toda a UI
consome apenas os tokens dele**. Não há cor literal espalhada nos componentes:
trocar de tema é escrever `Theme.active` e o resto do app repinta por binding.

```
MainWindow.theme-index   (property da janela, visível ao Rust)
        │  changed
        ▼
   Theme.active          (global in-out, índice 0..6)
        │  clamp → *_list[clamp]
        ▼
   Theme.base, Theme.surface, Theme.accent, …   (tokens expostos)
        │
        ▼
   todos os componentes .slint
```

O Rust **não tem acessor gerado para globals do Slint**. Por isso a aplicação
passa pela property raiz `MainWindow.theme-index`, cujo handler `changed` seta
`Theme.active` (`ui/main.slint:410-413`). É o único caminho de escrita.

---

## 2. As 7 paletas

Cada token é uma **lista de 7 cores**, uma por tema, na mesma ordem de índice.
O comentário no arquivo nomeia cada posição (`[0]`…`[6]`), e a ordem é sempre
a mesma em todas as listas:

| # | Nome | Papel |
|---|---|---|
| 0 | **Studio Dark** | Tema PADRÃO do app |
| 1 | Midnight Blue | |
| 2 | Violeta Ruana | |
| 3 | Volt | |
| 4 | Sunset | |
| 5 | Ocean Teal | |
| 6 | **Paper** | Único tema CLARO — tem tratamento especial |

`Paper` não é só "mais uma paleta": é o tema claro, e por isso tem dois
desvios deliberados em relação aos outros (ver §5 e §6).

---

## 3. A escada de superfícies (regra de uso)

Esta é a regra que organiza todo o design. **Não pular degrau.**

| Token | O que é | Exemplos |
|---|---|---|
| `base` | O **fundo**. A janela, o vazio atrás de tudo, e os POÇOS/"inset" — campo de texto, LCD, trilho de medidor, slot vazio. Coisa que afunda. | MainWindow |
| `surface` | O **painel** que senta em cima do fundo. | Explorer, canal do mixer, painel do beat, popup |
| `surface-high` | O **widget** dentro do painel. Botão em repouso, aba, item de lista, célula do grid. | botões da edit_toolbar |

Dois erros que a escada elimina (documentados no próprio arquivo):

- **painel com `surface-high`** → vira widget; some um degrau e o filho não
  contrasta com o pai;
- **botão com `base`** → vira poço; some um degrau para TRÁS e o botão fica
  mais escuro que o painel que o contém. **Botão em repouso é SEMPRE
  `surface-high`.**

### Degraus exclusivos da Timeline

Estes **não existem fora da timeline**. Explorer, mixer e beat não os tocam —
eles usam só a escada de 3 degraus:

| Token | Papel |
|---|---|
| `surface-step` | Um degrau acima de `surface`; é o fundo do **grid** da timeline. É o que `grid-line`/`grid-sub` precisam enxergar. |
| `timeline-bg` | Degrau próprio: mais claro que `base`, mais escuro que `surface-step`. |
| `grid-line` | Grid principal. |
| `grid-sub` | Sub-grid (divisões menores). |

Cada tema escuro foi calibrado com valores explícitos, ex. Studio Dark:
`base #313131` → `timeline-bg #696969` → `surface-step #424242` → `surface
#242424` → `surface-high #555555`. Os comentários no arquivo registram sobre
qual fundo cada cor de grid precisa ser visível.

---

## 4. Tokens expostos (a API do tema)

Só estes nomes são consumidos pela UI (`ui/theme.slint:324-349`):

| Token | Uso |
|---|---|
| `base`, `surface`, `surface-high` | escada principal |
| `border` | bordas e separadores |
| `shadow` | cor do `drop-shadow` de widgets elevados |
| `accent`, `accent-hover`, `on-accent` | destaque e o texto sobre ele |
| `text-primary`, `text-dim` | texto principal / discreto |
| `ok`, `warn`, `danger` | semântico |
| `playhead` | cor da agulha (`#DC143C` em todos os temas) |
| `crimson`, `on-crimson` | fundo das **ferramentas ativas** na toolbar |
| `clip-audio`, `clip-midi` | cores-base dos clips |
| `activ` | estado "recém-ativado" |
| `grid-line`, `grid-sub` | grid da timeline |
| `toggle-active-bg` | toggles ativos (follow/snap) |
| `timeline-bg`, `surface-step` | exclusivos da timeline |
| `is-paper` | **`bool`** = `active == 6` |
| `Transparent` | `#00000000` |

### Regra de contraste

`on-accent` / `on-crimson` são a cor do **texto quando o fundo já é**
`accent` / `crimson`. Nunca usar `text-primary` sobre accent.

### Regra da sombra

A sombra tem de ser **sempre mais escura que o fundo onde ela cai**. No Paper
(fundo claro) `#2F2F2F` entrega isso de graça. Nos temas escuros o mesmo
`#2F2F2F` ficaria **mais claro** que o painel (`surface #242424`, `base
#313131`), virando **halo em vez de sombra** — por isso cada tema escuro tem o
seu quase-preto, tingido com o matiz da própria paleta para a elevação ler
igual ao Paper.

---

## 5. Paper (o tema claro)

Paper é índice **6** e a constante `THEME_PAPER: i32 = 6` em `app/src/theme.rs`
existe para não espalhar o número mágico. Dois desvios deliberados:

1. **Sombra `#2F2F2F`** — marcada no arquivo como *"aprovado, NÃO MEXER"*.
2. **Cursores escuros** — `set_dark_cursors(idx == theme::THEME_PAPER)`. No
   Paper os cursores de ferramenta usam a variante `-dark.cur` (ver
   [[Cursores e Ferramentas]]).

Vários tokens marcam o Paper com ★ e recebem valores próprios:
`accent`/`accent-hover` continuam verdes (`#00C853`/`#09F664`) mas `text-primary`
vira `#1E1E1E`, `border` vira `#D2D2D2`, `grid-line` vira `#D6D6D6`, etc.

---

## 6. Troca de tema em runtime

### Do Slint

`ui/components/theme_picker.slint` — componente `ThemePicker`:

- caixa de 92×18 px, mostra o nome do tema ativo (`names[safe-index]`);
- `safe-index` = `min(max(0, Theme.active), names.length - 1)` — o seletor
  nunca indexa fora da lista;
- clique abre um `PopupWindow` (132×152) com a lista rolável;
- o item ativo é destacado com `Theme.accent.transparentize(0.78)` e as linhas
  alternadas usam `surface` para zebrado;
- clicar faz **duas coisas**: `Theme.active = i` (repinta na hora) e
  `root.theme-selected(i)` (avisa o Rust).

O próprio seletor usa `Theme.surface-high` de fundo e `Theme.accent` no texto
— é *"o único jeito de dar tema no próprio seletor antes de aplicar"*.

### Do Rust (persistência)

`app/src/theme.rs`:

```rust
pub const THEME_COUNT: i32 = 7;
pub const THEME_PAPER: i32 = 6;

fn cfg_path() -> PathBuf   // theme.cfg ao lado do executável
pub fn apply(ui, index) -> i32   // clamp(0, 6) + set_theme_index
pub fn save(index)               // grava o índice
pub fn load() -> Option<i32>     // lê no boot
```

No boot (`app/src/main.rs`):

```rust
let idx = theme::load().unwrap_or(0);
theme::apply(ui, idx);
set_dark_cursors(idx == theme::THEME_PAPER);
```

No callback `theme-selected`:

```rust
let idx = theme::apply(&ui, index);
theme::save(idx);
set_dark_cursors(idx == theme::THEME_PAPER);
```

Ou seja: **o Rust valida/clampa, persiste em `theme.cfg` e sincroniza os
cursores**; quem guarda as cores é só o `.slint`.

---

## 7. Fluxo completo de uma troca de tema

1. Usuário clica no `ThemePicker` (na Toolbar).
2. Slint: `Theme.active = i` → todos os bindings `*_list[clamp]` re-selecionam
   → a UI inteira repinta.
3. Slint: `theme-selected(i)` sobe até o `MainWindow`.
4. Rust: `theme::apply` clampa e escreve `MainWindow.theme-index`.
5. Rust: `theme::save(i)` grava `theme.cfg`.
6. Rust: `set_dark_cursors(i == 6)` troca a família de cursores se for Paper.

---

## 8. Pitfalls conhecidos

1. **Não existe acessor Rust para global do Slint.** Escrever tema por outro
   caminho que não `MainWindow.theme-index` não funciona.
2. **A ordem das listas é contrato.** Inserir um tema no meio de uma lista sem
   inserir em TODAS as outras desalinha as paletas silenciosamente (compila,
   mas as cores saem trocadas).
3. **`shadow` não é cosmético.** Um quase-preto reaproveitado entre temas
   claros e escuros vira halo (ver §4).
4. **Cursores seguem o tema.** Esquecer `set_dark_cursors` deixa o cursor
   claro invisível sobre o Paper.
5. **`is-paper` é o único token booleano** — use-o em vez de comparar
   `Theme.active == 6` espalhado pelos componentes.

---

## Links

- [[Referência Slint 1.18]]
- [[Cursores e Ferramentas]]
- [[Fluxo de Interface]]
- [[Componentes e Tokens]]
- [[Patterns de UI Slint para DAW]]
