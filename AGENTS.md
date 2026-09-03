# AGENTS.md — PLAN: Cores e Grid da Timeline Procedural

*(Copie este texto para o arquivo AGENTS.md do seu projeto)*

---

## PLANO: Dados de Cores e Grid para Timeline Procedural (timeline-render)

### 1. Fonte de Cores — `ui/theme.slint` (tokens por índice ativo)

Todos os tokens abaixo vêm do `ui/theme.slint` (linhas 277-301). O Rust deve expor esses valores via uniform buffer para o WGSL ler.

#### Tokens principais (nomes que o procedural consumirá):

```
Theme.base
Theme.surface
Theme.surface-high
Theme.border
Theme.accent
Theme.grid-line
Theme.grid-sub
Theme.timeline-bg
Theme.surface-step
Theme.playhead      // sempre #DC143C (crimson) — todos os temas
Theme.is-paper       // bool: true quando active == 6 (tema Paper/claro)
```

#### Tabela completa de cores por tema (índice 0 a 6):

| Token | 0:Studio Dark | 1:Midnight Blue | 2:Violeta Ruana | 3:Volt | 4:Sunset | 5:Ocean Teal | 6:Paper (claro) |
|-------|--------------|----------------|-----------------|--------|----------|-------------|-----------------|
| **base**      | `#313131` | `#161A21` | `#191722` | `#14160E` | `#211A12` | `#0F1E21` | `#6464` |
| **surface**   | `#242424` | `#1D242E` | `#221E2E` | `#1C1F15` | `#2A2117` | `#15262A` | `#919191` |
| **surface-high** | `#555555` | `#26303D` | `#2D2740` | `#262A1D` | `#362A1B` | `#1E3035` | `#919191` |
| **border**    | `#3D3D3D` | `#33404F` | `#3C3350` | `#383F2C` | `#4A3A26` | `#2C4247` | `#D2D2D2` |
| **accent**    | `#00C853` | `#4FA8FF` | `#B85CFF` | `#A8FF00` | `#FF9F43` | `#2EC4B6` | `#00C853` |
| **playhead**  | `#DC143C` | `#DC143C` | `#DC143C` | `#DC143C` | `#DC143C` | `#DC143C` | `#DC143C` |
| **grid-line** | `#757575` | `#4A5A70` | `#4A4466` | `#4A4D3A` | `#5A4E3A` | `#3A5C66` | `#D6D6D6` |
| **grid-sub**  | `#454545` | `#2A3645` | `#2F2C42` | `#2A2D24` | `#3A3028` | `#21363C` | `#EFEFEF` |
| **timeline-bg** | `#696969` | `#1E2C35` | `#1F1B28` | `#181B11` | `#252119` | `#122628` | `#797979` |
| **surface-step** | `#424242` | `#232D38` | `#272334` | `#20231A` | `#30261F` | `#192C31` | `#7A7A7A` |

**Nota:** `Theme.playhead` é fixo em `#DC143C` (crimson) para **todos** os 7 temas. Não varia.

---

### 2. Grid — Comportamento por Zoom (px-per-bar)

O grid se adapta baseado na largura do compasso em pixels. Regras (já implementadas no `ruler.slint:120-356`):

| px-per-bar | O que aparece |
|-----------|--------------|
| **< 40px** | Apenas linhas de compasso (main grid) |
| **≥ 40px** | + Linhas de batida (beat divisions — a cada 1 beat) |
| **≥ 120px** | + Subdivisões 1/8 (a cada 0.5 beat) |
| **≥ 240px** | + Subdivisões 1/16 (a cada 0.25 e 0.75 beat) |

#### Cores do grid por camada:

- **Main grid lines** (linhas principais do compasso): `Theme.grid-line`
- **Subdivisões 1/8**: `Theme.is-paper ? Theme.base.transparentize(0.6) : Theme.grid-line.transparentize(0.6)`
- **Subdivisões 1/16**: `Theme.is-paper ? Theme.base.transparentize(0.8) : Theme.grid-line.transparentize(0.8)`

#### Fundo dos compassos (alternância):
- Ímpar: `Theme.surface-high`
- Par: `Theme.surface-step`

#### Texto dos números dos compassos:
- Ímpar: `Theme.text-primary`
- Par: `Theme.text-dim`
- Fonte: `10px` / `9px` dependendo de par/ímpar
- Peso: `700` / `500`

---

### 3. Ruler (Régua fixa acima da timeline)

Estrutura a recriar no procedural:

1. **Fundo da régua:** `Theme.surface-high`
2. **Linhas verticais dos compassos:** `1px` sólido em `Theme.grid-line` (ou transparente no Paper com `0.45` alpha)
3. **Números dos compassos:**
   - Posição `x: 0px` (colado no grid — mudou de `3px` para `0px`)
   - Texto: `Theme.text-primary` / `Theme.text-dim` (alternância par/ímpar)
   - Fonte tamanho: `10px` / `9px`
   - Peso: `700` / `500`
4. **Linhas de batida** (quando `px-per-bar >= 40px`):
   - Linhas de 1px em `Theme.grid-line.transparentize(0.6)`
   - Posicionadas a cada beat (`k * px-per-beat`)
5. **Subdivisões 1/8** (quando `px-per-bar >= 120px`):
   - Linhas menores de 7px no baixo do ruler (`parent.height - 8px`)
   - Posicionadas a cada 0.5 beat
6. **Subdivisões 1/16** (quando `px-per-bar >= 240px`):
   - Linhas de 5px no baixo do ruler (`parent.height - 6px`)
   - Posicionadas a cada 0.25 e 0.75 beat
7. **Linha inferior da régua:** 1px em `Theme.is-paper ? Theme.base : Theme.grid-line`
8. **Highlight do loop range** (se `loop-enabled`):
   - Retângulo transparente com borda `Theme.accent`
   - Borda grossa 1px, `border-radius: 6px`

---

### 4. Playhead (Bolinha + Linha)

Estrutura a recriar:

1. **Linha vertical:** Retângulo 2px largura, cor `Theme.playhead` (`#DC143C`)
2. **Bolinha (cabeça):**
   - Círculo 12×12px com `border-radius: 50%` (ou `6px` para 12px)
   - Cor: `Theme.playhead` (`#DC143C`)
   - Posicionada em `head-x - 6px`, `y: 1px`
3. **Posição:** Baseada no `playhead-x` (já calculado no Rust/SLint)
4. **Z-index:** Fica acima da régua, abaixo das seleções

---

### 5. Scroll Horizontal/Virtual

Comportamento a implementar no procedural:

- **2 dedos horizontal** (movimento lateral) → scroll-x
- **2 dedos vertical** (movimento vertical) → scroll-y
- **Middle-drag** → scroll ambos os eixos simultaneamente
- **Shift+roda** → scroll horizontal instantâneo
- **Ctrl+roda/arrasto 2 dedos** → zoom horizontal com âncora fixa (suave, sem drift)

#### Fórmula de scroll compensation (do `interaction.rs`):

```
new_sx = (pad + anchor_px * new_zoom - mx).clamp(0, max_sx)
```

Como `anchor_px` é fixo no ponto sob o cursor, o zoom não "imagina" — o ponto permanece onde estava.

---

### 6. Resumo do que copiar para o procedural

Se estiver criando os shaders/estruturas do zero, copie estas definições:

#### Cores (via Theme.active):

```glsl
// No Rust, expõe pra WGSL:
uniform ThemeColors {
    float4 base;
    float4 surface;
    float4 surface_high;
    float4 border;
    float4 accent;
    float4 grid_line;
    float4 grid_sub;
    float4 timeline_bg;
    float4 surface_step;
    float4 playhead;      // sempre #DC143C = 0.8596, 0.0824, 0.2980, 1.0
    bool  is_paper;       // 1.0 quando tema 6 (Paper)
};
```

#### Grid logic (por px-per-bar):

```glsl
// Se px-per-bar >= 240: desenha subdiv 1/16
// Se px-per-bar >= 120: desenha subdiv 1/8  
// Se px-per-bar >= 40:  desenha linhas de beat
// Sempre: desenha linha principal do compasso
```

#### Ruler numbers:

```glsl
// x posição: 0px (colado no grid)
// cor: text-primary / text-dim alternado por índice do compasso
// fonte tamanho: 10px / 9px
// peso: 700 / 500
```

---

### 7. Próximos passos

1. **No Rust side** (`timeline-render/crates`): garantir que `Theme.active` e todos os tokens acima sejam expostos via uniform buffer pra WGSL ler.
2. **No WGSL** (`timeline-wgsl/`): implementar o kernel de grid rendering que lê esses uniforms e desenha:
   - Grade principal a cada `px-per-bar`
   - Subdivisões baseadas no threshold (≥40, ≥120, ≥240)
   - Fundos de compasso alternados
3. **Ruler drawing:** régua fixa acima, números alinhados x=0, todas as cores vindo dos uniforms.
4. **Playhead:** círculo redondo + linha vertical, posição baseada no tempo corrente.

---

*Plano gerado a partir de requisitos: bolinha playhead redonda, scroll 2 dedos touchpad (horizontal→horizontal, vertical→vertical), números ruler colados no grid (x: 0px), cores consistentes com Theme.slint tokens.*