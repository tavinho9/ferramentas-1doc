//! Linha de comando: `ferramentas1doc-cli <ferramenta> [argumentos]`, os
//! mesmos argumentos de `python -m ferramentas1doc.<ferramenta>`.

use std::io::{BufRead, Write};

use crate::ambiente::Ambiente;
use crate::erro::{Erro, Res};
use crate::saida::Saida;
use crate::{datas, processos_do_dia, sessao};

pub const AJUDA: &str = "Ferramentas 1Doc — linha de comando

Uso:
  ferramentas1doc-cli processos-do-dia [DIA]
  ferramentas1doc-cli busca [opções]            (busca --help mostra todas)
  ferramentas1doc-cli faltantes [PLANILHA] [--aba ABA] [--de DATA] [--ate DATA]
  ferramentas1doc-cli verificar-planilha [PLANILHA] [--aba ABA] [--html PASTA]

O login é sempre feito por você, na janela do navegador: o programa só espera.";

/// `input(pergunta)`.
pub fn perguntar(pergunta: &str) -> String {
    print!("{pergunta}");
    let _ = std::io::stdout().flush();
    let mut linha = String::new();
    let _ = std::io::stdin().lock().read_line(&mut linha);
    linha.trim().to_string()
}

/// Pergunta o dia de emissão no terminal até receber uma data válida.
fn pedir_data(hoje: chrono::NaiveDate) -> chrono::NaiveDate {
    loop {
        let txt = perguntar(&format!(
            "Dia de emissão dos processos [Enter = hoje, {}]: ",
            datas::br(hoje)
        ))
        .to_lowercase();
        match processos_do_dia::ler_dia(&txt, hoje) {
            None => {
                println!("  ✗ Data não reconhecida. Use DD/MM/AAAA, DD/MM/AA, DD/MM ou 'ontem'.")
            }
            Some(d) if d > hoje => println!("  ✗ {} ainda não chegou.", datas::br(d)),
            Some(d) => return d,
        }
    }
}

fn processos_do_dia_cli(amb: &Ambiente, args: &[String]) -> Res<()> {
    let hoje = datas::hoje();
    let alvo = match args.first() {
        Some(a) => processos_do_dia::ler_dia(a, hoje)
            .ok_or_else(|| Erro::encerrar(format!("✗ Data não reconhecida: '{a}'")))?,
        None => pedir_data(hoje),
    };
    let cred = sessao::obter_sessao(amb, false, None, None, None)?;
    processos_do_dia::executar(
        amb,
        alvo,
        &cred,
        processos_do_dia::Opcoes {
            hoje: Some(hoje),
            ..Default::default()
        },
    )?;
    Ok(())
}

#[derive(clap::Parser)]
#[command(
    name = "faltantes",
    about = "Processos que estão no 1Doc antes da Pesquisa de Mercado e faltam na planilha de acompanhamento"
)]
struct ArgsFaltantes {
    planilha: Option<String>,
    /// nome da aba (padrão: a ativa)
    #[arg(long)]
    aba: Option<String>,
    /// só processos emitidos a partir desta data (DD/MM/AAAA, DD/MM/AA, DD/MM, hoje, ontem ou -N); padrão: 1º/jan do ano mais antigo da planilha
    #[arg(long, value_name = "DATA", allow_hyphen_values = true)]
    de: Option<String>,
    /// só processos emitidos até esta data; padrão: hoje
    #[arg(long, value_name = "DATA", allow_hyphen_values = true)]
    ate: Option<String>,
}

#[derive(clap::Parser)]
#[command(
    name = "verificar-planilha",
    about = "Verifica se os processos de uma planilha já chegaram à Pesquisa de Mercado no 1Doc"
)]
struct ArgsVerificar {
    planilha: Option<String>,
    /// nome da aba (padrão: a ativa)
    #[arg(long)]
    aba: Option<String>,
    /// salva cada página de processo baixada nesta pasta (diagnóstico)
    #[arg(long, value_name = "PASTA")]
    html: Option<String>,
}

fn verificar_cli(amb: &Ambiente, args: &[String]) -> Res<()> {
    let Some(a) = analisar::<ArgsVerificar>("verificar-planilha", args)? else {
        return Ok(());
    };
    let caminho = a.planilha.unwrap_or_else(escolher_arquivo);
    let cred = sessao::obter_sessao(amb, false, None, None, None)?;
    let op = crate::verificar_planilha::Opcoes {
        pasta_html: a.html.map(Into::into),
        ..Default::default()
    };
    crate::verificar_planilha::executar(
        amb,
        std::path::Path::new(&caminho),
        a.aba.as_deref(),
        &cred,
        op,
    )?;
    Ok(())
}

/// Planilha pedida no terminal (sem janela de seleção na linha de comando).
fn escolher_arquivo() -> String {
    perguntar("Caminho da planilha .xlsx: ")
        .trim_matches('"')
        .to_string()
}

fn analisar<T: clap::Parser>(nome: &str, args: &[String]) -> Res<Option<T>> {
    let mut v = vec![nome.to_string()];
    v.extend(args.iter().cloned());
    match T::try_parse_from(v) {
        Ok(a) => Ok(Some(a)),
        Err(e) if matches!(e.kind(), clap::error::ErrorKind::DisplayHelp) => {
            println!("{e}");
            Ok(None)
        }
        Err(e) => Err(Erro::encerrar(
            e.to_string()
                .lines()
                .next()
                .unwrap_or("argumentos inválidos")
                .to_string(),
        )),
    }
}

fn faltantes_cli(amb: &Ambiente, args: &[String]) -> Res<()> {
    let Some(a) = analisar::<ArgsFaltantes>("faltantes", args)? else {
        return Ok(());
    };
    let (de, ate) = datas::ler_periodo(
        a.de.as_deref().unwrap_or(""),
        a.ate.as_deref().unwrap_or(""),
        datas::hoje(),
    )
    .map_err(|e| Erro::encerrar(format!("faltantes: error: {e}")))?;
    let caminho = a.planilha.unwrap_or_else(escolher_arquivo);
    let cred = sessao::obter_sessao(amb, false, None, None, None)?;
    let op = crate::faltantes::Opcoes {
        de,
        ate,
        ..Default::default()
    };
    crate::faltantes::executar(
        amb,
        std::path::Path::new(&caminho),
        a.aba.as_deref(),
        &cred,
        op,
    )?;
    Ok(())
}

/// Ponto de entrada do programa de linha de comando. Devolve o código de saída.
pub fn main(args: Vec<String>) -> i32 {
    let amb = Ambiente::padrao(Saida::stdout());
    let Some(ferramenta) = args.first() else {
        println!("{AJUDA}");
        return 2;
    };
    let resto = &args[1..];
    let r = match ferramenta.replace('_', "-").as_str() {
        "processos-do-dia" => processos_do_dia_cli(&amb, resto),
        "busca" => crate::busca::main(&amb, resto, None).map(|_| ()),
        "faltantes" => faltantes_cli(&amb, resto),
        "verificar-planilha" | "verificar" => verificar_cli(&amb, resto),
        "-h" | "--help" | "ajuda" => {
            println!("{AJUDA}");
            Ok(())
        }
        outro => {
            eprintln!("Ferramenta desconhecida: {outro}\n\n{AJUDA}");
            return 2;
        }
    };
    match r {
        Ok(()) => 0,
        Err(Erro::Encerrar(m)) => {
            eprintln!("{m}");
            1
        }
        Err(Erro::Cancelado) => {
            eprintln!("\n⏹ Cancelado.");
            130
        }
        Err(Erro::LoginCancelado(m)) => {
            eprintln!("\n✗ Login não concluído: {m}");
            1
        }
        Err(e) => {
            eprintln!("\n✗ Erro: {e}");
            1
        }
    }
}
