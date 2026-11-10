//! Chrome ou Edge já instalados, controlados pelo protocolo do navegador (CDP).
//!
//! O navegador é aberto com `--remote-debugging-port=0` num perfil próprio;
//! ele escolhe uma porta livre em 127.0.0.1 e a grava no arquivo
//! `DevToolsActivePort` do perfil, e o programa conversa com ele por
//! WebSocket. Não há driver (chromedriver/msedgedriver) e nada é baixado.
//!
//! O perfil próprio é obrigatório: a partir do Chrome 136 a porta de
//! depuração é ignorada no perfil padrão da pessoa. Sem perfil informado,
//! usa-se uma pasta temporária, apagada ao fechar.

use std::collections::HashMap;
use std::io::ErrorKind;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};

use crate::erro::{Erro, Res, corta};

pub const NOMES: [(&str, &str); 2] = [("chrome", "Google Chrome"), ("edge", "Microsoft Edge")];

pub fn nome(qual: &str) -> String {
    NOMES
        .iter()
        .find(|(k, _)| *k == qual)
        .map(|(_, v)| v.to_string())
        .unwrap_or_else(|| qual.to_string())
}

/// Uma aba aberta.
#[derive(Clone, Debug, PartialEq)]
pub struct Pagina {
    pub id: String,
    pub url: String,
}

/// O que o programa faz com um navegador. A implementação real é
/// [`NavegadorCdp`]; os testes usam navegadores falsos.
pub trait Navegador: Send {
    /// Abre `url` na aba (não espera carregar).
    fn ir(&mut self, url: &str, alvo: Option<&str>) -> Res<()>;
    /// As abas abertas.
    fn paginas(&mut self) -> Res<Vec<Pagina>>;
    /// A aba de trabalho: a mesma enquanto existir; senão a primeira; senão uma nova.
    fn pagina(&mut self) -> Res<String>;
    fn url(&mut self, alvo: Option<&str>) -> Res<String>;
    /// Valor (JSON) de uma expressão JavaScript na aba.
    fn avaliar(&mut self, alvo: &str, expressao: &str) -> Res<Value>;
    fn versao(&mut self) -> Res<String>;
    /// False se a pessoa fechou todas as janelas (ou o navegador).
    fn viva(&mut self) -> bool;
    /// Cookies (inclusive HttpOnly) no formato do sessao.json; com `host`,
    /// só os que o navegador mandaria para ele.
    fn cookies(&mut self, host: Option<&str>) -> Res<Vec<Value>>;
    /// Grava os cookies no navegador, um a um. Devolve quantos foram aceitos.
    fn definir_cookies(&mut self, cookies: &[Value]) -> usize;
    fn fechar(&mut self);
}

// ============================================================
// Onde está o navegador
// ============================================================

#[cfg(windows)]
fn app_paths(exe: &str) -> Vec<PathBuf> {
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    let mut achados = Vec::new();
    for raiz in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        let chave = format!(r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\{exe}");
        if let Ok(k) = RegKey::predef(raiz).open_subkey(chave)
            && let Ok(v) = k.get_value::<String, _>("")
            && !v.is_empty()
        {
            achados.push(PathBuf::from(crate::caminhos::expandir_variaveis(
                v.trim_matches('"'),
            )));
        }
    }
    achados
}

fn no_path(nome: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|p| p.join(nome))
        .find(|p| p.is_file())
}

/// Caminhos possíveis do executável, na ordem em que são tentados.
pub fn candidatos(qual: &str) -> Vec<PathBuf> {
    if qual == "chrome"
        && let Some(c) = std::env::var_os("FERRAMENTAS1DOC_CHROME").filter(|c| !c.is_empty())
    {
        return vec![PathBuf::from(c)]; // só para desenvolvimento e testes
    }
    #[cfg(windows)]
    {
        let exe = if qual == "chrome" {
            "chrome.exe"
        } else {
            "msedge.exe"
        };
        let pasta: [&str; 2] = if qual == "chrome" {
            ["Google", "Chrome"]
        } else {
            ["Microsoft", "Edge"]
        };
        let mut lista = app_paths(exe);
        for var in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
            if let Some(base) = std::env::var_os(var) {
                lista.push(
                    PathBuf::from(base)
                        .join(pasta[0])
                        .join(pasta[1])
                        .join("Application")
                        .join(exe),
                );
            }
        }
        lista
    }
    #[cfg(target_os = "macos")]
    {
        vec![PathBuf::from(if qual == "chrome" {
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
        } else {
            "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge"
        })]
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let nomes: &[&str] = if qual == "chrome" {
            &[
                "google-chrome",
                "google-chrome-stable",
                "chromium",
                "chromium-browser",
            ]
        } else {
            &["microsoft-edge", "microsoft-edge-stable"]
        };
        nomes.iter().filter_map(|n| no_path(n)).collect()
    }
}

/// Caminho do chrome.exe/msedge.exe instalado, ou None.
pub fn localizar_executavel(qual: &str) -> Option<PathBuf> {
    let _ = no_path; // usado só fora do Windows/macOS
    candidatos(qual).into_iter().find(|c| c.is_file())
}

// ============================================================
// Transporte (WebSocket do navegador)
// ============================================================

/// O WebSocket do navegador (trocável nos testes).
pub trait Transporte: Send {
    fn definir_timeout(&mut self, t: Duration);
    fn enviar(&mut self, texto: &str) -> Result<(), ErroTransporte>;
    fn receber(&mut self) -> Result<String, ErroTransporte>;
    fn fechar(&mut self);
}

#[derive(Debug)]
pub enum ErroTransporte {
    Timeout,
    Caiu(String),
}

struct WsReal {
    ws: tungstenite::WebSocket<TcpStream>,
}

impl Transporte for WsReal {
    fn definir_timeout(&mut self, t: Duration) {
        let _ = self.ws.get_ref().set_read_timeout(Some(t));
        let _ = self.ws.get_ref().set_write_timeout(Some(t));
    }

    fn enviar(&mut self, texto: &str) -> Result<(), ErroTransporte> {
        self.ws
            .send(tungstenite::Message::text(texto))
            .map_err(erro_ws)
    }

    fn receber(&mut self) -> Result<String, ErroTransporte> {
        loop {
            match self.ws.read().map_err(erro_ws)? {
                tungstenite::Message::Text(t) => return Ok(t.to_string()),
                tungstenite::Message::Binary(b) => {
                    return Ok(String::from_utf8_lossy(&b).into_owned());
                }
                tungstenite::Message::Close(_) => {
                    return Err(ErroTransporte::Caiu("ConnectionClosed".into()));
                }
                _ => continue,
            }
        }
    }

    fn fechar(&mut self) {
        let _ = self.ws.close(None);
        let _ = self.ws.flush();
    }
}

fn erro_ws(e: tungstenite::Error) -> ErroTransporte {
    match e {
        tungstenite::Error::Io(ref io)
            if matches!(io.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) =>
        {
            ErroTransporte::Timeout
        }
        outro => ErroTransporte::Caiu(
            format!("{outro:?}")
                .split('(')
                .next()
                .unwrap_or("Erro")
                .to_string(),
        ),
    }
}

// ============================================================
// Navegador por CDP
// ============================================================

/// Um navegador aberto pelo programa, falando CDP por um único WebSocket
/// (o do próprio navegador). As páginas são acessadas por sessões
/// "flatten" desse WebSocket.
pub struct NavegadorCdp {
    processo: Option<Child>,
    pub qual: String,
    ws: Box<dyn Transporte>,
    perfil_temporario: Option<PathBuf>,
    id: u64,
    sessoes: HashMap<String, String>,
    pagina_atual: Option<String>,
    caiu: bool,
    fechado: bool,
}

impl NavegadorCdp {
    /// Navegador sobre um transporte qualquer (testes).
    pub fn com_transporte(ws: Box<dyn Transporte>, qual: &str) -> Self {
        NavegadorCdp {
            processo: None,
            qual: qual.to_string(),
            ws,
            perfil_temporario: None,
            id: 0,
            sessoes: HashMap::new(),
            pagina_atual: None,
            caiu: false,
            fechado: false,
        }
    }

    pub fn abrir(
        qual: &str,
        perfil: Option<&Path>,
        headless: bool,
        url: &str,
        espera: f64,
    ) -> Res<Self> {
        let exe = localizar_executavel(qual).ok_or_else(|| {
            Erro::Navegador(format!("{} não encontrado neste computador", nome(qual)))
        })?;
        let (perfil, temporario) = match perfil {
            Some(p) => {
                std::fs::create_dir_all(p).map_err(|e| Erro::Navegador(e.to_string()))?;
                (p.to_path_buf(), None)
            }
            None => {
                let t = tempfile::Builder::new()
                    .prefix("ferramentas1doc-navegador-")
                    .tempdir()
                    .map_err(|e| Erro::Navegador(e.to_string()))?
                    .keep();
                (t.clone(), Some(t))
            }
        };
        let porta_arq = perfil.join("DevToolsActivePort");
        match std::fs::remove_file(&porta_arq) {
            Ok(_) => {}
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => {
                return Err(Erro::Navegador(format!(
                    "o perfil do navegador está em uso ({e})"
                )));
            }
        }
        let mut args: Vec<String> = vec![
            "--remote-debugging-port=0".into(),
            format!("--user-data-dir={}", perfil.display()),
            "--no-first-run".into(),
            "--no-default-browser-check".into(),
        ];
        if headless {
            args.push("--headless=new".into());
            args.push("--window-size=1920,1080".into());
        }
        if qual == "chrome"
            && std::env::var_os("FERRAMENTAS1DOC_CHROME").is_some_and(|c| !c.is_empty())
        {
            args.push("--no-sandbox".into());
        }
        args.push(url.into());
        let proc = Command::new(&exe)
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        let mut proc = match proc {
            Ok(p) => p,
            Err(e) => {
                apagar(temporario.as_deref());
                return Err(Erro::Navegador(format!(
                    "não consegui iniciar {} ({e})",
                    exe.display()
                )));
            }
        };
        match conectar(&mut proc, &porta_arq, espera) {
            Ok(ws) => {
                let mut nav = NavegadorCdp::com_transporte(Box::new(ws), qual);
                nav.processo = Some(proc);
                nav.perfil_temporario = temporario;
                Ok(nav)
            }
            Err(e) => {
                encerrar(&mut proc, 0.0);
                apagar(temporario.as_deref());
                Err(e)
            }
        }
    }

    fn enviar(
        &mut self,
        metodo: &str,
        params: Value,
        sessao: Option<&str>,
        timeout: f64,
    ) -> Res<Map<String, Value>> {
        if self.caiu || self.fechado {
            return Err(Erro::Navegador("o navegador foi fechado".into()));
        }
        self.id += 1;
        let mut pedido = json!({"id": self.id, "method": metodo, "params": params});
        if let Some(s) = sessao {
            pedido["sessionId"] = json!(s);
        }
        self.ws.definir_timeout(Duration::from_secs_f64(timeout));
        let resposta: Value = (|| {
            self.ws.enviar(&pedido.to_string())?;
            loop {
                let texto = self.ws.receber()?;
                let r: Value = serde_json::from_str(&texto)
                    .map_err(|_| ErroTransporte::Caiu("ValueError".into()))?;
                if r.get("id").and_then(Value::as_u64) == Some(self.id) {
                    return Ok(r); // o resto são eventos ou respostas atrasadas: ignorados
                }
            }
        })()
        .map_err(|e| match e {
            ErroTransporte::Timeout => {
                Erro::SemResposta(format!("o navegador não respondeu a {metodo}"))
            }
            ErroTransporte::Caiu(tipo) => {
                self.caiu = true;
                Erro::Navegador(format!("conexão com o navegador perdida ({tipo})"))
            }
        })?;
        if let Some(erro) = resposta.get("error") {
            let msg = erro
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| erro.to_string());
            return Err(Erro::Navegador(format!("{metodo}: {msg}")));
        }
        Ok(resposta
            .get("result")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default())
    }

    fn sessao(&mut self, alvo: &str) -> Res<String> {
        if let Some(s) = self.sessoes.get(alvo) {
            return Ok(s.clone());
        }
        let r = self.enviar(
            "Target.attachToTarget",
            json!({"targetId": alvo, "flatten": true}),
            None,
            15.0,
        )?;
        let s = r
            .get("sessionId")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        self.sessoes.insert(alvo.to_string(), s.clone());
        Ok(s)
    }

    fn na_pagina(
        &mut self,
        alvo: &str,
        metodo: &str,
        params: Value,
        timeout: f64,
    ) -> Res<Map<String, Value>> {
        let r = self
            .sessao(alvo)
            .and_then(|s| self.enviar(metodo, params, Some(&s), timeout));
        if r.is_err() {
            self.sessoes.remove(alvo); // a página pode ter fechado; na próxima, conecta de novo
        }
        r
    }

    fn processo_saiu(&mut self) -> bool {
        match self.processo.as_mut() {
            Some(p) => !matches!(p.try_wait(), Ok(None)),
            None => false,
        }
    }

    pub fn avaliar_com_timeout(&mut self, alvo: &str, expressao: &str, timeout: f64) -> Res<Value> {
        let r = self.na_pagina(
            alvo,
            "Runtime.evaluate",
            json!({"expression": expressao, "returnByValue": true, "awaitPromise": true}),
            timeout,
        )?;
        if let Some(d) = r.get("exceptionDetails") {
            let desc = d
                .get("exception")
                .and_then(|e| e.get("description"))
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| {
                    d.get("text")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string()
                });
            return Err(Erro::Navegador(format!(
                "erro no script: {}",
                corta(&desc, 160)
            )));
        }
        Ok(r.get("result")
            .and_then(|x| x.get("value"))
            .cloned()
            .unwrap_or(Value::Null))
    }
}

impl Navegador for NavegadorCdp {
    fn ir(&mut self, url: &str, alvo: Option<&str>) -> Res<()> {
        let alvo = match alvo {
            Some(a) => a.to_string(),
            None => self.pagina()?,
        };
        let r = self.na_pagina(&alvo, "Page.navigate", json!({"url": url}), 15.0)?;
        if let Some(e) = r
            .get("errorText")
            .and_then(Value::as_str)
            .filter(|e| !e.is_empty())
        {
            return Err(Erro::Navegador(format!("não abriu {url} ({e})")));
        }
        Ok(())
    }

    fn paginas(&mut self) -> Res<Vec<Pagina>> {
        let r = self.enviar("Target.getTargets", json!({}), None, 15.0)?;
        Ok(r.get("targetInfos")
            .and_then(Value::as_array)
            .map(|v| {
                v.iter()
                    .filter(|t| t.get("type").and_then(Value::as_str) == Some("page"))
                    .map(|t| Pagina {
                        id: t
                            .get("targetId")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        url: t
                            .get("url")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    fn pagina(&mut self) -> Res<String> {
        let ids: Vec<String> = self.paginas()?.into_iter().map(|p| p.id).collect();
        if self.pagina_atual.as_ref().is_none_or(|p| !ids.contains(p)) {
            self.pagina_atual = Some(match ids.first() {
                Some(id) => id.clone(),
                None => {
                    let r = self.enviar(
                        "Target.createTarget",
                        json!({"url": "about:blank"}),
                        None,
                        15.0,
                    )?;
                    r.get("targetId")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string()
                }
            });
        }
        Ok(self.pagina_atual.clone().unwrap_or_default())
    }

    fn url(&mut self, alvo: Option<&str>) -> Res<String> {
        let alvo = match alvo {
            Some(a) => a.to_string(),
            None => self.pagina()?,
        };
        let r = self.enviar(
            "Target.getTargetInfo",
            json!({"targetId": alvo}),
            None,
            15.0,
        )?;
        Ok(r.get("targetInfo")
            .and_then(|t| t.get("url"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string())
    }

    fn avaliar(&mut self, alvo: &str, expressao: &str) -> Res<Value> {
        self.avaliar_com_timeout(alvo, expressao, 15.0)
    }

    fn versao(&mut self) -> Res<String> {
        let r = self.enviar("Browser.getVersion", json!({}), None, 15.0)?;
        Ok(r.get("product")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string())
    }

    fn viva(&mut self) -> bool {
        if self.caiu || self.fechado || self.processo_saiu() {
            return false;
        }
        match self.paginas() {
            Ok(p) => !p.is_empty(),
            Err(Erro::SemResposta(_)) => true, // lento, mas aberto
            Err(_) => false,
        }
    }

    fn cookies(&mut self, host: Option<&str>) -> Res<Vec<Value>> {
        let r = self.enviar("Storage.getCookies", json!({}), None, 15.0)?;
        let mut lista = Vec::new();
        for c in r
            .get("cookies")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
        {
            let dominio = c
                .get("domain")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if let Some(h) = host
                && !dominio_vale(&dominio, h)
            {
                continue;
            }
            let mut d = Map::new();
            d.insert(
                "name".into(),
                json!(c.get("name").and_then(Value::as_str).unwrap_or("")),
            );
            d.insert(
                "value".into(),
                json!(c.get("value").and_then(Value::as_str).unwrap_or("")),
            );
            d.insert("domain".into(), json!(dominio));
            let caminho = c
                .get("path")
                .and_then(Value::as_str)
                .filter(|p| !p.is_empty())
                .unwrap_or("/");
            d.insert("path".into(), json!(caminho));
            d.insert(
                "secure".into(),
                json!(c.get("secure").is_some_and(crate::arquivos::verdadeiro)),
            );
            d.insert(
                "httpOnly".into(),
                json!(c.get("httpOnly").is_some_and(crate::arquivos::verdadeiro)),
            );
            if let Some(s) = c.get("sameSite").filter(|s| crate::arquivos::verdadeiro(s)) {
                d.insert("sameSite".into(), s.clone());
            }
            let sessao = c.get("session").is_some_and(crate::arquivos::verdadeiro);
            if !sessao
                && let Some(expira) = c.get("expires").and_then(Value::as_f64)
                && expira > 0.0
            {
                d.insert("expiry".into(), json!(expira as i64));
            }
            lista.push(Value::Object(d));
        }
        Ok(lista)
    }

    fn definir_cookies(&mut self, cookies: &[Value]) -> usize {
        let mut aceitos = 0;
        for c in cookies {
            let Some(p) = cookie_cdp(c) else { continue };
            if self
                .enviar("Storage.setCookies", json!({"cookies": [p]}), None, 15.0)
                .is_ok()
            {
                aceitos += 1;
            }
        }
        aceitos
    }

    fn fechar(&mut self) {
        if self.fechado {
            return;
        }
        if !self.caiu {
            let _ = self.enviar("Browser.close", json!({}), None, 5.0); // normal: o navegador fecha a conexão
        }
        self.fechado = true;
        self.ws.fechar();
        if let Some(mut p) = self.processo.take() {
            encerrar(&mut p, 10.0);
        }
        apagar(self.perfil_temporario.as_deref());
    }
}

impl Drop for NavegadorCdp {
    fn drop(&mut self) {
        if self.processo.is_some() {
            self.fechar();
        }
    }
}

/// True se um cookie do `dominio` é enviado ao `host`.
pub fn dominio_vale(dominio: &str, host: &str) -> bool {
    let d = dominio.trim_start_matches('.').to_lowercase();
    let host = host.to_lowercase();
    !d.is_empty() && (host == d || host.ends_with(&format!(".{d}")))
}

/// Cookie do sessao.json → parâmetro do `Storage.setCookies`. Cookie de
/// domínio (".x.com") continua de domínio; os demais ficam só para o host.
pub fn cookie_cdp(c: &Value) -> Option<Value> {
    let dominio = c.get("domain").and_then(Value::as_str).unwrap_or("");
    let host = dominio.trim_start_matches('.');
    let caminho = c
        .get("path")
        .and_then(Value::as_str)
        .filter(|p| !p.is_empty())
        .unwrap_or("/");
    let seguro = c.get("secure").is_some_and(crate::arquivos::verdadeiro);
    let mut p = Map::new();
    p.insert("name".into(), c.get("name")?.clone());
    p.insert("value".into(), c.get("value")?.clone());
    p.insert("path".into(), json!(caminho));
    p.insert(
        "url".into(),
        json!(format!(
            "{}://{host}{caminho}",
            if seguro { "https" } else { "http" }
        )),
    );
    if dominio.starts_with('.') {
        p.insert("domain".into(), json!(dominio));
    }
    if seguro {
        p.insert("secure".into(), json!(true));
    }
    if c.get("httpOnly").is_some_and(crate::arquivos::verdadeiro) {
        p.insert("httpOnly".into(), json!(true));
    }
    if let Some(s) = c
        .get("sameSite")
        .and_then(Value::as_str)
        .filter(|s| ["Strict", "Lax", "None"].contains(s))
    {
        p.insert("sameSite".into(), json!(s));
    }
    if let Some(e) = c
        .get("expiry")
        .filter(|e| crate::arquivos::verdadeiro(e))
        .and_then(Value::as_f64)
    {
        p.insert("expires".into(), json!(e));
    }
    Some(Value::Object(p))
}

/// Espera o navegador gravar DevToolsActivePort e abre o WebSocket.
fn conectar(proc: &mut Child, porta_arq: &Path, espera: f64) -> Res<WsReal> {
    let fim = Instant::now() + Duration::from_secs_f64(espera);
    let linhas: Vec<String> = loop {
        let linhas: Vec<String> = std::fs::read_to_string(porta_arq)
            .map(|t| t.split_whitespace().map(str::to_string).collect())
            .unwrap_or_default();
        if linhas.len() >= 2
            && !linhas[0].is_empty()
            && linhas[0].chars().all(|c| c.is_ascii_digit())
        {
            break linhas;
        }
        if let Ok(Some(st)) = proc.try_wait() {
            // Sai na hora quando o perfil já está aberto noutra janela (a
            // instância antiga recebe o pedido e esta encerra).
            let codigo = st
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "?".into());
            return Err(Erro::Navegador(format!(
                "o navegador fechou ao abrir (código {codigo}); o perfil pode estar em uso por outra janela"
            )));
        }
        if Instant::now() > fim {
            return Err(Erro::Navegador(format!(
                "o navegador não ativou a porta de controle em {espera} s"
            )));
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let url = format!("ws://127.0.0.1:{}{}", linhas[0], linhas[1]);
    let falhou = |tipo: &str, e: String| {
        Erro::Navegador(format!(
            "não consegui falar com o navegador ({tipo}: {})",
            corta(&e, 120)
        ))
    };
    // direto em 127.0.0.1 (sem proxy) e sem Origin (o navegador recusa origens desconhecidas)
    let end = ("127.0.0.1", linhas[0].parse::<u16>().unwrap_or(0))
        .to_socket_addrs()
        .map_err(|e| falhou("OSError", e.to_string()))?
        .next()
        .ok_or_else(|| falhou("OSError", "endereço".into()))?;
    let tcp = TcpStream::connect_timeout(&end, Duration::from_secs(15))
        .map_err(|e| falhou("OSError", e.to_string()))?;
    let _ = tcp.set_read_timeout(Some(Duration::from_secs(15)));
    let (ws, _) = tungstenite::client(url.as_str(), tcp)
        .map_err(|e| falhou("WebSocketException", e.to_string()))?;
    Ok(WsReal { ws })
}

/// Espera o processo sair sozinho (depois do Browser.close); senão, encerra.
fn encerrar(proc: &mut Child, espera: f64) {
    let fim = Instant::now() + Duration::from_secs_f64(espera);
    loop {
        if !matches!(proc.try_wait(), Ok(None)) {
            return;
        }
        if Instant::now() >= fim {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = proc.kill();
    let fim = Instant::now() + Duration::from_secs(5);
    while matches!(proc.try_wait(), Ok(None)) && Instant::now() < fim {
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Apaga o perfil temporário. No Windows, os arquivos podem ficar presos por
/// alguns instantes depois que o navegador sai.
fn apagar(pasta: Option<&Path>) {
    let Some(pasta) = pasta else { return };
    for _ in 0..10 {
        let _ = std::fs::remove_dir_all(pasta);
        if !pasta.exists() {
            return;
        }
        std::thread::sleep(Duration::from_millis(300));
    }
}

// ============================================================
// "Localizar" do painel (último recurso dos verificadores de planilha)
// ============================================================

const JS_PAINEL_PRONTO: &str = "!window.__ferramentas1doc_antiga && document.readyState === 'complete' && !!document.getElementById('num_codigo')";

const JS_PREENCHER: &str = r#"(function (idDocumento, numero, ano) {
  // O <select> é escondido pelo select2: o valor vai no próprio elemento
  // nativo, que é o que o formulário envia.
  var s = document.getElementById('id_documento_rapido');
  if (!s) return 'sem o campo de tipo de documento';
  s.value = idDocumento;
  s.dispatchEvent(new Event('change', {bubbles: true}));
  function digitar(id, valor) {
    var e = document.getElementById(id);
    if (!e) return false;
    e.focus();
    e.value = valor;
    ['input', 'keyup', 'change'].forEach(function (t) { e.dispatchEvent(new Event(t, {bubbles: true})); });
    return true;
  }
  if (!digitar('num_codigo', numero)) return 'sem o campo do número';
  if (!digitar('localiza_ano', ano)) return 'sem o campo do ano';
  var b = document.querySelector("form.form_localiza button[type='submit']");
  if (!b) return 'sem o botão de localizar';
  window.__ferramentas1doc_antiga = true;
  b.click();
  return '';
})(%s, %s, %s)"#;

fn aguardar<T>(timeout: f64, mut condicao: impl FnMut() -> Res<Option<T>>) -> Option<T> {
    let fim = Instant::now() + Duration::from_secs_f64(timeout);
    loop {
        if let Ok(Some(r)) = condicao() {
            return Some(r); // erro do navegador = página trocando: tenta de novo
        }
        if Instant::now() > fim {
            return None;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Preenche o "Localizar" do painel e devolve a URL em que a aba parou
/// quando `achou(url, url_antes)` for verdadeiro; None se não chegou lá.
pub fn localizar_no_painel(
    nav: &mut dyn Navegador,
    url_painel: &str,
    id_documento: &str,
    digitos: &str,
    ano: &str,
    timeout: f64,
    achou: &dyn Fn(&str, &str) -> bool,
) -> Res<Option<String>> {
    let alvo = nav.pagina()?;
    // Marca o documento atual: o painel só conta como carregado num documento novo.
    let _ = nav.avaliar(&alvo, "window.__ferramentas1doc_antiga = true");
    nav.ir(url_painel, Some(&alvo))?;
    if aguardar(timeout, || {
        Ok(nav
            .avaliar(&alvo, JS_PAINEL_PRONTO)?
            .as_bool()
            .filter(|b| *b))
    })
    .is_none()
    {
        return Ok(None);
    }
    let url_antes = nav.url(Some(&alvo))?;
    let js = JS_PREENCHER
        .replacen("%s", &json!(id_documento).to_string(), 1)
        .replacen("%s", &json!(digitos).to_string(), 1);
    let js = js.replacen("%s", &json!(ano).to_string(), 1);
    let falta = nav.avaliar(&alvo, &js)?;
    if crate::arquivos::verdadeiro(&falta) {
        return Ok(None);
    }
    Ok(aguardar(timeout, || {
        let u = nav.url(Some(&alvo))?;
        Ok(if achou(&u, &url_antes) { Some(u) } else { None })
    }))
}
