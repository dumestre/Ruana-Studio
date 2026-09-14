//! Stub de entrada: a chamada real vai para a biblioteca, porque em
//! Makepad o `app_main!` precisa expandir a porta de entrada por plataforma.

fn main() {
    ruana_studio_makepad::app::app_main()
}