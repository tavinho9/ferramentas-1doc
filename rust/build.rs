//! No Windows: ícone e informações de versão dentro do .exe.

fn main() {
    println!("cargo:rerun-if-changed=../ferramentas1doc/gui/icone.ico");
    #[cfg(windows)]
    {
        if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
            let mut r = winresource::WindowsResource::new();
            r.set_icon("../ferramentas1doc/gui/icone.ico")
                .set("ProductName", "Ferramentas 1Doc")
                .set(
                    "FileDescription",
                    "Ferramentas 1Doc — Prefeitura de Montes Claros",
                )
                .set("CompanyName", "Uso interno — Prefeitura de Montes Claros")
                .set(
                    "LegalCopyright",
                    "Uso interno — Prefeitura de Montes Claros",
                );
            if let Err(e) = r.compile() {
                println!("cargo:warning=ícone do .exe não embutido: {e}");
            }
        }
    }
}
