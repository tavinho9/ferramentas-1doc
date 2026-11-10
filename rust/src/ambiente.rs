//! O ambiente de uma execução: endereço do 1Doc, pastas, cancelamento e
//! saída das mensagens. Nos scripts Python isso era estado global
//! (`sessao.URL_1DOC`, `caminhos.pasta_dados()`, `sessao.CANCELAR`,
//! `sys.stdout`); aqui é passado explicitamente, e os testes criam o deles.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::caminhos;
use crate::cancelar::Cancelar;
use crate::erro::Res;
use crate::saida::Saida;

pub const URL_1DOC: &str = "https://montesclaros.1doc.com.br";

#[derive(Clone)]
pub struct Ambiente {
    /// Endereço do 1Doc, sem a barra final (`URL_1DOC`).
    pub url_1doc: String,
    pasta_dados: PathBuf,
    /// Área de Trabalho (None = a do sistema). Os testes usam uma pasta temporária.
    area_trabalho: Option<PathBuf>,
    pub cancelar: Arc<Cancelar>,
    pub saida: Saida,
}

impl Ambiente {
    /// O ambiente de verdade: 1Doc da prefeitura, pasta de dados do usuário.
    pub fn padrao(saida: Saida) -> Self {
        Ambiente {
            url_1doc: URL_1DOC.to_string(),
            pasta_dados: caminhos::pasta_dados_padrao(),
            area_trabalho: None,
            cancelar: Arc::new(Cancelar::new()),
            saida,
        }
    }

    /// Ambiente sob medida (testes e `--base` da busca).
    pub fn novo(
        url_1doc: &str,
        pasta_dados: PathBuf,
        area_trabalho: Option<PathBuf>,
        saida: Saida,
    ) -> Self {
        Ambiente {
            url_1doc: url_1doc.trim_end_matches('/').to_string(),
            pasta_dados,
            area_trabalho,
            cancelar: Arc::new(Cancelar::new()),
            saida,
        }
    }

    /// Mesmo ambiente com outra saída de mensagens.
    pub fn com_saida(&self, saida: Saida) -> Self {
        Ambiente {
            saida,
            ..self.clone()
        }
    }

    /// `BASE` (`URL_1DOC + "/"`).
    pub fn base(&self) -> String {
        format!("{}/", self.url_1doc)
    }

    /// Host do 1Doc (`urlparse(BASE).hostname`).
    pub fn host(&self) -> String {
        url::Url::parse(&self.base())
            .ok()
            .and_then(|u| u.host_str().map(|h| h.to_lowercase()))
            .unwrap_or_default()
    }

    /// Pasta de dados, criada se faltar.
    pub fn pasta_dados(&self) -> PathBuf {
        let _ = std::fs::create_dir_all(&self.pasta_dados);
        self.pasta_dados.clone()
    }

    /// Caminho de um arquivo dentro da pasta de dados.
    pub fn arquivo_dados(&self, nome: &str) -> PathBuf {
        self.pasta_dados().join(nome)
    }

    pub fn pasta_logs(&self) -> PathBuf {
        let p = self.pasta_dados().join("logs");
        let _ = std::fs::create_dir_all(&p);
        p
    }

    /// Perfil próprio do navegador usado no login (não mexe no perfil pessoal).
    pub fn pasta_perfil_navegador(&self) -> PathBuf {
        self.pasta_dados().join("navegador")
    }

    pub fn area_de_trabalho(&self) -> PathBuf {
        self.area_trabalho
            .clone()
            .unwrap_or_else(caminhos::area_de_trabalho)
    }

    pub fn checar_cancelamento(&self) -> Res<()> {
        self.cancelar.checar()
    }

    pub fn esperar(&self, segundos: f64) -> Res<()> {
        self.cancelar.esperar(segundos)
    }

    pub fn abrir_arquivo(&self, caminho: &Path) {
        caminhos::abrir_arquivo(&self.saida, caminho)
    }
}
