# Plugins e Extensibilidade

> Sistema de plugins do RuanaStudio (fase futura).

---

## Estado atual

**Fase futura** — placeholder. O crate `daw-plugins` está vazio.

```rust
//! daw-plugins — plugin host (VST3/CLAP, Fase futura). Placeholder.
```

## Visão planejada

- Formatos alvo: VST3 e CLAP
- Carregamento dinâmico via `libloading` (já como dependência do app)
- Gerenciamento de pacotes via `ruana-hub` (reqwest, zip, sha2)

## Dependências já disponíveis no app

| Crate | Uso |
|---|---|
| `libloading = "0.9.0"` | Carregamento dinâmico de bibliotecas |
| `reqwest = "0.12"` | Download de plugins |
| `zip = "2.4"` | Descompactação de pacotes |
| `sha2 = "0.10"` | Verificação de integridade |

## Relacionado

- [[Projeto e Persistência]]
- [[Engine de Áudio]]
