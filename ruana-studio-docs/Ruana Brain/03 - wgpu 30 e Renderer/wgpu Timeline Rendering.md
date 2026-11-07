# wgpu Timeline Rendering

> Técnicas de renderização de timeline DAW com wgpu no RuanaStudio.

---

## Quando usar

- Renderizar viewport de timeline com wgpu (standalone ou embedded em Slint)
- Renderizar texto (números de compasso, labels de tempo) sem biblioteca de texto completa
- Subdivisões de grid dependentes de zoom (1/4, 1/8, 1/16, 1/32, 1/64)
- Instanced rendering para elementos de timeline (clips, linhas de grid)
- Animação de playhead amarada à posição de scroll

## Padrão: Fonte Bitmap via Instancing

```rust
fn build_text_instances(
    text: &str,
    x: f32, y: f32,
    char_w: f32, char_h: f32,
    gap: f32,
    color: [u8; 4],
) -> Vec<RectInstance> {
    let mut instances = Vec::new();
    for (i, ch) in text.chars().enumerate() {
        let glyph = FONT_8X8.get(ch);
        let cx = x + i as f32 * (char_w + gap);
        for row in 0..8 {
            for col in 0..8 {
                if glyph[row][col] {
                    instances.push(RectInstance::new(
                        cx + col as f32 * char_w / 8.0,
                        y + row as f32 * char_h / 8.0,
                        char_w / 8.0, char_h / 8.0,
                        color,
                    ));
                }
            }
        }
    }
    instances
}
```

**Pitfall**: tabela de fonte completa é verbosa. Para régua de DAW, apenas dígitos 0-9 são necessários — tabela de 10 entradas é suficiente.

## Padrão: Subdivisões de Grid Dependentes de Zoom

| Threshold (px_per_bar) | Subdivisão | Alpha (body) | Ruler tick | Tick alpha |
|---|---|---|---|---|
| ≥40 | 1/4 (beat) | 150 | 10px | 200 |
| ≥120 | 1/8 | 130 | 8px | 180 |
| ≥240 | 1/16 | 110 | 6px | 160 |
| ≥480 | 1/32 | 90 | 4px | 140 |
| ≥960 | 1/64 | 70 | 2px | 120 |

**Pitfall**: thresholds devem ser monótonos. Lacunas entre thresholds criam bandas invisíveis.

## Padrão: Números de Compasso na Régua

Números de compasso devem ser centralizados em sua barra na área da régua.

**Pitfall**: centralização deve considerar multi-dígitos. "10" tem o dobro da largura de "1".

## Padrão: Header de Clip

```rust
let header_h = 16.0;  // Mínimo para legibilidade, maximiza espaço vertical para waveform
```

**Pitfall**: manter standalone e app sincronizados. `timeline-render/src/main.rs` e `app/src/timeline_canvas.rs` devem usar o mesmo `header_h`.

## Padrão: Sincronização Standalone ↔ App

Dois codebases renderizam a mesma timeline:
- `timeline-render/src/main.rs` — janela wgpu standalone (demo/preview)
- `app/src/timeline_canvas.rs` — render headless offscreen embedded no Slint

Ambos devem compartilhar:
1. Mesmo shader (`wgpu.wgsl` / `timeline.wgsl`)
2. Mesmo layout de `RectInstance`
3. Mesmas arrays de paleta (SURFACE_HIGH, GRID_LINE, GRID_SUB, TIMELINE_BG, CLIP_PALETTE, PLAYHEAD_COLOR)
4. Mesmos thresholds de subdivisão e valores de alpha
5. Mesmo `header_h` (16.0)
6. Mesma cor de playhead (#DC143C crimson)
7. Mesma fonte bitmap 8x8 para números da régua

**Pitfall**: manutenção dual. Qualquer mudança em grid/subdivisão/header deve ser aplicada em AMBOS os arquivos.

## Padrão: Playhead com Consciência de Scroll

```rust
// Sempre clipar ambos > 0 e < width para evitar triângulo off-screen
let playhead_x = (playhead_beat * px_per_beat - scroll_x).max(0.0).min(width);
```

## Padrão: Auto-Scroll de Borda Durante Drag de Clip

- Definir `edge_zone` (ex.: 40px) de cada borda do viewport como banda de trigger
- Velocidade proporcional à proximidade: `speed = max_speed * proximity`
- `update_edge_scroll` e `apply_edge_scroll` são chamados em `RedrawRequested`, NÃO em `handle_cursor_moved`
- Compensar `drag_live_beat += dx / pps` para manter clip visualmente preso ao cursor
- Resetar edge_scroll em todo `begin_drag()` e no release

## Padrão: Roteamento de Eventos em Modo Hosted (winit + Slint Overlay)

Quando a timeline é renderizada via wgpu em modo hosted (offscreen) com overlay Slint por cima, `WindowEvent`s do winit devem ser roteados para ambos o engine wgpu e o overlay Slint.

**Pitfall**: `slint::platform::WindowEvent::PointerPressed` NÃO carrega modificadores de teclado. Ações gated por modificador (Shift+click paste, Ctrl/Ctrl+scroll zoom) devem ser interceptadas no handler winit usando estado de modificador armazenado em `RefCell` compartilhado.

## Referências

- `timeline-render/src/main.rs` — implementação completa
- `timeline-render/src/timeline.wgsl` — pipeline de shader WGSL
- `app/src/timeline_gpu/wgpu.rs` — renderer wgpu em nível de app (headless)

## Links

- [[Referência wgpu 30]]
- [[Patterns de UI Slint para DAW]]
- [[Arquitetura Geral]]
