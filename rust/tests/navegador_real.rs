//! Login com um navegador de verdade contra um "1Doc" local (porta de
//! tests/test_navegador_real.py).
//!
//! A página de login deste servidor tem um script que faz o papel da
//! pessoa: depois de alguns segundos ele preenche os campos e envia o
//! formulário. O programa só pode esperar — o teste confere que ele não
//! detecta o login antes do envio, que os valores enviados são os da
//! "pessoa" e que os cookies capturados valem nas requisições.
//!
//! Roda só quando há navegador configurado: `FERRAMENTAS1DOC_CHROME=/caminho/chrome`
//! ou `AUTOTESTE_NAVEGADOR_REAL=1` (usa o Chrome/Edge instalado;
//! `AUTOTESTE_NAVEGADOR=edge` testa o Edge).

mod comum;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ferramentas1doc::navegador::Navegador;
use ferramentas1doc::sessao::{self, Credenciais, Espera, NavegadorSobDemanda};
use serde_json::json;

fn habilitado() -> bool {
    let ok = std::env::var_os("FERRAMENTAS1DOC_CHROME").is_some_and(|v| !v.is_empty())
        || std::env::var_os("AUTOTESTE_NAVEGADOR_REAL").is_some_and(|v| !v.is_empty());
    if !ok {
        eprintln!("pulado: sem navegador real configurado");
    }
    ok
}

fn qual() -> String {
    std::env::var("AUTOTESTE_NAVEGADOR").unwrap_or_else(|_| "chrome".into())
}

const LOGIN: &str = r#"<html><body>
<div class="tab-content">(a página de login também tem abas: não pode contar como logada)</div>
<form id="f" method="post" action="/login">
  <input name="identifier" type="email"><input name="senha" type="password"><button type="submit">Entrar</button>
</form>
<script>
  // "a pessoa" digita e entra depois de 2,5 s
  setTimeout(function () {
    document.querySelector('[name=identifier]').value = 'pessoa@montesclaros.mg.gov.br';
    document.querySelector('[name=senha]').value = 'segredo-da-pessoa';
    document.getElementById('f').submit();
  }, 2500);
</script></body></html>"#;

const LOGADO: &str = r#"<input type="hidden" id="sis_usr_email" value="pessoa@montesclaros.mg.gov.br">
<input type="hidden" id="s_s" value="SPOT-DCL-GP-CPPFI">"#;

pub const PAINEL_LOCALIZAR: &str = r#"<form class="form_localiza" onsubmit="return localizar(event)">
  <select id="id_documento_rapido" style="display:none">
    <option value="">--</option><option value="3">Proc. Administrativo</option></select>
  <input id="num_codigo"><input id="localiza_ano"><button type="submit">Localizar</button>
</form>
<script>
  function localizar(ev) {
    ev.preventDefault();
    var tipo = document.getElementById('id_documento_rapido').value,
        num = document.getElementById('num_codigo').value,
        ano = document.getElementById('localiza_ano').value,
        hash = (tipo === '3' && num === '31632' && ano === '2026') ? 'AAAA1111BBBB2222CCCC' : '';
    setTimeout(function () { location.href = '/?pg=doc/ver&hash=' + hash + '&itd=3'; }, 300);
    return false;
  }
</script>"#;

#[derive(Default)]
pub struct Registro {
    pub envios: Vec<(Instant, HashMap<String, String>)>,
    pub inicio: Option<Instant>,
}

pub struct Servidor {
    pub base: String,
    pub reg: Arc<Mutex<Registro>>,
    srv: Arc<tiny_http::Server>,
}

impl Drop for Servidor {
    fn drop(&mut self) {
        self.srv.unblock();
    }
}

pub fn servidor() -> Servidor {
    let srv = Arc::new(tiny_http::Server::http("127.0.0.1:0").unwrap());
    let porta = srv.server_addr().to_ip().unwrap().port();
    let reg = Arc::new(Mutex::new(Registro::default()));
    let (s2, r2) = (srv.clone(), reg.clone());
    std::thread::spawn(move || {
        for mut req in s2.incoming_requests() {
            let cookie = req
                .headers()
                .iter()
                .find(|h| h.field.equiv("Cookie"))
                .map(|h| h.value.as_str().to_string())
                .unwrap_or_default();
            let html = |t: String| {
                tiny_http::Response::from_string(t).with_header(
                    "Content-Type: text/html; charset=utf-8"
                        .parse::<tiny_http::Header>()
                        .unwrap(),
                )
            };
            if req.method() == &tiny_http::Method::Post {
                let mut corpo = String::new();
                let _ = req.as_reader().read_to_string(&mut corpo);
                let campos = url::form_urlencoded::parse(corpo.as_bytes())
                    .into_owned()
                    .collect();
                r2.lock().unwrap().envios.push((Instant::now(), campos));
                let resp = tiny_http::Response::empty(302)
                    .with_header(
                        "Set-Cookie: PHPSESSID=ok; Path=/; HttpOnly"
                            .parse::<tiny_http::Header>()
                            .unwrap(),
                    )
                    .with_header(
                        "Location: /?pg=painel/listar"
                            .parse::<tiny_http::Header>()
                            .unwrap(),
                    );
                let _ = req.respond(resp);
            } else if cookie.contains("PHPSESSID=ok") {
                let corpo = if req.url().contains("caixa=entrada") {
                    format!("<html><body>{LOGADO}\n{PAINEL_LOCALIZAR}</body></html>")
                } else {
                    format!(
                        r#"<html><body>{LOGADO}<div class="tab-content">caixas</div></body></html>"#
                    )
                };
                let _ = req.respond(html(corpo));
            } else {
                let mut r = r2.lock().unwrap();
                r.inicio.get_or_insert_with(Instant::now);
                drop(r);
                let _ = req.respond(html(LOGIN.to_string()));
            }
        }
    });
    Servidor {
        base: format!("http://127.0.0.1:{porta}"),
        reg,
        srv,
    }
}

#[test]
fn login_real_no_navegador() {
    if !habilitado() {
        return;
    }
    let srv = servidor();
    let d = comum::dados(&srv.base);
    let fabrica = || sessao::abrir_navegador(&d.amb, &qual(), true, false);
    let validar = |c: &[serde_json::Value], u: &str| sessao::verificar_cookies(&d.amb, c, u);
    let cred = sessao::aguardar_login(
        &d.amb,
        &Espera {
            minutos: 1.0,
            avisar: &|t| println!("{t}"),
            fabrica: &fabrica,
            validar: &validar,
            intervalo: 0.3,
            intervalo_validacao: 1.0,
            pausa_cookies: 1.0,
        },
    )
    .unwrap();
    let detectado = Instant::now();
    let reg = srv.reg.lock().unwrap();
    assert_eq!(reg.envios.len(), 1); // um envio, o da "pessoa"
    let (envio_t, campos) = &reg.envios[0];
    assert_eq!(campos["identifier"], "pessoa@montesclaros.mg.gov.br");
    assert_eq!(campos["senha"], "segredo-da-pessoa");
    assert!(envio_t.duration_since(reg.inicio.unwrap()) >= Duration::from_secs(2)); // nada enviado antes
    assert!(detectado >= *envio_t); // e só detectou depois do envio
    assert_eq!(
        (cred.email.as_str(), cred.setor.as_str()),
        ("pessoa@montesclaros.mg.gov.br", "SPOT-DCL-GP-CPPFI")
    );
    assert!(
        cred.cookies
            .iter()
            .any(|c| c["name"] == "PHPSESSID" && c["value"] == "ok")
    );
}

pub fn cred_ok() -> Credenciais {
    Credenciais {
        cookies: vec![
            json!({"name": "PHPSESSID", "value": "ok", "domain": "127.0.0.1", "path": "/",
                             "httpOnly": true, "secure": false, "sameSite": "Lax"}),
        ],
        ua: "UA".into(),
        ..Default::default()
    }
}

#[test]
fn navegador_invisivel_com_cookies_da_sessao() {
    // Fallback do 'Localizar': o navegador invisível já entra logado com os cookies.
    if !habilitado() {
        return;
    }
    let srv = servidor();
    let d = comum::dados(&srv.base);
    let nav = NavegadorSobDemanda::novo(&cred_ok(), Some(&qual()));
    let email = "document.getElementById('sis_usr_email') && document.getElementById('sis_usr_email').value";
    let lido = nav
        .com(&d.amb, |drv: &mut dyn Navegador| {
            drv.ir(&format!("{}/?pg=painel/listar", srv.base), None)
                .unwrap();
            let aba = drv.pagina().unwrap();
            for _ in 0..50 {
                if drv
                    .avaliar(&aba, email)
                    .is_ok_and(|v| v.as_str().is_some_and(|s| !s.is_empty()))
                {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            drv.avaliar(&aba, email).unwrap()
        })
        .expect("navegador invisível não abriu");
    assert_eq!(lido, "pessoa@montesclaros.mg.gov.br");
    // reaproveita, não abre outro: a aba continua no painel
    let url = nav.com(&d.amb, |drv| drv.url(None).unwrap()).unwrap();
    assert!(url.contains("pg=painel/listar"), "{url}");
    nav.fechar();
}
