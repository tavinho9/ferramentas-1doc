//! Login: o programa só espera; nunca digita nem clica (porta de tests/test_sessao.py).

mod comum;

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ferramentas1doc::navegador::{
    self, ErroTransporte, Navegador, NavegadorCdp, Pagina, Transporte,
};
use ferramentas1doc::sessao::{self, Credenciais, Espera, Ganchos};
use ferramentas1doc::{Erro, Res};
use serde_json::{Value, json};

const HOST: &str = "https://montesclaros.1doc.com.br/";

/// Simula a pessoa: fica na tela de login por `login_apos` verificações e
/// então "entra"; ou fecha a janela depois de `fechar_apos` consultas às
/// abas. Guarda todo script executado nas páginas, para o teste conferir
/// que o programa só lê.
#[derive(Default)]
struct Estado {
    verificacoes: usize,
    acessos: usize,
    urls: Vec<String>,
    scripts: Vec<String>,
    fechado: bool,
    encerrado: bool,
}

#[derive(Clone)]
struct NavegadorFalso {
    login_apos: Option<usize>,
    fechar_apos: Option<usize>,
    url_forcada: Option<String>,
    e: Arc<Mutex<Estado>>,
}

impl NavegadorFalso {
    fn novo(
        login_apos: Option<usize>,
        fechar_apos: Option<usize>,
        url_forcada: Option<&str>,
    ) -> Self {
        NavegadorFalso {
            login_apos,
            fechar_apos,
            url_forcada: url_forcada.map(str::to_string),
            e: Default::default(),
        }
    }

    fn logado(&self, e: &Estado) -> bool {
        self.login_apos.is_some_and(|n| e.verificacoes >= n)
    }
}

impl Navegador for NavegadorFalso {
    fn ir(&mut self, url: &str, _alvo: Option<&str>) -> Res<()> {
        self.e.lock().unwrap().urls.push(url.to_string());
        Ok(())
    }

    fn paginas(&mut self) -> Res<Vec<Pagina>> {
        let mut e = self.e.lock().unwrap();
        e.acessos += 1;
        if self.fechar_apos.is_some_and(|n| e.acessos >= n) {
            e.fechado = true;
        }
        if e.fechado {
            return Err(Erro::Navegador("conexão com o navegador perdida".into()));
        }
        let url = self.url_forcada.clone().unwrap_or_else(|| {
            if self.logado(&e) {
                format!("{HOST}?pg=painel/listar")
            } else {
                format!("{HOST}login")
            }
        });
        Ok(vec![Pagina {
            id: "aba1".into(),
            url,
        }])
    }

    fn pagina(&mut self) -> Res<String> {
        Ok("aba1".into())
    }

    fn url(&mut self, _alvo: Option<&str>) -> Res<String> {
        Ok(String::new())
    }

    fn avaliar(&mut self, _alvo: &str, js: &str) -> Res<Value> {
        let mut e = self.e.lock().unwrap();
        e.scripts.push(js.to_string());
        e.verificacoes += 1;
        if self.logado(&e) {
            Ok(
                json!({"email": "teste@pmc", "abas": true, "senha": false, "ua": "Mozilla/5.0 (teste)"}),
            )
        } else {
            Ok(json!({"email": "", "abas": true, "senha": true, "ua": "Mozilla/5.0 (teste)"}))
        }
    }

    fn versao(&mut self) -> Res<String> {
        Ok("falso".into())
    }

    fn viva(&mut self) -> bool {
        self.paginas().is_ok_and(|p| !p.is_empty())
    }

    fn cookies(&mut self, _host: Option<&str>) -> Res<Vec<Value>> {
        Ok(vec![
            json!({"name": "PHPSESSID", "value": "abc", "domain": "montesclaros.1doc.com.br", "path": "/"}),
        ])
    }

    fn definir_cookies(&mut self, _c: &[Value]) -> usize {
        0
    }

    fn fechar(&mut self) {
        self.e.lock().unwrap().encerrado = true;
    }
}

fn aguardar(
    d: &comum::Dados,
    nav: &NavegadorFalso,
    validar: &sessao::Validar<'_>,
    minutos: f64,
) -> Res<Credenciais> {
    let n = nav.clone();
    let fabrica =
        move || -> Res<(Box<dyn Navegador>, String)> { Ok((Box::new(n.clone()), "chrome".into())) };
    sessao::aguardar_login(
        &d.amb,
        &Espera {
            minutos,
            avisar: &|_| {},
            fabrica: &fabrica,
            validar,
            intervalo: 0.01,
            intervalo_validacao: 0.0,
            pausa_cookies: 0.0,
        },
    )
}

fn sempre_ok(_c: &[Value], _u: &str) -> Res<Option<(String, String)>> {
    Ok(Some(("SPOT-DCL-GP-CPPFI".into(), "teste@pmc".into())))
}

fn dados() -> comum::Dados {
    comum::dados("https://montesclaros.1doc.com.br")
}

#[test]
fn espera_o_login_sem_digitar_nada() {
    let d = dados();
    let nav = NavegadorFalso::novo(Some(5), None, None);
    let cred = aguardar(&d, &nav, &sempre_ok, 1.0).unwrap();
    assert_eq!(
        (cred.setor.as_str(), cred.email.as_str()),
        ("SPOT-DCL-GP-CPPFI", "teste@pmc")
    );
    assert_eq!(cred.cookies[0]["name"], "PHPSESSID");
    assert_eq!(cred.ua, "Mozilla/5.0 (teste)");
    let e = nav.e.lock().unwrap();
    assert!(e.verificacoes >= 5); // esperou: não tomou a tela de login por logada
    assert_eq!(e.urls, [sessao::url_login(&d.amb)]); // só abriu a página
    assert!(e.scripts.iter().all(|s| s == sessao::JS_ESTADO)); // só leu: nada digitado nem clicado
    assert!(e.encerrado); // fechou o navegador
}

#[test]
fn cookies_que_nao_valem_continuam_esperando() {
    let d = dados();
    let respostas = RefCell::new(vec![
        Some(("SETOR".to_string(), "x@y".to_string())),
        None,
        None,
    ]);
    let validar = |_c: &[Value], _u: &str| Ok(respostas.borrow_mut().pop().unwrap());
    let cred = aguardar(
        &d,
        &NavegadorFalso::novo(Some(1), None, None),
        &validar,
        1.0,
    )
    .unwrap();
    assert_eq!(cred.setor, "SETOR");
}

#[test]
fn pagina_de_login_em_outro_dominio_nao_conta() {
    // SSO num domínio diferente: só a página do 1Doc da prefeitura vale.
    let d = dados();
    let nav = NavegadorFalso::novo(
        Some(0),
        Some(12),
        Some("https://login.exemplo.com/?sis_usr_email"),
    );
    let erro = aguardar(&d, &nav, &sempre_ok, 1.0).unwrap_err();
    assert!(
        matches!(&erro, Erro::LoginCancelado(m) if m.contains("fechada")),
        "{erro:?}"
    );
    assert_eq!(nav.e.lock().unwrap().verificacoes, 0); // nem leu a página fora do domínio do 1Doc
}

#[test]
fn janela_fechada_cancela_o_login() {
    let d = dados();
    let nav = NavegadorFalso::novo(None, Some(7), None);
    let erro = aguardar(&d, &nav, &sempre_ok, 1.0).unwrap_err();
    assert!(
        matches!(&erro, Erro::LoginCancelado(m) if m.contains("fechada")),
        "{erro:?}"
    );
    assert!(nav.e.lock().unwrap().encerrado);
}

#[test]
fn tempo_esgotado() {
    let d = dados();
    let erro = aguardar(
        &d,
        &NavegadorFalso::novo(None, None, None),
        &sempre_ok,
        0.002,
    )
    .unwrap_err();
    assert!(
        matches!(&erro, Erro::LoginCancelado(m) if m.contains("tempo esgotado")),
        "{erro:?}"
    );
}

#[test]
fn botao_cancelar() {
    let d = dados();
    let nav = NavegadorFalso::novo(None, None, None);
    let c = d.amb.cancelar.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        c.pedir();
    });
    assert_eq!(
        aguardar(&d, &nav, &sempre_ok, 1.0).unwrap_err(),
        Erro::Cancelado
    );
    assert!(nav.e.lock().unwrap().encerrado);
}

fn nao_abre(_q: &str, _m: f64) -> Res<Credenciais> {
    panic!("abriu o navegador")
}

#[test]
fn sessao_salva_valida_nao_abre_navegador() {
    let d = dados();
    let cred = comum::cred();
    sessao::salvar(&d.amb, &cred).unwrap();
    let verificar = |_c: &[Value], _u: &str| Ok(Some(("SETOR X".to_string(), "a@b".to_string())));
    let obtida = sessao::obter_sessao_com(
        &d.amb,
        false,
        None,
        None,
        None,
        &Ganchos {
            verificar: &verificar,
            aguardar: &nao_abre,
        },
    )
    .unwrap();
    assert_eq!(obtida.cookies, cred.cookies);
    assert_eq!(obtida.setor, "SETOR X");
}

#[test]
fn sessao_expirada_abre_o_navegador_e_regrava() {
    let d = dados();
    sessao::salvar(&d.amb, &comum::cred()).unwrap();
    let nova = Credenciais {
        cookies: vec![json!({"name": "novo", "value": "1"})],
        ua: "UA".into(),
        setor: "S".into(),
        email: "e".into(),
        salvo_em: String::new(),
    };
    let verificar = |_c: &[Value], _u: &str| Ok(None);
    let n = nova.clone();
    let aguardar = move |_q: &str, _m: f64| Ok(n.clone());
    let estados = RefCell::new(Vec::new());
    let ao_esperar = |s: bool| estados.borrow_mut().push(s);
    let obtida = sessao::obter_sessao_com(
        &d.amb,
        false,
        None,
        None,
        Some(&ao_esperar),
        &Ganchos {
            verificar: &verificar,
            aguardar: &aguardar,
        },
    )
    .unwrap();
    assert_eq!(obtida, nova);
    assert_eq!(*estados.borrow(), [true, false]);
    assert_eq!(
        sessao::carregar_salva(&d.amb).unwrap().cookies,
        vec![json!({"name": "novo", "value": "1"})]
    );
}

#[test]
fn sair_apaga_sessao_e_perfil() {
    let d = dados();
    sessao::salvar(&d.amb, &comum::cred()).unwrap();
    std::fs::create_dir_all(d.amb.pasta_perfil_navegador().join("chrome")).unwrap();
    assert!(sessao::sair(&d.amb).is_empty());
    assert!(sessao::carregar_salva(&d.amb).is_none());
    assert!(!d.amb.pasta_perfil_navegador().exists());
}

#[test]
fn sessao_salva_pela_versao_python_vale() {
    // sessao.json gravado pelo Python (json.dump, separadores com espaço)
    let d = dados();
    let texto = r#"{"base": "https://montesclaros.1doc.com.br/", "ua": "UA py", "cookies": [{"name": "PHPSESSID", "value": "abc", "domain": "montesclaros.1doc.com.br", "path": "/"}], "setor": "SPOT", "email": "a@b", "salvo_em": "2026-11-05T10:00:00"}"#;
    std::fs::write(d.amb.arquivo_dados("sessao.json"), texto).unwrap();
    let c = sessao::carregar_salva(&d.amb).unwrap();
    assert_eq!(
        (c.ua.as_str(), c.email.as_str(), c.cookies.len()),
        ("UA py", "a@b", 1)
    );
    // e o que o Rust grava tem o mesmo formato
    sessao::salvar(&d.amb, &c).unwrap();
    assert_eq!(
        std::fs::read_to_string(d.amb.arquivo_dados("sessao.json")).unwrap(),
        texto
    );
}

#[test]
fn verificar_cookies_troca_inbox_pessoal_para_o_do_setor() {
    let n = Arc::new(Mutex::new(0));
    let n2 = n.clone();
    let fake = comum::Fake1Doc::com(Some(Box::new(move |_p| {
        let mut n = n2.lock().unwrap();
        *n += 1;
        let pessoal = *n == 1;
        Some(comum::Resposta::html(format!(
            r#"<input id="sis_usr_email" value="a@b"><input id="s_s" value="SPOT">{}"#,
            if pessoal {
                r#"<li class="li_pessoal active">x</li>"#
            } else {
                ""
            }
        )))
    })));
    let d = comum::dados(&fake.base);
    assert_eq!(
        sessao::verificar_cookies(&d.amb, &[], "UA").unwrap(),
        Some(("SPOT".into(), "a@b".into()))
    );
    let pedidos = fake.pedidos.lock().unwrap();
    let q = &pedidos[1].query;
    assert_eq!(
        (q["pg"].as_str(), q["meu"].as_str(), q["trocar"].as_str()),
        ("painel/listar", "0", "1")
    );
}

#[test]
fn verificar_cookies_tela_de_login() {
    let fake = comum::Fake1Doc::novo();
    fake.deslogar();
    let d = comum::dados(&fake.base);
    assert_eq!(sessao::verificar_cookies(&d.amb, &[], "UA").unwrap(), None);
}

#[test]
fn sem_conexao_nao_descarta_a_sessao_salva() {
    // porta fechada: o 1Doc não responde
    let porta = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let d = comum::dados(&format!("http://127.0.0.1:{porta}"));
    sessao::salvar(&d.amb, &comum::cred()).unwrap();
    let verificar = |c: &[Value], u: &str| sessao::verificar_cookies(&d.amb, c, u);
    let erro = sessao::obter_sessao_com(
        &d.amb,
        false,
        None,
        None,
        None,
        &Ganchos {
            verificar: &verificar,
            aguardar: &nao_abre,
        },
    )
    .unwrap_err();
    assert!(
        matches!(&erro, Erro::LoginCancelado(m) if m.contains("não respondeu")),
        "{erro:?}"
    );
    assert!(sessao::carregar_salva(&d.amb).is_some()); // continua guardada
}

#[test]
fn sem_conexao_durante_a_espera_continua_esperando() {
    let d = dados();
    let respostas = RefCell::new(vec![
        Ok(Some(("SETOR".to_string(), "x@y".to_string()))),
        Err(Erro::SemConexao("x".into())),
    ]);
    let validar = |_c: &[Value], _u: &str| respostas.borrow_mut().pop().unwrap();
    assert_eq!(
        aguardar(
            &d,
            &NavegadorFalso::novo(Some(1), None, None),
            &validar,
            1.0
        )
        .unwrap()
        .setor,
        "SETOR"
    );
}

/// WebSocket do navegador: responde cada pedido com o resultado do método.
struct WsFalso {
    resultados: Value,
    ultimo: Option<Value>,
}

impl Transporte for WsFalso {
    fn definir_timeout(&mut self, _t: Duration) {}

    fn enviar(&mut self, texto: &str) -> Result<(), ErroTransporte> {
        self.ultimo = Some(serde_json::from_str(texto).unwrap());
        Ok(())
    }

    fn receber(&mut self) -> Result<String, ErroTransporte> {
        let p = self.ultimo.as_ref().unwrap();
        let metodo = p["method"].as_str().unwrap();
        Ok(json!({"id": p["id"], "result": self.resultados[metodo]}).to_string())
    }

    fn fechar(&mut self) {}
}

#[test]
fn cookies_do_navegador_so_os_do_1doc_no_formato_da_sessao_salva() {
    let ws = WsFalso {
        resultados: json!({"Storage.getCookies": {"cookies": [
            {"name": "PHPSESSID", "value": "abc", "domain": "montesclaros.1doc.com.br", "path": "/",
             "expires": -1, "session": true, "httpOnly": true, "secure": true, "sameSite": "Lax"},
            {"name": "lembrar", "value": "1", "domain": ".1doc.com.br", "path": "/", "expires": 1893456000.5,
             "session": false},
            {"name": "NID", "value": "x", "domain": ".google.com", "path": "/", "expires": 1893456000},
            {"name": "outra", "value": "y", "domain": "outra.1doc.com.br", "path": "/"}]}}),
        ultimo: None,
    };
    let mut nav = NavegadorCdp::com_transporte(Box::new(ws), "chrome");
    assert_eq!(
        nav.cookies(Some("montesclaros.1doc.com.br")).unwrap(),
        vec![
            json!({"name": "PHPSESSID", "value": "abc", "domain": "montesclaros.1doc.com.br", "path": "/",
                   "secure": true, "httpOnly": true, "sameSite": "Lax"}),
            json!({"name": "lembrar", "value": "1", "domain": ".1doc.com.br", "path": "/", "secure": false,
                   "httpOnly": false, "expiry": 1893456000}),
        ]
    );
}

#[test]
fn cookies_salvos_voltam_ao_navegador_com_o_mesmo_alcance() {
    let so_host = navegador::cookie_cdp(&json!({"name": "PHPSESSID", "value": "abc",
        "domain": "montesclaros.1doc.com.br", "path": "/", "secure": true, "httpOnly": true,
        "expiry": 1893456000, "sameSite": "Lax"}))
    .unwrap();
    assert_eq!(
        so_host,
        json!({"name": "PHPSESSID", "value": "abc", "path": "/", "url": "https://montesclaros.1doc.com.br/",
               "secure": true, "httpOnly": true, "sameSite": "Lax", "expires": 1893456000.0})
    );
    let dominio = navegador::cookie_cdp(
        &json!({"name": "x", "value": "1", "domain": ".1doc.com.br", "path": "/"}),
    )
    .unwrap();
    assert_eq!(
        (dominio["domain"].as_str(), dominio["url"].as_str()),
        (Some(".1doc.com.br"), Some("http://1doc.com.br/"))
    );
}

#[test]
fn automatico_usa_o_edge_quando_nao_ha_chrome() {
    let d = dados();
    let tentativas = RefCell::new(Vec::new());
    let criar = |qual: &str, perfil: Option<&Path>, _h: bool| -> Res<Box<dyn Navegador>> {
        tentativas
            .borrow_mut()
            .push((qual.to_string(), perfil.is_some()));
        if qual == "chrome" {
            return Err(Erro::Navegador(
                "Google Chrome não encontrado neste computador".into(),
            ));
        }
        Ok(Box::new(NavegadorFalso::novo(None, None, None)))
    };
    let localizar = |q: &str| {
        if q == "chrome" {
            None
        } else {
            Some(PathBuf::from("msedge.exe"))
        }
    };
    let (_n, nome) =
        sessao::abrir_navegador_com(&d.amb, "auto", false, true, &criar, &localizar).unwrap();
    assert_eq!(nome, "edge");
    // sem Chrome, nem tenta o perfil temporário
    assert_eq!(
        *tentativas.borrow(),
        [("chrome".to_string(), true), ("edge".to_string(), true)]
    );
}

#[test]
fn sem_navegador_explica_cada_tentativa() {
    let d = dados();
    let criar = |qual: &str, _p: Option<&Path>, _h: bool| -> Res<Box<dyn Navegador>> {
        Err(Erro::Navegador(format!(
            "{} não encontrado neste computador",
            navegador::nome(qual)
        )))
    };
    let localizar = |_q: &str| None;
    let erro = sessao::abrir_navegador_com(&d.amb, "auto", false, true, &criar, &localizar)
        .err()
        .unwrap();
    let Erro::LoginCancelado(m) = erro else {
        panic!("{erro:?}")
    };
    assert!(
        m.contains("Google Chrome não encontrado") && m.contains("Microsoft Edge não"),
        "{m}"
    );
}
