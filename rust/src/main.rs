//! Ferramentas 1Doc — a janela do programa.
//!
//! ```text
//!     Ferramentas1Doc.exe                    abre o programa
//!     Ferramentas1Doc.exe --autoteste        confere o pacote e sai (0 = tudo certo)
//!         [--relatorio ARQ] [--sem-navegador] [--sem-janela]
//! ```
// Sem console no Windows (o programa de linha de comando é o ferramentas1doc-cli).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--autoteste") {
        let relatorio = args
            .iter()
            .position(|a| a == "--relatorio")
            .and_then(|i| args.get(i + 1))
            .map(String::as_str);
        let navegador = !args.iter().any(|a| a == "--sem-navegador");
        let janela = !args.iter().any(|a| a == "--sem-janela");
        std::process::exit(ferramentas1doc::autoteste::rodar(
            relatorio, navegador, janela,
        ));
    }
    if let Err(e) = ferramentas1doc::gui::rodar() {
        eprintln!("Não consegui abrir a janela: {e}");
        std::process::exit(1);
    }
}
