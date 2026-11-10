//! Sessão do 1Doc: login feito pela pessoa no navegador, sessão lembrada.
//!
//! O programa NUNCA pede nem digita usuário e senha. Quando não há sessão
//! válida, ele abre o Chrome (ou o Edge) na página do 1Doc e espera: a
//! pessoa faz o login na janela, do jeito de sempre (inclusive captcha ou
//! segundo fator, se houver). Detectado o login, os cookies e o User-Agent
//! do navegador são copiados, a janela é fechada e todo o resto sai por
//! HTTP.
//!
//! A sessão fica em sessao.json, na pasta de dados do usuário (a mesma da
//! versão Python), e é reaproveitada enquanto o 1Doc a aceitar. "Sair"
//! apaga o arquivo e o perfil do navegador usado no login.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use chrono::Local;
use serde_json::{Map, Value, json};

use crate::ambiente::Ambiente;
use crate::erro::{Erro, Res, primeira_linha};
use crate::http::{Repeticao, SessaoHttp};
use crate::navegador::{self, Navegador, NavegadorCdp};
use crate::{arquivos, config, html};

pub const ARQUIVO_SESSAO: &str = "sessao.json";

/// `URL_LOGIN`: sem sessão, o 1Doc redireciona para o login.
pub fn url_login(amb: &Ambiente) -> String {
    format!("{}?pg=painel/listar", amb.base())
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Credenciais {
    pub cookies: Vec<Value>,
    pub ua: String,
    pub setor: String,
    pub email: String,
    pub salvo_em: String,
}

pub fn agora_iso() -> String {
    Local::now().format("%Y-%m-%dT%H:%M:%S").to_string()
}

fn caminho_sessao(amb: &Ambiente) -> PathBuf {
    amb.arquivo_dados(ARQUIVO_SESSAO)
}

pub fn carregar_salva(amb: &Ambiente) -> Option<Credenciais> {
    let dados = arquivos::ler_json(&caminho_sessao(amb))?;
    if dados.get("base").and_then(Value::as_str) != Some(amb.base().as_str()) {
        return None;
    }
    let cookies = dados.get("cookies")?.as_array()?.clone();
    Some(Credenciais {
        cookies,
        ua: arquivos::texto(&dados, "ua"),
        setor: arquivos::texto(&dados, "setor"),
        email: arquivos::texto(&dados, "email"),
        salvo_em: arquivos::texto(&dados, "salvo_em"),
    })
}

pub fn salvar(amb: &Ambiente, cred: &Credenciais) -> std::io::Result<()> {
    let caminho = caminho_sessao(amb);
    let mut m = Map::new();
    m.insert("base".into(), json!(amb.base()));
    m.insert("ua".into(), json!(cred.ua));
    m.insert("cookies".into(), Value::Array(cred.cookies.clone()));
    m.insert("setor".into(), json!(cred.setor));
    m.insert("email".into(), json!(cred.email));
    let salvo = if cred.salvo_em.is_empty() {
        agora_iso()
    } else {
        cred.salvo_em.clone()
    };
    m.insert("salvo_em".into(), json!(salvo));
    arquivos::gravar_atomico(&caminho, &arquivos::json_python(&Value::Object(m)))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&caminho, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// Apaga só a sessão salva (ex.: o 1Doc a recusou no meio da execução).
pub fn esquecer(amb: &Ambiente) {
    let _ = std::fs::remove_file(caminho_sessao(amb));
}

/// Apaga a sessão salva e o perfil do navegador do login. Devolve os avisos
/// (pastas que não puderam ser apagadas, p.ex. navegador aberto).
pub fn sair(amb: &Ambiente) -> Vec<String> {
    esquecer(amb);
    let mut avisos = Vec::new();
    let perfil = amb.pasta_perfil_navegador();
    if perfil.is_dir() {
        let _ = std::fs::remove_dir_all(&perfil);
        if perfil.is_dir() {
            avisos.push(format!(
                "não consegui apagar {} (o navegador do login está aberto?)",
                perfil.display()
            ));
        }
    }
    avisos
}

// ============================================================
// Validação por HTTP
// ============================================================

/// (setor, e-mail) se os cookies valem para requisições; None se o 1Doc
/// devolveu a tela de login; `SemConexao` se ele não respondeu. Se o "Inbox
/// pessoal" estiver ativo, troca para o inbox do setor (o mesmo link do
/// menu), porque as caixas seguem o inbox ativo.
pub fn verificar_cookies(
    amb: &Ambiente,
    cookies: &[Value],
    ua: &str,
) -> Res<Option<(String, String)>> {
    let s = SessaoHttp::nova(
        amb,
        cookies,
        ua,
        30.0,
        Repeticao {
            total: 0,
            ..Repeticao::padrao(0.0)
        },
    );
    let sem_conexao = |e: Erro| match e {
        Erro::Cancelado => Erro::Cancelado,
        outro => Erro::SemConexao(primeira_linha(&outro.to_string(), 160)),
    };
    for tentativa in 1..=2 {
        let r = s
            .get(&url_login(amb), &[], &[])
            .and_then(|r| r.conferir())
            .map_err(sem_conexao)?;
        let doc = html::doc(&r.texto_utf8());
        let email = html::um_doc(&doc, sel!("input#sis_usr_email"))
            .map(|e| html::attr_ou_vazio(e, "value").trim().to_string())
            .unwrap_or_default();
        if email.is_empty() {
            return Ok(None);
        }
        if html::um_doc(&doc, sel!("li.li_pessoal.active")).is_some() && tentativa == 1 {
            let params = [
                ("pg", "painel/listar".to_string()),
                ("meu", "0".into()),
                ("trocar", "1".into()),
            ];
            s.get(&amb.base(), &params, &[]).map_err(sem_conexao)?;
            continue;
        }
        let setor = html::um_doc(&doc, sel!("input#s_s"))
            .map(|e| html::attr_ou_vazio(e, "value").to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "(setor não identificado)".into());
        return Ok(Some((setor, email)));
    }
    Ok(None)
}

// ============================================================
// Navegador
// ============================================================

pub type Criar<'a> = dyn Fn(&str, Option<&Path>, bool) -> Res<Box<dyn Navegador>> + 'a;
pub type Localizar<'a> = dyn Fn(&str) -> Option<PathBuf> + 'a;

fn criar_real(qual: &str, perfil: Option<&Path>, headless: bool) -> Res<Box<dyn Navegador>> {
    Ok(Box::new(NavegadorCdp::abrir(
        qual,
        perfil,
        headless,
        "about:blank",
        30.0,
    )?))
}

/// Devolve (navegador, nome). Em "auto": Chrome, depois Edge. Se o perfil
/// próprio estiver travado (outra janela do login aberta), tenta sem ele.
pub fn abrir_navegador(
    amb: &Ambiente,
    qual: &str,
    headless: bool,
    usar_perfil: bool,
) -> Res<(Box<dyn Navegador>, String)> {
    abrir_navegador_com(
        amb,
        qual,
        headless,
        usar_perfil,
        &criar_real,
        &navegador::localizar_executavel,
    )
}

pub fn abrir_navegador_com(
    amb: &Ambiente,
    qual: &str,
    headless: bool,
    usar_perfil: bool,
    criar: &Criar<'_>,
    localizar: &Localizar<'_>,
) -> Res<(Box<dyn Navegador>, String)> {
    let ordem: Vec<&str> = if qual == "auto" {
        vec!["chrome", "edge"]
    } else {
        vec![qual]
    };
    let mut erros = Vec::new();
    for q in ordem {
        let perfis: Vec<Option<PathBuf>> = if usar_perfil {
            vec![Some(amb.pasta_perfil_navegador().join(q)), None]
        } else {
            vec![None]
        };
        for perfil in perfis {
            match criar(q, perfil.as_deref(), headless) {
                Ok(n) => return Ok((n, q.to_string())),
                Err(e) if e.eh_navegador() || matches!(e, Erro::Outro(_)) => {
                    let marca = if perfil.is_some() {
                        " (perfil próprio)"
                    } else {
                        ""
                    };
                    erros.push(format!(
                        "{q}{marca}: {}",
                        primeira_linha(&e.to_string(), 160)
                    ));
                    if localizar(q).is_none() {
                        break; // não instalado: o perfil temporário também não vai abrir
                    }
                }
                Err(e) => return Err(e),
            }
        }
    }
    Err(Erro::LoginCancelado(format!(
        "não consegui abrir o navegador.\n  {}",
        erros.join("\n  ")
    )))
}

/// Única coisa que o programa executa nas páginas durante o login: lê, não
/// digita nem clica.
pub const JS_ESTADO: &str = "(function () {
  var e = document.getElementById('sis_usr_email');
  return {email: e ? (e.value || '').trim() : '',
          abas: !!document.querySelector('.tab-content'),
          senha: !!document.querySelector('input[type=password]'),
          ua: navigator.userAgent};
})()";

fn host_de(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_lowercase))
        .unwrap_or_default()
}

/// Estado da primeira aba do 1Doc com usuário logado ({email, ua, ...}), ou
/// None. O sinal é o campo #sis_usr_email preenchido, que só existe em
/// páginas logadas; a área de abas (.tab-content) vale como reserva desde
/// que não haja campo de senha. Abas de outros domínios (SSO) não contam.
fn pagina_logada(nav: &mut dyn Navegador, host: &str) -> Res<Option<Value>> {
    for pag in nav.paginas()? {
        if host_de(&pag.url) != host {
            continue;
        }
        let Ok(estado) = nav.avaliar(&pag.id, JS_ESTADO) else {
            continue; // página carregando / trocando
        };
        let v = |k: &str| estado.get(k).is_some_and(arquivos::verdadeiro);
        if estado.is_object() && (v("email") || (v("abas") && !v("senha"))) {
            return Ok(Some(estado));
        }
    }
    Ok(None)
}

pub type Validar<'a> = dyn Fn(&[Value], &str) -> Res<Option<(String, String)>> + 'a;
pub type Fabrica<'a> = dyn Fn() -> Res<(Box<dyn Navegador>, String)> + 'a;

/// Parâmetros da espera do login (os de `aguardar_login`).
pub struct Espera<'a> {
    pub minutos: f64,
    pub avisar: &'a dyn Fn(&str),
    pub fabrica: &'a Fabrica<'a>,
    pub validar: &'a Validar<'a>,
    pub intervalo: f64,
    pub intervalo_validacao: f64,
    /// Pausa para o 1Doc terminar de gravar os cookies.
    pub pausa_cookies: f64,
}

/// Abre o navegador no 1Doc e espera a pessoa fazer o login. Não preenche
/// nem clica em nada. Devolve Credenciais já validadas por HTTP.
pub fn aguardar_login(amb: &Ambiente, e: &Espera<'_>) -> Res<Credenciais> {
    let host = amb.host();
    let (mut nav, nome) = (e.fabrica)()?;
    let r = (|| -> Res<Credenciais> {
        if let Err(err) = nav.ir(&url_login(amb), None) {
            if err.eh_navegador() {
                return Err(Erro::LoginCancelado(format!(
                    "o navegador não abriu o 1Doc ({})",
                    primeira_linha(&err.to_string(), 120)
                )));
            }
            return Err(err);
        }
        (e.avisar)(&format!(
            "→ Faça o login no 1Doc na janela do {} que abriu.",
            navegador::nome(&nome)
        ));
        (e.avisar)(&format!(
            "  O programa está esperando (até {} min). Ele não digita nada: usuário e senha são só com você.",
            minutos_texto(e.minutos)
        ));
        let fim = Instant::now() + Duration::from_secs_f64(e.minutos * 60.0);
        let mut ultima_validacao: Option<Instant> = None;
        loop {
            amb.checar_cancelamento()?;
            if Instant::now() > fim {
                return Err(Erro::LoginCancelado(format!(
                    "tempo esgotado ({} min) esperando o login no navegador",
                    minutos_texto(e.minutos)
                )));
            }
            if !nav.viva() {
                return Err(Erro::LoginCancelado(
                    "a janela do navegador foi fechada antes do login".into(),
                ));
            }
            let logado = match pagina_logada(nav.as_mut(), &host) {
                Ok(l) => l,
                Err(err) if err.eh_navegador() => None,
                Err(err) => return Err(err),
            };
            let pode_validar =
                ultima_validacao.is_none_or(|u| u.elapsed().as_secs_f64() >= e.intervalo_validacao);
            if let Some(logado) = logado.filter(|_| pode_validar) {
                ultima_validacao = Some(Instant::now());
                amb.esperar(e.pausa_cookies)?; // deixa o 1Doc terminar de gravar os cookies
                let cookies = match nav.cookies(Some(&host)) {
                    Ok(c) => c,
                    Err(err) if err.eh_navegador() => continue,
                    Err(err) => return Err(err),
                };
                let ua = logado
                    .get("ua")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                amb.checar_cancelamento()?;
                let ok = match (e.validar)(&cookies, &ua) {
                    Ok(ok) => ok,
                    Err(Erro::SemConexao(_)) => None, // tenta de novo na próxima volta
                    Err(err) => return Err(err),
                };
                if let Some((setor, email)) = ok {
                    (e.avisar)(&format!("✓ Login detectado — {email}, setor {setor}"));
                    return Ok(Credenciais {
                        cookies,
                        ua,
                        setor,
                        email,
                        salvo_em: agora_iso(),
                    });
                }
                // Página parece logada, mas os cookies ainda não valem fora do
                // navegador: continua esperando (o 1Doc pode estar redirecionando).
            }
            amb.esperar(e.intervalo)?;
        }
    })();
    nav.fechar();
    r
}

fn minutos_texto(m: f64) -> String {
    if m.fract() == 0.0 {
        format!("{}", m as i64)
    } else {
        format!("{m}")
    }
}

/// Ganchos do `obter_sessao` (os testes trocam as funções reais).
pub struct Ganchos<'a> {
    pub verificar: &'a Validar<'a>,
    pub aguardar: &'a dyn Fn(&str, f64) -> Res<Credenciais>,
}

/// Credenciais válidas: as salvas, se o 1Doc ainda as aceita; senão abre o
/// navegador e espera o login. `ao_esperar(true/false)` avisa a interface
/// de que a janela de login está aberta.
pub fn obter_sessao(
    amb: &Ambiente,
    forcar_login: bool,
    qual: Option<&str>,
    minutos: Option<f64>,
    ao_esperar: Option<&dyn Fn(bool)>,
) -> Res<Credenciais> {
    let avisar = |t: &str| amb.saida.linha(t);
    let verificar = |c: &[Value], ua: &str| verificar_cookies(amb, c, ua);
    let aguardar = |qual: &str, minutos: f64| {
        let fabrica = || abrir_navegador(amb, qual, false, true);
        aguardar_login(
            amb,
            &Espera {
                minutos,
                avisar: &avisar,
                fabrica: &fabrica,
                validar: &verificar,
                intervalo: 1.0,
                intervalo_validacao: 4.0,
                pausa_cookies: 1.0,
            },
        )
    };
    obter_sessao_com(
        amb,
        forcar_login,
        qual,
        minutos,
        ao_esperar,
        &Ganchos {
            verificar: &verificar,
            aguardar: &aguardar,
        },
    )
}

pub fn obter_sessao_com(
    amb: &Ambiente,
    forcar_login: bool,
    qual: Option<&str>,
    minutos: Option<f64>,
    ao_esperar: Option<&dyn Fn(bool)>,
    g: &Ganchos<'_>,
) -> Res<Credenciais> {
    let cfg = config::carregar(amb);
    let qual = qual.map(str::to_string).unwrap_or(cfg.navegador);
    let minutos = minutos.unwrap_or(cfg.minutos_login as f64);

    if !forcar_login && let Some(mut salva) = carregar_salva(amb) {
        amb.checar_cancelamento()?;
        match (g.verificar)(&salva.cookies, &salva.ua) {
            Err(Erro::SemConexao(e)) => {
                return Err(Erro::LoginCancelado(format!(
                    "o 1Doc não respondeu — confira a internet e tente de novo ({e})"
                )));
            }
            Err(e) => return Err(e),
            Ok(Some((setor, email))) => {
                salva.setor = setor;
                salva.email = email;
                amb.saida.linha(&format!(
                    "✓ Sessão salva reaproveitada — {}, setor {}",
                    salva.email, salva.setor
                ));
                return Ok(salva);
            }
            Ok(None) => {
                amb.saida
                    .linha("A sessão salva expirou; é preciso entrar de novo.");
                esquecer(amb);
            }
        }
    }

    amb.saida.linha("Abrindo o navegador para o login no 1Doc…");
    if let Some(f) = ao_esperar {
        f(true);
    }
    let cred = (g.aguardar)(&qual, minutos);
    if let Some(f) = ao_esperar {
        f(false);
    }
    let cred = cred?;
    salvar(amb, &cred)?;
    Ok(cred)
}

// ============================================================
// Navegador invisível sob demanda (fallback do "Localizar")
// ============================================================

/// Os verificadores de planilha usam o navegador como último recurso para
/// achar o hash de um processo pelo formulário "Localizar". Um navegador
/// invisível é aberto só na primeira vez que precisar, com os cookies da
/// sessão, e fechado no fim.
pub struct NavegadorSobDemanda {
    cred: Credenciais,
    qual: Option<String>,
    estado: Mutex<(Option<Box<dyn Navegador>>, bool)>,
}

impl NavegadorSobDemanda {
    pub fn novo(cred: &Credenciais, qual: Option<&str>) -> Self {
        NavegadorSobDemanda {
            cred: cred.clone(),
            qual: qual.map(str::to_string),
            estado: Mutex::new((None, false)),
        }
    }

    /// Roda `f` com o navegador (abrindo-o na primeira vez); None se ele
    /// não pôde ser aberto.
    pub fn com<T>(&self, amb: &Ambiente, f: impl FnOnce(&mut dyn Navegador) -> T) -> Option<T> {
        let mut est = self.estado.lock().unwrap_or_else(|e| e.into_inner());
        if est.0.is_none() && !est.1 {
            let qual = self
                .qual
                .clone()
                .unwrap_or_else(|| config::carregar(amb).navegador);
            match abrir_navegador(amb, &qual, true, false) {
                Ok((mut n, _)) => {
                    n.definir_cookies(&self.cred.cookies);
                    est.0 = Some(n);
                }
                Err(e) => {
                    est.1 = true;
                    amb.saida.linha(&format!(
                        "  ⚠ navegador invisível indisponível para o 'Localizar' ({})",
                        crate::texto::corta(&e.to_string(), 80)
                    ));
                }
            }
        }
        est.0.as_mut().map(|n| f(n.as_mut()))
    }

    pub fn fechar(&self) {
        let mut est = self.estado.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(mut n) = est.0.take() {
            n.fechar();
        }
    }
}

impl Drop for NavegadorSobDemanda {
    fn drop(&mut self) {
        self.fechar();
    }
}
