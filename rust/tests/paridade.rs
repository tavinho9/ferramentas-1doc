//! Paridade Python × Rust, lado Rust (ver paridade/gerar_python.py).
//! Roda só com `PARIDADE_PASTA` (a pasta em que o lado Python gravou as
//! entradas e as saídas dele); grava as saídas do Rust em PASTA/rust.

mod comum;

use std::path::{Path, PathBuf};

use ferramentas1doc::{busca, busca_argv, faltantes, processos_do_dia, verificar_planilha};

fn guardar(origem: Option<PathBuf>, destino: &Path) {
    if let Some(o) = origem {
        std::fs::copy(o, destino).unwrap();
    }
}

#[test]
fn paridade() {
    let Some(pasta) = std::env::var_os("PARIDADE_PASTA").map(PathBuf::from) else {
        eprintln!("pulado: sem PARIDADE_PASTA");
        return;
    };
    let entrada = pasta.join("entrada");
    let saida = pasta.join("rust");
    let _ = std::fs::remove_dir_all(&saida);
    std::fs::create_dir_all(&saida).unwrap();
    let fake = comum::Fake1Doc::novo();
    let d = comum::dados(&fake.base);
    let cred = comum::cred();

    let r = processos_do_dia::executar(
        &d.amb,
        comum::hoje(),
        &cred,
        processos_do_dia::Opcoes {
            pausa: 0.0,
            ..Default::default()
        },
    )
    .unwrap();
    guardar(r, &saida.join("processos_do_dia.xlsx"));

    let form = busca_argv::Formulario {
        caixas: vec!["entrada".into()],
        ..Default::default()
    }
    .com("etapa", "Pesquisa de Mercado");
    let mut argv =
        busca_argv::montar_argv(&form, None, chrono::Local::now().naive_local(), false).unwrap();
    argv.extend(["--pausa".into(), "0".into()]);
    guardar(
        busca::main(&d.amb, &argv, Some(&cred))
            .unwrap()
            .unwrap()
            .arquivo,
        &saida.join("busca.xlsx"),
    );

    let op = || faltantes::Opcoes {
        pausa: 0.0,
        sem_navegador: true,
        ..Default::default()
    };
    guardar(
        faltantes::executar(&d.amb, &entrada.join("acomp.xlsx"), None, &cred, op()).unwrap(),
        &saida.join("faltantes.xlsx"),
    );

    let op = || verificar_planilha::Opcoes {
        pausa: 0.0,
        sem_navegador: true,
        ..Default::default()
    };
    let r = verificar_planilha::executar(
        &d.amb,
        &entrada.join("verificar.xlsx"),
        Some("Planilha1"),
        &cred,
        op(),
    )
    .unwrap();
    guardar(r, &saida.join("verificar.xlsx"));
    let r = verificar_planilha::executar(
        &d.amb,
        &entrada.join("sem_cabecalho.xlsx"),
        None,
        &cred,
        op(),
    )
    .unwrap();
    guardar(r, &saida.join("sem_cabecalho.xlsx"));
    println!("{}", d.saida_texto());
}
