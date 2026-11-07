# Referência Slint 1.18

> Referência do Slint usado no RuanaStudio (1.18.0, backend Skia + wgpu overlay).

---

## Fontes oficiais

- **Slint Docs**: https://docs.slint.dev/latest/docs/slint/
- **Rust API**: https://docs.slint.dev/latest/docs/rust/slint/
- **Language Reference**: https://docs.slint.dev/latest/docs/slint/reference/
- **std-widgets**: https://docs.slint.dev/latest/docs/slint/reference/std-widgets/
- **Blog 1.18 release**: https://slint.dev/blog/slint-1.18-released

## Features usadas no projeto

```toml
slint = { workspace = true, features = [
    "renderer-skia",           # Renderização via Skia
    "raw-window-handle-06",   # Integração com winit 0.30
    "unstable-wgpu-30",       # Backend wgpu 30 (overlay)
] }
```

## Build

```toml
[build-dependencies]
slint-build = "1.18.0"
```

```rust
// build.rs
fn main() {
    slint_build::compile("ui/main.slint").unwrap();
}
```

## Estrutura de pastas UI

```
ui/
├── main.slint              # Componente raiz
├── theme.slint             # Tokens de tema (cores, espaçamento)
├── timeline/
│   ├── timeline.slint      # Componente timeline
│   ├── track_clip.slint    # Clip individual
│   └── clip_data.slint     # Dados de clip
├── mixer/
│   ├── mixer.slint         # Mixer principal
│   └── mixer_channel.slint # Canal individual
├── piano_roll.slint         # Piano roll
├── clip_pool.slint         # Pool de clips
└── ...
```

## Padrões da linguagem

### Componente

```slint
export component MyButton inherits Rectangle {
    in property <string> text: "Click me";
    out property <bool> pressed: false;
    callback clicked();

    TouchArea {
        clicked => { root.clicked(); }
    }
    Text {
        text: root.text;
    }
}
```

### Propriedades

```slint
// Input property (default)
in property <int> value: 0;

// Output property (read-only para consumidores)
out property <bool> active: false;

// Private property (interno ao componente)
private property <length> internal-width: 40px;

// Property com binding
property <color> background: Theme.surface;
```

### Callbacks

```slint
// Declaração
callback value-changed(int);
callback clip-selected(int, int);

// Invocação
TouchArea {
    clicked => {
        root.value-changed(42);
        root.clip-selected(1, 2);
    }
}
```

### Layouts

```slint
VerticalLayout {
    spacing: 8px;
    padding: 12px;
    alignment: LayoutAlignment.stretch;

    Rectangle { /* ... */ }
    HorizontalLayout {
        spacing: 4px;
        Text { text: "Label"; }
        TextInput { text: "value"; }
    }
}
```

### Estados e animações

```slint
// State
states [
    pressed: {
        background: Theme.accent;
        text-color: Theme.on-accent;
    }
    hover: {
        background: Theme.surface-high;
    }
]

// Animação
animate background {
    duration: 150ms;
    easing: ease-out;
}
```

### Modelos (arrays reativos)

```slint
// Em Rust
let model = Rc::new(slint::VecModel::from(vec![
    ClipData { name: "Kick".into(), color: Color::from_rgb_u8(255, 0, 0) },
    ClipData { name: "Snare".into(), color: Color::from_rgb_u8(0, 255, 0) },
]));

// Em Slint
for clip in model : ClipItem {
    // clip.name, clip.color
}
```

### Funções

```slint
export function format-time(seconds: float) -> string {
    let mins = floor(seconds / 60.0);
    let secs = seconds - mins * 60.0;
    return "{mins}:{secs:01.0}";
}
```

## Integração com Rust

```rust
// Gerar código a partir do .slint
slint::include_modules!();

// Instanciar componente
let app = App::new().unwrap();

// Acessar propriedades
app.set_bpm(128.0);
let bpm = app.get_bpm();

// Registrar callbacks
app.on_clip_selected(move |track, beat| {
    println!("Clip selected at track {}, beat {}", track, beat);
});

// Rodar
app.run().unwrap();
```

## Tema

O tema vive em `ui/theme.slint` e é acessível via `Theme.*`:

```slint
// theme.slint
export global Theme {
    in property <color> surface: #1e1e1e;
    in property <color> surface-high: #2a2a2a;
    in property <color> accent: #4a9eff;
    in property <color> on-accent: #ffffff;
    in property <color> text-primary: #e0e0e0;
    in property <color> text-secondary: #a0a0a0;
    in property <color> grid-line: #333333;
    in property <color> playhead: #dc143c;
}
```

## Pitfalls conhecidos

1. **Binding loops**: `Flickable.content-height -> layout-cache -> height -> layoutinfo-v -> root.layoutinfo-v -> min-height -> flickable.content-height` causa panic em tempo de compilação. Solução: usar posicionamento absoluto em vez de `HorizontalLayout` dentro de `Flickable`.

2. **Parse errors aparecem como panic no build script**: erro de sintaxe `.slint` não falha o crate limpanmente — panica dentro do `build.rs`. A linha reportada frequentemente NÃO é a linha real do erro. Contar chaves antes de investigar.

3. **Eventos de pointer vazam entre popups e timeline**: adicionar `TouchArea` de fundo no popup para consumir eventos.

4. **`padding-bottom` não funciona em `Rectangle`**: usar `Text.y = parent.height - text-height` para posicionar texto com padding inferior.

## Links

- [[Referência Rust]]
- [[Patterns de UI Slint para DAW]]
- [[Arquitetura Geral]]
