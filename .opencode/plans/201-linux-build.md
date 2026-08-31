# Plano: Compilar o RuanaStudio no Linux (Fedora 44)

> Data: 2026-08-31
> Status: REVISADO pelo usuário (escopo completo + rfd)
> Modo atual: PLANO (edição de código bloqueada por permissão)

## Contexto
O app foi desenvolvido focado no Windows. A maioria do código já está
protegida com `#[cfg(windows)]`, mas há 4 pontos que bloqueiam ou degradam
a compilação/execução no Linux.

## Mudanças de código

### 1. `app/build.rs` — proteger `winresource` com `#[cfg(windows)]`
- **Problema:** linha ~58-64 chama `winresource::WindowsResource::new()` incondicionalmente.
  No Linux tenta invocar `rc.exe`/`windres` (inexistentes) — pode falhar o build.
- **Ação:** envolver o bloco de geração/embreveção do `.ico` + `winresource` em `#[cfg(windows)]`.
  O PNG 64 de runtime (usado via `icon.rs`, multiplataforma) permanece ativo nos dois SO.

### 2. `app/Cargo.toml` — mover dependências Windows para `[target.'cfg(windows)']`
- **Problema:** `windows` e `winresource` declaradas incondicionalmente.
- **Ação:**
  - Mover `windows` para `[target.'cfg(windows)'.dependencies]`
  - Mover `winresource` para `[target.'cfg(windows)'.build-dependencies]`
  - Adicionar `rfd = "0.15"` (ou versão compatível) nas dependências (multiplataforma)

### 3. `app/src/main.rs` — `choose_audio_file` no Linux usando `rfd`
- **Problema:** a versão `#[cfg(not(windows))]` (linha 84-87) retorna `None`,
  então o botão "carregar sample" do sampler não funciona no Linux.
- **Ação:** trocar a implementação não-Windows para usar `rfd::FileDialog`
  com filtro de arquivos de áudio (`wav`, `mp3`, `flac`, `ogg`, `oga`), retornando o path.

```rust
#[cfg(not(windows))]
fn choose_audio_file() -> Option<String> {
    use rfd::FileDialog;
    let file = FileDialog::new()
        .add_filter("Audio Files", &["wav", "mp3", "flac", "ogg", "oga"])
        .add_filter("All Files", &["*"])
        .pick_file();
    file.map(|p| p.to_string_lossy().into_owned())
}
```

## Dependências de sistema (dnf) — necessárias para winit/Skia/alsa compilarem

```bash
sudo dnf install \
    xorg-x11-server-devel \
    libXcursor-devel \
    libXrandr-devel \
    libX11-devel \
    libXi-devel \
    libxkbcommon-devel \
    fontconfig-devel \
    freetype-devel \
    vulkan-loader-devel \
    systemd-devel \
    wayland-devel \
    alsa-lib-devel
```

### Já presentes (confirmado via pkg-config): ALSA, xkbcommon, fontconfig, freetype, glib, x11

## Verificação

```bash
cargo build 2>&1 | head -50
cargo run   # janela deve abrir
```

## Funcionalidades que já funcionam multiplataforma (sem mudança)
- `os_drop.rs` — protegido por `#[cfg(windows)]`
- `cursors.rs` — protegido; no Linux usa cursor padrão do sistema
- `filesystem.rs` — `list_drives` já trata Unix (`/`)
- `icon.rs` — usa winit (multiplataforma)
- Áudio via `cpal`/`alsa`
- Renderer Skia do Slint (X11)
