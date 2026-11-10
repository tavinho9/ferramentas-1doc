//! Ferramentas 1Doc — linha de comando (as quatro ferramentas, sem janela).

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(ferramentas1doc::cli::main(args));
}
