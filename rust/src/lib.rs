//! Ferramentas 1Doc — Prefeitura de Montes Claros (versão Rust).
//!
//! Os quatro scripts de consulta ao 1Doc reunidos num programa com
//! interface. O login é sempre feito pela própria pessoa, na janela do
//! navegador: o programa só espera e reaproveita a sessão.

pub const VERSAO: &str = env!("CARGO_PKG_VERSION");
pub const NOME_APP: &str = "Ferramentas 1Doc";

pub mod texto;
pub mod verificadores;
#[macro_use]
pub mod html;
pub mod saida;

pub mod ambiente;
pub mod arquivos;
pub mod busca;
pub mod busca_argv;
pub mod caminhos;
pub mod cancelar;
pub mod cli;
pub mod config;
pub mod datas;
pub mod erro;
pub mod faltantes;

pub mod http;
pub mod navegador;
pub mod paralelo;
pub mod planilha;
pub mod processos_do_dia;
pub mod sessao;

pub use ambiente::Ambiente;
pub use erro::{Erro, Res};
