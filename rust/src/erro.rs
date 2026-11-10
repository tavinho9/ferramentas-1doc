//! Erros do programa. Espelham as exceções dos scripts Python.
//!
//! `Cancelado` e `Encerrar` correspondem a `BaseException` no Python
//! (`Cancelado` e `SystemExit`): atravessam os pontos que registram a falha
//! de um processo e seguem para o próximo. Veja [`Erro::atravessa`].

use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum Erro {
    /// A pessoa pediu para parar (botão Cancelar).
    Cancelado,
    /// O login não aconteceu: janela fechada, tempo esgotado, navegador indisponível.
    LoginCancelado(String),
    /// O 1Doc não respondeu (rede, proxy, servidor fora). A sessão salva continua.
    SemConexao(String),
    /// A resposta veio com cara de tela de login.
    SessaoExpirada(String),
    /// A interface do 1Doc não foi reconhecida.
    LayoutDesconhecido(String),
    /// Fim com mensagem para a pessoa (o `SystemExit("✗ ...")` dos scripts).
    Encerrar(String),
    /// Falha de requisição HTTP (status >= 400, conexão, tempo esgotado).
    Http { msg: String, status: Option<u16> },
    /// O navegador não abriu, fechou ou não respondeu.
    Navegador(String),
    /// O navegador não respondeu a tempo, mas a conexão continua de pé.
    SemResposta(String),
    /// Qualquer outra falha (arquivo, planilha, JSON...).
    Outro(String),
}

pub type Res<T> = Result<T, Erro>;

impl Erro {
    /// True para os erros que, no Python, são `BaseException` e não são
    /// capturados por `except Exception`.
    pub fn atravessa(&self) -> bool {
        matches!(self, Erro::Cancelado | Erro::Encerrar(_))
    }

    /// True para `ErroNavegador` e suas subclasses.
    pub fn eh_navegador(&self) -> bool {
        matches!(self, Erro::Navegador(_) | Erro::SemResposta(_))
    }

    /// True para `requests.RequestException`.
    pub fn eh_http(&self) -> bool {
        matches!(self, Erro::Http { .. })
    }

    pub fn outro(msg: impl Into<String>) -> Self {
        Erro::Outro(msg.into())
    }

    pub fn encerrar(msg: impl Into<String>) -> Self {
        Erro::Encerrar(msg.into())
    }

    /// Nome curto do tipo, como `type(e).__name__` nas mensagens dos scripts.
    pub fn nome_tipo(&self) -> &'static str {
        match self {
            Erro::Cancelado => "Cancelado",
            Erro::LoginCancelado(_) => "LoginCancelado",
            Erro::SemConexao(_) => "SemConexao",
            Erro::SessaoExpirada(_) => "SessaoExpirada",
            Erro::LayoutDesconhecido(_) => "LayoutDesconhecido",
            Erro::Encerrar(_) => "SystemExit",
            Erro::Http {
                status: Some(_), ..
            } => "HTTPError",
            Erro::Http { .. } => "ConnectionError",
            Erro::Navegador(_) => "ErroNavegador",
            Erro::SemResposta(_) => "_SemResposta",
            Erro::Outro(_) => "Erro",
        }
    }
}

impl fmt::Display for Erro {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Erro::Cancelado => write!(f, "cancelado"),
            Erro::LoginCancelado(m)
            | Erro::SemConexao(m)
            | Erro::SessaoExpirada(m)
            | Erro::LayoutDesconhecido(m)
            | Erro::Encerrar(m)
            | Erro::Navegador(m)
            | Erro::SemResposta(m)
            | Erro::Outro(m) => write!(f, "{m}"),
            Erro::Http { msg, .. } => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for Erro {}

impl From<std::io::Error> for Erro {
    fn from(e: std::io::Error) -> Self {
        Erro::Outro(e.to_string())
    }
}

impl From<serde_json::Error> for Erro {
    fn from(e: serde_json::Error) -> Self {
        Erro::Outro(format!("JSON inválido: {e}"))
    }
}

/// Primeira linha de uma mensagem, cortada em `n` caracteres
/// (`str(e).splitlines()[0][:n]`).
pub fn primeira_linha(texto: &str, n: usize) -> String {
    corta(texto.lines().next().unwrap_or(""), n)
}

/// Os primeiros `n` caracteres (não bytes) de `texto` (`texto[:n]`).
pub fn corta(texto: &str, n: usize) -> String {
    texto.chars().take(n).collect()
}
