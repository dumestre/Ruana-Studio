# Cursores e Ferramentas

> Como os cursores de ferramenta da timeline são carregados, aplicados e
> trocados entre as variantes clara e escura. Implementação **Win32 nativa**.

**Última verificação:** 2026-10-13 · **Fonte:** `app/src/cursors.rs` (442 linhas)

---

## 1. Por que Win32 nativo

Slint não expõe "cursor de arquivo `.cur`". Para uma DAW, o cursor **é** a
indicação de ferramenta ativa (como no Ableton/Cubase): a faca, a cola, o
stretch e o lápis precisam de cursores próprios. A solução é falar direto com
o Win32:

- `LoadImageW(…, IMAGE_CURSOR, LR_DEFAULTSIZE | LR_LOADFROMFILE)` carrega o
  `.cur` do disco;
- `SetWindowSubclass(hwnd, cursor_subclass_proc, …)` intercepta `WM_SETCURSOR`
  para **reimpor** o cursor a cada movimento do mouse.

O subclassing é necessário porque qualquer widget pode chamar `SetCursor`
por baixo e sobrescrever o cursor customizado.

---

## 2. As 6 ferramentas

`enum ToolCursor` mapeia **1:1 com o índice da ferramenta na UI**:

| Índice | `ToolCursor` | Arquivo | Ícone |
|---|---|---|---|
| 0 | `Select` | `select.cur` | `select.svg` / `select_filled.svg` |
| 1 | `Slice` | `slice.cur` | `slice.svg` / `slice_filled.svg` |
| 2 | `Glue` | `glue.cur` | `glue.svg` / `glue_filled.svg` |
| 3 | `Stretch` | `stretch.cur` | `stretch.svg` / `stretch_filled.svg` |
| 4 | `Draw` | `draw.cur` | `draw.svg` / `draw_filled.svg` |
| 5 | `Erase` | `erase.cur` | `erase.svg` / `erase_filled.svg` |

`ToolCursor::from_tool_index(i)` faz a conversão; índice fora de 0..5 devolve
`None` e o cursor volta ao padrão (seta do sistema).

`ui/edit_toolbar.slint` é quem escreve `active-tool` (0..5) e dispara
`tool-changed(i)`; o Rust roteia para `set_cursor_for_tool(i)`.

---

## 3. Variante escura (tema Paper)

Cada ferramenta tem **dois** arquivos: `X.cur` (clara, para temas escuros) e
`X-dark.cur` (escura, para o Paper).

```rust
pub fn dark_cursor_file_name(&self) -> &'static str  // "select-dark.cur", …
pub fn set_dark(&mut self, dark: bool)               // reaplica a ferramenta atual
```

`set_dark` **reaplica o cursor da ferramenta corrente** quando ela muda — não
basta guardar a flag, senão o cursor só trocaria na próxima troca de
ferramenta. Ver [[Sistema de Tema]] §6 para quem chama isso.

`CursorManager` mantém dois `HashMap<ToolCursor, CursorHandle>`: `cursors` e
`dark_cursors`.

---

## 4. Onde os cursores moram

`cursor_directory()` tenta, nesta ordem:

1. `exe_dir/cursors` (layout padrão do `build.rs`);
2. `exe_dir/../cursors`;
3. `cwd/app/cursors`;
4. `cwd/cursors`.

Vence o primeiro diretório que **existe e não está vazio**. Se nenhum servir,
cai em `exe_dir/cursors` e cada `load_cursor` falha com log
`[cursor] arquivo nao encontrado: …`.

Se o arquivo existe mas o `LoadImageW` devolve `NULL`, o log é
`[cursor] LoadImageW falhou para … (file existe, load retornou null)` — útil
para distinguir "não achei o arquivo" de "o arquivo é inválido".

---

## 5. Estado global (o padrão `OnceLock`)

O cursor precisa ser lido de dentro do `subclass_proc`, que é uma função C
sem acesso a `self`. Por isso o módulo usa estáticos:

| Estático | Tipo | Papel |
|---|---|---|
| `CURSOR_MANAGER` | `OnceLock<Mutex<CursorManager>>` | o gerenciador |
| `ACTIVE_CURSOR` | `OnceLock<Mutex<Option<isize>>>` | handle do cursor a reimpor |
| `TIMELINE_BOUNDS` | `OnceLock<Mutex<TimelineBounds>>` | retângulo da timeline |
| `CURSOR_OVER_TIMELINE_ENABLED` | `OnceLock<AtomicBool>` | restringe à timeline |

`CursorHandle` tem `unsafe impl Send + Sync` porque `HCURSOR` é um ponteiro
cru — o contrato é que ele é válido pela vida do processo.

---

## 6. Restrição à área da timeline

`WM_SETCURSOR` com `(lparam & 0xFFFF) == 1` significa "cursor sobre a área de
cliente". O handler então decide:

```rust
if !must_check || is_cursor_over_timeline(hwnd) {
    SetCursor(active);  // cursor customizado
    return 1;
}
```

- `is_cursor_over_timeline` pega `GetCursorPos` → `ScreenToClient(hwnd)` e
  testa contra `TIMELINE_BOUNDS`;
- **correção de DPI:** `ScreenToClient` devolve **pixels físicos**, mas os
  bounds são publicados pelo Slint em **unidades lógicas**. Em DPI ≠ 100% a
  checagem falharia sempre e o cursor nunca apareceria. Por isso o ponto é
  convertido físico → lógico com `scale = GetDpiForWindow(hwnd) / 96.0`.

`set_timeline_bounds(x, y, w, h)` é quem alimenta o retângulo; `valid` começa
`false`, então **antes da primeira publicação o cursor customizado não é
aplicado**.

---

## 7. API pública

```rust
init_cursor_manager() -> Result<()>            // cria o manager (OnceLock)
set_cursor_for_tool(tool_index: i32) -> Result<()>
set_dark_cursors(dark: bool) -> Result<()>
set_timeline_bounds(x, y, w, h)                // no-op fora do Windows
reset_cursor_to_default() -> Result<()>        // volta à seta do sistema
set_cursor_window_handle(hwnd: *mut c_void)    // instala o subclass
```

Fora do Windows todo o módulo compila para no-ops (`#[cfg(not(windows))]`) —
a API existe, o efeito é nulo. `init_cursor_manager` e
`set_cursor_for_tool` logam em `stderr` com o prefixo `[cursor]`.

`WindowEvent::CloseRequested` chama `reset_cursor_to_default()` antes de
`event_loop.exit()` (`app/src/host.rs:865-868`) — sem isso a seta ficaria
"presa" no cursor de ferramenta se a janela fechasse com um ativo.

---

## 8. Pitfalls conhecidos

1. **`.cur`, não `.png`.** `LoadImageW` com `IMAGE_CURSOR` exige o formato
   `.cur` (com hotspot embutido).
2. **DPI.** Toda comparação de posição física × bounds lógicos precisa do
   divisor de escala (§6). Este foi um bug real e está comentado no código.
3. **`set_dark` sem ferramenta ativa é no-op silencioso** — ele só reaplica
   se `current_tool` for `Some`.
4. **O subclass é instalado por janela.** A janela flutuante do mixer/sampler
   não herda o cursor da timeline.
5. **Diretório vazio = diretório inválido.** Um `cursors/` existente mas vazio
   faz o fallback pular para o próximo candidato.

---

## Links

- [[Sistema de Tema]]
- [[Fluxo de Interface]]
- [[Referência winit 0.30]]
