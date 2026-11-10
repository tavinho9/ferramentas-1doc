//! Cliente HTTP com os cookies da sessão do navegador (o `requests.Session`
//! dos scripts): User-Agent e cookies do navegador logado, repetição
//! automática em erro transitório com espera crescente (`urllib3.Retry`),
//! redirecionamentos seguidos à mão (para guardar o `Location` de cada um,
//! como `r.history`) e cancelamento antes de cada requisição.

use std::sync::{Arc, Once};
use std::time::Duration;

use reqwest::blocking::Client;
use reqwest::cookie::{CookieStore, Jar};
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderValue, LOCATION, RETRY_AFTER};
use serde_json::Value;

use crate::ambiente::Ambiente;
use crate::erro::{Erro, Res, corta};

/// Política de repetição (`Retry(total=3, backoff_factor=..., status_forcelist=...)`).
#[derive(Clone, Debug)]
pub struct Repeticao {
    pub total: u32,
    pub fator: f64,
    pub status: &'static [u16],
    /// Métodos repetidos (o Retry padrão não repete POST).
    pub post_tambem: bool,
}

impl Repeticao {
    pub const fn padrao(fator: f64) -> Self {
        Repeticao {
            total: 3,
            fator,
            status: &[429, 500, 502, 503, 504],
            post_tambem: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Cookie {
    pub nome: String,
    pub valor: String,
    pub dominio: String,
    pub caminho: String,
}

/// Uma resposta já lida.
#[derive(Clone, Debug)]
pub struct Resposta {
    pub status: u16,
    /// URL final, depois dos redirecionamentos (`r.url`).
    pub url: String,
    /// `Location` de cada redirecionamento seguido, mais o da resposta final.
    pub locations: Vec<String>,
    pub content_type: String,
    pub corpo: Vec<u8>,
}

impl Resposta {
    /// `r.content.decode("utf-8", errors="replace")`.
    pub fn texto_utf8(&self) -> String {
        String::from_utf8_lossy(&self.corpo).into_owned()
    }

    /// `r.text` com o charset do Content-Type; UTF-8 se não houver
    /// (o `_texto(r)` dos verificadores).
    pub fn texto(&self) -> String {
        let ct = self.content_type.to_lowercase();
        if let Some(i) = ct.find("charset=") {
            let nome = ct[i + 8..]
                .split(';')
                .next()
                .unwrap_or("")
                .trim()
                .trim_matches(['"', '\'']);
            if let Some(enc) = encoding_rs::Encoding::for_label(nome.as_bytes()) {
                return enc.decode_without_bom_handling(&self.corpo).0.into_owned();
            }
        }
        self.texto_utf8()
    }

    /// `r.raise_for_status()`.
    pub fn conferir(self) -> Res<Self> {
        if self.status >= 400 {
            let motivo = if self.status >= 500 {
                "Server Error"
            } else {
                "Client Error"
            };
            return Err(Erro::Http {
                msg: format!("{} {motivo} for url: {}", self.status, self.url),
                status: Some(self.status),
            });
        }
        Ok(self)
    }
}

fn instalar_cripto() {
    static UMA: Once = Once::new();
    UMA.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

/// Sessão HTTP autenticada pelos cookies do navegador.
#[derive(Clone)]
pub struct SessaoHttp {
    cliente: Client,
    jar: Arc<Jar>,
    pub repeticao: Repeticao,
    pub timeout: Duration,
    amb: Ambiente,
}

/// Cookie do `sessao.json` (`{"name", "value", "domain", "path"}`).
pub fn cookie_de_json(c: &Value) -> Option<Cookie> {
    Some(Cookie {
        nome: c.get("name")?.as_str()?.to_string(),
        valor: c
            .get("value")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        dominio: c
            .get("domain")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        caminho: c
            .get("path")
            .and_then(Value::as_str)
            .filter(|p| !p.is_empty())
            .unwrap_or("/")
            .to_string(),
    })
}

impl SessaoHttp {
    pub fn nova(
        amb: &Ambiente,
        cookies: &[Value],
        ua: &str,
        timeout: f64,
        repeticao: Repeticao,
    ) -> Self {
        instalar_cripto();
        let jar = Arc::new(Jar::default());
        let base = url::Url::parse(&amb.base()).ok();
        for c in cookies.iter().filter_map(cookie_de_json) {
            let host = c.dominio.trim_start_matches('.');
            let url = if host.is_empty() {
                base.clone()
            } else {
                url::Url::parse(&format!("https://{host}/")).ok()
            };
            let Some(url) = url else { continue };
            let mut s = format!("{}={}; Path={}", c.nome, c.valor, c.caminho);
            if !c.dominio.is_empty() {
                s.push_str(&format!("; Domain={}", c.dominio));
            }
            jar.add_cookie_str(&s, &url);
            // http:// (testes) e https:// compartilham o mesmo pote de cookies.
        }
        let mut cab = HeaderMap::new();
        let ua = if ua.is_empty() { "Mozilla/5.0" } else { ua };
        cab.insert(
            reqwest::header::USER_AGENT,
            HeaderValue::from_str(ua).unwrap_or(HeaderValue::from_static("Mozilla/5.0")),
        );
        cab.insert(
            reqwest::header::ACCEPT_LANGUAGE,
            HeaderValue::from_static("pt-BR,pt;q=0.9"),
        );
        let t = Duration::from_secs_f64(timeout);
        let cliente = Client::builder()
            .default_headers(cab)
            .cookie_provider(jar.clone())
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(t)
            // o requests conta o tempo entre pacotes; o cliente bloqueante só
            // tem tempo total: folga para páginas grandes que chegam devagar.
            .timeout(t * 3)
            .pool_max_idle_per_host(16)
            .build()
            .expect("cliente HTTP");
        SessaoHttp {
            cliente,
            jar,
            repeticao,
            timeout: t,
            amb: amb.clone(),
        }
    }

    /// Os cookies que seriam enviados para `url` (`nome=valor; ...`).
    pub fn cookies_para(&self, url: &str) -> String {
        url::Url::parse(url)
            .ok()
            .and_then(|u| self.jar.cookies(&u))
            .and_then(|h| h.to_str().ok().map(str::to_string))
            .unwrap_or_default()
    }

    pub fn get(
        &self,
        url: &str,
        params: &[(&str, String)],
        cabecalhos: &[(&str, String)],
    ) -> Res<Resposta> {
        self.pedir(false, url, params, &[], cabecalhos)
    }

    pub fn post(
        &self,
        url: &str,
        dados: &[(&str, String)],
        cabecalhos: &[(&str, String)],
    ) -> Res<Resposta> {
        self.pedir(true, url, &[], dados, cabecalhos)
    }

    fn pedir(
        &self,
        post: bool,
        url: &str,
        params: &[(&str, String)],
        dados: &[(&str, String)],
        cabecalhos: &[(&str, String)],
    ) -> Res<Resposta> {
        self.amb.checar_cancelamento()?;
        let mut alvo = url::Url::parse(url).map_err(|e| Erro::Http {
            msg: format!("URL inválida {url}: {e}"),
            status: None,
        })?;
        if !params.is_empty() {
            let mut q = alvo.query_pairs_mut();
            for (k, v) in params {
                q.append_pair(k, v);
            }
        }
        let mut metodo_post = post;
        let mut locations = Vec::new();
        for _ in 0..=30 {
            let r = self.com_repeticao(
                metodo_post,
                alvo.as_str(),
                if metodo_post { dados } else { &[] },
                cabecalhos,
            )?;
            let status = r.status().as_u16();
            let loc = r
                .headers()
                .get(LOCATION)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);
            if let Some(l) = &loc {
                locations.push(l.clone());
            }
            if matches!(status, 301 | 302 | 303 | 307 | 308)
                && let Some(l) = loc
            {
                let proximo = alvo.join(&l).map_err(|e| Erro::Http {
                    msg: format!("redirecionamento inválido: {e}"),
                    status: None,
                })?;
                if status == 303 || (matches!(status, 301 | 302) && metodo_post) {
                    metodo_post = false;
                }
                alvo = proximo;
                continue;
            }
            let content_type = r
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();
            let url_final = r.url().to_string();
            let corpo = r.bytes().map_err(|e| erro_rede(&e))?.to_vec();
            return Ok(Resposta {
                status,
                url: url_final,
                locations,
                content_type,
                corpo,
            });
        }
        Err(Erro::Http {
            msg: format!("Exceeded 30 redirects: {url}"),
            status: None,
        })
    }

    fn com_repeticao(
        &self,
        post: bool,
        url: &str,
        dados: &[(&str, String)],
        cabecalhos: &[(&str, String)],
    ) -> Res<reqwest::blocking::Response> {
        let rep = &self.repeticao;
        let pode_repetir = !post || rep.post_tambem;
        let mut erros = 0u32;
        loop {
            let mut req = if post {
                self.cliente.post(url).form(dados)
            } else {
                self.cliente.get(url)
            };
            for (k, v) in cabecalhos {
                req = req.header(*k, v.as_str());
            }
            match req.send() {
                Ok(r) => {
                    let st = r.status().as_u16();
                    if pode_repetir && rep.status.contains(&st) {
                        if erros >= rep.total {
                            return Err(Erro::Http {
                                msg: format!(
                                    "Max retries exceeded with url: {url} (too many {st} error responses)"
                                ),
                                status: Some(st),
                            });
                        }
                        erros += 1;
                        let depois = r
                            .headers()
                            .get(RETRY_AFTER)
                            .and_then(|v| v.to_str().ok())
                            .and_then(|v| v.trim().parse::<f64>().ok())
                            .filter(|_| matches!(st, 413 | 429 | 503));
                        self.amb
                            .esperar(depois.unwrap_or_else(|| espera(rep.fator, erros)))?;
                        continue;
                    }
                    return Ok(r);
                }
                Err(e) => {
                    if !pode_repetir || erros >= rep.total {
                        return Err(erro_rede(&e));
                    }
                    erros += 1;
                    self.amb.esperar(espera(rep.fator, erros))?;
                }
            }
        }
    }
}

/// `Retry.get_backoff_time()` do urllib3 2: 0 na primeira repetição, depois
/// `fator * 2^(n-1)`, até 120 s.
fn espera(fator: f64, erros: u32) -> f64 {
    if erros <= 1 {
        0.0
    } else {
        (fator * 2f64.powi(erros as i32 - 1)).min(120.0)
    }
}

fn erro_rede(e: &reqwest::Error) -> Erro {
    let mut msg = e.to_string();
    let mut fonte = std::error::Error::source(e);
    while let Some(f) = fonte {
        msg.push_str(&format!(": {f}"));
        fonte = f.source();
    }
    Erro::Http {
        msg: corta(&msg, 300),
        status: e.status().map(|s| s.as_u16()),
    }
}
