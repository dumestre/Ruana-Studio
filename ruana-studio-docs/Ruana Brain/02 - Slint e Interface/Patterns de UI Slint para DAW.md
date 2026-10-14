# Patterns de UI Slint para DAW

> Padrões de interação Slint específicos para DAW (clip editing, fades, gain).

---

## Quando usar

- Implementar edição de clips com controles de fade e gain
- Visualização de waveform com atualizações em tempo real
- Gestos de mouse para drag/drop de clips de áudio
- Feedback visual para manipulação de envelope de áudio
- Tematização de cores de grip/handle a partir do tema central
- Prevenir movimento acidental de clip durante interação com fade/gain

## Padrão: Controle de Fade

```slint
Path {
    commands: "M 0,100 C 25,100 75,0 100,0";  // Fade-in com Bezier
    // ou
    commands: "M 0,0 C 25,0 75,100 100,100";  // Fade-out with Bezier
}
```

**Por que Bezier?** Fornece feedback visual sobre a forma do envelope de fade, importante para percepção de transição de áudio.

## Padrão: Gerenciamento de Estado de Mouse

Clips de DAW usam padrão "live" vs "display":

- `fade-in`, `fade-out` — vêm do modelo (Rust core)
- `live-fade-in`, `live-fade-out` — durante drag, seguem o mouse
- `display-fade-in`, `display-fade-out` — computado: `fading ? live-* : model-*`

## Padrão: Feedback de Cursor

```slint
mouse-cursor:
    root.fading ? ew-resize
    : root.on-fade-grip ? ew-resize
    : root.on-edge ? ew-resize
    : pointer;
```

## Padrão: Drag Local-Only

Todos os gestos commitam uma vez no UP (release), não durante MOVE:

```slint
if event.kind == PointerEventKind.down {
    if root.on-fade-grip {
        root.fading = true;
        // Inicializa live-*
        return;
    }
}
if event.kind == PointerEventKind.move {
    if root.fading {
        // Atualiza live-*
        return;
    }
}
if event.kind == PointerEventKind.up {
    root.fading = false;
    root.clip-fade-committed(root.clip-id, fi, fo);
}
```

## Padrão: Re-render de Waveform

```slint
TimelineMetrics.waveform-resolution-requested(
    id, width, wx0, wx1, height,
    gain-linear, fade-in-px, fade-out-px
);
```

## Padrão: Controle de Gain

1. **Display property**: `gain-linear = Math.pow(10.0, display-gain-db / 20.0)`
2. **Sensibilidade**: 1.5px por dB
3. **Range válido**: -24dB a +12dB
4. **Feedback visual**: Grip no centro mostrando valor dB durante drag

## Padrão: Cores de Grip Tematizadas

```slint
private property <bool> fade-grip-active:
    root.on-fade-grip
    && (touch.pressed || root.fading);

private property <bool> gain-grip-active:
    root.on-gain-grip
    && (touch.pressed || root.gaining);
```

- hover/normal: `Theme.accent-hover`
- pressed/active: `Theme.accent`
- border/text on accent: `Theme.on-accent`

## Padrão: Layout Horizontal Estilo Ableton (Meter)

Evitar `HorizontalLayout` dentro do componente. Usar posicionamento absoluto:

```slint
export component Meter inherits Rectangle {
    private property <length> triangle-width: 14px;
    private property <length> meter-width: 13px;
    private property <length> gap: 2px;
    private property <length> db-width: 16px;

    width: 45px;  // triangle + gap + meter + gap + db
    min-height: 40px;

    if root.show-volume-control : Rectangle {
        x: 0px;
        y: 0px;
        width: root.triangle-width;
        height: parent.height;
    }

    Rectangle {
        x: root.show-volume-control ? root.triangle-width + root.gap : 0px;
        y: 0px;
        width: root.meter-width;
        height: parent.height;
    }

    if root.show-volume-control : Rectangle {
        x: root.triangle-width + root.gap + root.meter-width + root.gap;
        y: 0px;
        width: root.db-width;
        height: parent.height;
    }
}
```

**Pitfall**: largura total deve ser exata. `padding-bottom` não funciona em `Rectangle` — usar `Text.y = parent.height - text-height`.

## Padrão: Meter Signal-Fill-Only

```slint
Meter {
    signal-fill-only: true;
}
```

## Prevenir Movimento Acidental de Clip

```slint
if root.on-fade-grip {
    root.fading = true;
    return;  // Garante que não cai no drag genérico
}
if root.on-gain-grip {
    root.gaining = true;
    return;
}
// Só então cair para resize/drag/marquee
```

## Referências

- `ui/timeline/track_clip.slint`, `ui/timeline/clip_data.slint`
- `app/src/waveform.rs`
- `ui/theme.slint`

## Links

- [[Referência Slint 1.18]]
- [[wgpu Timeline Rendering]]
