//! Apoio dos testes: um 1Doc de mentira num servidor HTTP local (porta de
//! tests/fake1doc.py) e um ambiente com pastas temporárias (o `conftest.py`).
#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use chrono::{Local, NaiveDate, TimeZone};
use ferramentas1doc::saida::Saida;
use ferramentas1doc::sessao::Credenciais;
use ferramentas1doc::{Ambiente, config};
use serde_json::json;

pub fn hoje() -> NaiveDate {
    Local::now().date_naive()
}

pub fn emissao() -> String {
    format!("{} 09:15", hoje().format("%d/%m/%Y"))
}

pub struct Processo {
    pub hash: &'static str,
    pub numero: &'static str,
    pub objeto: &'static str,
    pub descricao: &'static str,
    pub etapas: [(&'static str, &'static str, bool); 4],
    pub prioridade: &'static str,
}

/// P1 31.632/2026 — Abertura, etapa atual "Pesquisa de Mercado" (há 2 horas)
/// P2 31.700/2026 — Abertura, etapa atual "Validação de Requisição" (há 3 dias)
pub const PROCESSOS: [Processo; 2] = [
    Processo {
        hash: "AAAA1111BBBB2222CCCC",
        numero: "31.632/2026",
        objeto: "Aquisição de medicamentos",
        descricao: "Aquisição de medicamentos para a rede municipal de saúde",
        etapas: [
            ("Validação de Requisição", "Ficou aqui 1 dia", false),
            ("Correção de Documentação", "", false),
            ("Pesquisa de Mercado", "Esta aqui 2 horas", true),
            ("Etapa Final", "", false),
        ],
        prioridade: "Alta",
    },
    Processo {
        hash: "DDDD3333EEEE4444FFFF",
        numero: "31.700/2026",
        objeto: "Reforma da escola municipal",
        descricao: "Reforma da Escola Municipal Exemplo",
        etapas: [
            ("Validação de Requisição", "Esta aqui 3 dias", true),
            ("Correção de Documentação", "", false),
            ("Pesquisa de Mercado", "", false),
            ("Etapa Final", "", false),
        ],
        prioridade: "Média",
    },
];

pub const LOGADO: &str = r#"<input type="hidden" id="sis_usr_email" value="teste@montesclaros.mg.gov.br">
<input type="hidden" id="s_s" value="SPOT-DCL-GP-CPPFI"><input type="hidden" id="s_is" value="285">"#;

pub fn painel() -> String {
    format!(
        r#"<html><body>{LOGADO}
<ul><li class="li_setor active">Inbox de SPOT-DCL-GP-CPPFI</li></ul>
<div class="tab-content"><input type="hidden" id="caixa_parametro_ordem" value="desc"></div>
</body></html>"#
    )
}

pub fn linha(p: &Processo) -> String {
    let hoje = hoje().format("%d/%m/%Y");
    format!(
        r#"<tr>
<td><a class="link_emissao_a" href="./?pg=doc/ver&hash={h}&itd=3&origem=painel_setor">Proc.
   Administrativo {num} <small class="data">{em}</small></a></td>
<td><div class="td_remetente"><span class="badge badge_env tt">SS-DAF</span>
  <span class="pp" data-content="Maria da Silva">Maria S.</span></div></td>
<td class="link_emissao td ignora normal celula_para"><span class="badge">SPOT-DCL-GP-CPPFI</span></td>
<td class="link_emissao td ignora normal">{obj}
  <span class="badge" title="Abertura de Procedimento Licitatório">Abertura de Proc...</span>
  <div class="resumo">resumo do processo</div>
  <span class="label" title="Prioridade">{pri}</span></td>
<td><small class="tt data" title="Última movimentação em {hoje} 10:00:00">2 horas</small></td>
</tr>"#,
        h = p.hash,
        num = p.numero,
        em = emissao(),
        obj = p.objeto,
        pri = p.prioridade
    )
}

pub fn listagem(caixa: &str, pagina: u32) -> String {
    let linhas: String = if caixa == "entrada" && pagina == 1 {
        PROCESSOS.iter().map(linha).collect()
    } else {
        String::new()
    };
    let n = if caixa == "entrada" {
        PROCESSOS.len()
    } else {
        0
    };
    format!(
        r#"<div class="navega_caixa">Página 1/1</div>
<table class="table">{linhas}</table>
<script>$('#num_tab_{caixa}').html('({n})')</script>"#
    )
}

pub fn kanban(p: &Processo, modelo: bool) -> String {
    let mut blocos = String::new();
    for (nome, marca, atual) in p.etapas {
        let marca = if modelo {
            if atual {
                "Aqui há alguns segundos"
            } else {
                ""
            }
        } else {
            marca
        };
        let cls = if nome == "Etapa Final" {
            "kanban_estagios estagio_final"
        } else {
            "kanban_estagios"
        };
        let item = if atual {
            format!(r#"<p class="kanban_item">{marca}</p>"#)
        } else if !marca.is_empty() {
            format!(r#"<div class="kanban-time">{marca}</div>"#)
        } else {
            String::new()
        };
        blocos.push_str(&format!(
            r#"<div class="{cls}" data-current="{}"><div class="kanban_titulo"><p>{nome}</p></div><div class="kanban_itens">{item}</div><div class="kanban_footer"><span class="badge_env">SPOT-DCL-GP-CPPFI</span></div></div>"#,
            if atual { "true" } else { "false" }
        ));
    }
    format!(r#"<div class="workflow_v2_kanban">{blocos}</div>"#)
}

pub fn detalhe(p: &Processo) -> String {
    let h = hoje();
    let mk = Local
        .from_local_datetime(&h.and_hms_opt(9, 15, 0).unwrap())
        .earliest()
        .map(|d| d.timestamp())
        .unwrap_or(0);
    let hoje = h.format("%d/%m/%Y");
    format!(
        r#"<html><body>{LOGADO}
<input type="hidden" id="nome_em" value="Proc. Administrativo {num}">
<input type="hidden" id="hash_emissao" value="{hash}">
<input type="hidden" id="id_setor_origem" value="12">
<div id="emissao_principal" data-datamk="{mk}">
  <h2 class="tt pull-left emission_infos_item_atual"><span class="nd">Proc. Administrativo</span>
    <small class="nd_num">{num}</small> <span class="label" title="Prioridade">{pri}</span></h2>
  <h4 class="tit tit_assunto" data-id_assunto="961721">{obj}
    <span class="badge" title="Assunto: Abertura de Procedimento Licitatório">Abertura de Proc...</span></h4>
  <a class="ir_quadro">Ir ao quadro: Abertura de Procedimento Licitatório</a>
  {kanban}
  <div id="campo_adicional_descricao_objeto"><div class="cp">
    <span data-valor="{desc}">Objeto<span>*</span>:</span><small>{desc}</small></div></div>
</div>
<span class="badge" data-id_setor="12" title="Secretaria de Saúde » SS-DAF">SS-DAF</span>
<div class="timeline_conteudo" data-datamk="{mk}">
<table class="relative despacho"><tr><th>
  <abbr class="despacho_data" title="Em {hoje} 10:00">hoje</abbr>
  <div class="rf_header"><strong>Despacho 1</strong></div>
  <div class="rf_de"><span class="badge_env">SS-DAF</span></div>
</th></tr><tr><td class="celula_despacho"><div class="texto_original">Encaminho para análise.</div></td></tr>
</table></div>
<div class="tm_conteudo"><div id="parent_wa" data-current_idph="1">{modelo}</div></div>
</body></html>"#,
        num = p.numero,
        hash = p.hash,
        pri = p.prioridade,
        obj = p.objeto,
        desc = p.descricao,
        kanban = kanban(p, false),
        modelo = kanban(p, true),
    )
}

pub const TELA_LOGIN: &str =
    r#"<form><input name="identifier"><input type="password" name="senha"></form>"#;

/// Um pedido recebido: método, caminho+query e corpo (formulário).
#[derive(Clone, Debug)]
pub struct Pedido {
    pub metodo: String,
    pub caminho: String,
    pub query: HashMap<String, String>,
    pub corpo: HashMap<String, String>,
}

impl Pedido {
    /// `(método, chamada | pg | caminho)`, como `Fake1Doc.pedidos`.
    pub fn chave(&self) -> (String, String) {
        let rotulo = self
            .query
            .get("chamada")
            .or_else(|| self.query.get("pg"))
            .cloned()
            .unwrap_or_else(|| self.caminho.clone());
        (self.metodo.clone(), rotulo)
    }
}

pub struct Resposta {
    pub status: u16,
    pub corpo: String,
    pub location: Option<String>,
}

impl Resposta {
    pub fn html(corpo: impl Into<String>) -> Self {
        Resposta {
            status: 200,
            corpo: corpo.into(),
            location: None,
        }
    }
}

pub type Manipulador = dyn Fn(&Pedido) -> Option<Resposta> + Send + Sync;

pub struct Fake1Doc {
    pub base: String,
    pub logado: Arc<AtomicBool>,
    pub pedidos: Arc<Mutex<Vec<Pedido>>>,
    pub intervalos: Arc<Mutex<Vec<Option<String>>>>,
    servidor: Arc<tiny_http::Server>,
}

fn decodificar(q: &str) -> HashMap<String, String> {
    url::form_urlencoded::parse(q.as_bytes())
        .into_owned()
        .collect()
}

impl Fake1Doc {
    pub fn novo() -> Self {
        Self::com(None)
    }

    /// Com um manipulador próprio, consultado antes das rotas padrão.
    pub fn com(extra: Option<Box<Manipulador>>) -> Self {
        let servidor = Arc::new(tiny_http::Server::http("127.0.0.1:0").unwrap());
        let porta = servidor.server_addr().to_ip().unwrap().port();
        let fake = Fake1Doc {
            base: format!("http://127.0.0.1:{porta}"),
            logado: Arc::new(AtomicBool::new(true)),
            pedidos: Arc::new(Mutex::new(Vec::new())),
            intervalos: Arc::new(Mutex::new(Vec::new())),
            servidor: servidor.clone(),
        };
        let (logado, pedidos, intervalos) = (
            fake.logado.clone(),
            fake.pedidos.clone(),
            fake.intervalos.clone(),
        );
        let extra: Arc<Option<Box<Manipulador>>> = Arc::new(extra);
        std::thread::spawn(move || {
            for mut req in servidor.incoming_requests() {
                let url = req.url().to_string();
                let (caminho, q) = url.split_once('?').unwrap_or((&url, ""));
                let mut corpo = String::new();
                let _ = req.as_reader().read_to_string(&mut corpo);
                let pedido = Pedido {
                    metodo: req.method().as_str().to_uppercase(),
                    caminho: caminho.to_string(),
                    query: decodificar(q),
                    corpo: decodificar(&corpo),
                };
                pedidos.lock().unwrap().push(pedido.clone());
                let r = (*extra)
                    .as_ref()
                    .and_then(|f| f(&pedido))
                    .unwrap_or_else(|| rotear(&pedido, logado.load(Ordering::SeqCst), &intervalos));
                let mut resp = tiny_http::Response::from_string(r.corpo)
                    .with_status_code(r.status)
                    .with_header(
                        "Content-Type: text/html; charset=utf-8"
                            .parse::<tiny_http::Header>()
                            .unwrap(),
                    );
                if let Some(l) = r.location {
                    resp = resp.with_header(tiny_http::Header::from_bytes("Location", l).unwrap());
                }
                let _ = req.respond(resp);
            }
        });
        fake
    }

    pub fn deslogar(&self) {
        self.logado.store(false, Ordering::SeqCst);
    }

    pub fn chaves(&self) -> Vec<(String, String)> {
        self.pedidos
            .lock()
            .unwrap()
            .iter()
            .map(Pedido::chave)
            .collect()
    }

    pub fn intervalos(&self) -> Vec<Option<String>> {
        self.intervalos.lock().unwrap().clone()
    }
}

impl Drop for Fake1Doc {
    fn drop(&mut self) {
        self.servidor.unblock();
    }
}

fn rotear(p: &Pedido, logado: bool, intervalos: &Mutex<Vec<Option<String>>>) -> Resposta {
    if !logado {
        return Resposta::html(TELA_LOGIN);
    }
    if p.caminho.contains("formEmissao.php") || p.query.contains_key("formAcessoRapido") {
        let num = format!(
            "{}/{}",
            p.corpo.get("num_codigo").cloned().unwrap_or_default(),
            p.corpo.get("ano").cloned().unwrap_or_default()
        )
        .replace('.', "");
        return match PROCESSOS.iter().find(|x| x.numero.replace('.', "") == num) {
            Some(x) => Resposta {
                status: 302,
                corpo: String::new(),
                location: Some(format!("/?pg=doc/ver&hash={}&itd=3", x.hash)),
            },
            None => Resposta::html("<html></html>"),
        };
    }
    if p.query.get("chamada").map(String::as_str) == Some("caixa") {
        intervalos
            .lock()
            .unwrap()
            .push(p.query.get("intervalo").cloned());
        let pagina = p
            .query
            .get("num_pagina")
            .and_then(|n| n.parse().ok())
            .unwrap_or(1);
        return Resposta::html(listagem(
            p.query.get("caixa").map(String::as_str).unwrap_or(""),
            pagina,
        ));
    }
    if p.query.get("pg").map(String::as_str) == Some("doc/ver") {
        let h = p
            .query
            .get("hash")
            .cloned()
            .unwrap_or_default()
            .to_uppercase();
        return match PROCESSOS.iter().find(|x| x.hash == h) {
            Some(x) => Resposta::html(detalhe(x)),
            None => Resposta {
                status: 404,
                corpo: "não encontrado".into(),
                location: None,
            },
        };
    }
    Resposta::html(painel())
}

/// Pastas de dados e de saída temporárias; config com a planilha não abrindo.
pub struct Dados {
    pub tmp: tempfile::TempDir,
    pub amb: Ambiente,
}

impl Dados {
    pub fn pasta_dados(&self) -> std::path::PathBuf {
        self.tmp.path().join("dados")
    }

    pub fn pasta_saida(&self) -> std::path::PathBuf {
        self.tmp.path().join("saida")
    }

    pub fn saida_texto(&self) -> String {
        self.amb.saida.texto()
    }
}

pub fn dados(base: &str) -> Dados {
    let tmp = tempfile::tempdir().unwrap();
    let pasta = tmp.path().join("dados");
    let saida = tmp.path().join("saida");
    std::fs::create_dir_all(&pasta).unwrap();
    std::fs::create_dir_all(&saida).unwrap();
    let amb = Ambiente::novo(base, pasta, Some(saida.clone()), Saida::memoria());
    let cfg = config::Config {
        abrir_planilha: false,
        pasta_saida: saida.to_string_lossy().into_owned(),
        ..Default::default()
    };
    config::salvar(&amb, &cfg).unwrap();
    Dados { tmp, amb }
}

pub fn cred() -> Credenciais {
    Credenciais {
        cookies: vec![
            json!({"name": "PHPSESSID", "value": "abc", "domain": "montesclaros.1doc.com.br", "path": "/"}),
        ],
        ua: "Mozilla/5.0 (teste)".into(),
        setor: "SPOT-DCL-GP-CPPFI".into(),
        email: "teste@montesclaros.mg.gov.br".into(),
        salvo_em: String::new(),
    }
}

/// Valores de uma aba (None = a ativa) como texto; célula vazia = "".
pub fn valores(caminho: &std::path::Path, aba: Option<&str>) -> Vec<Vec<String>> {
    let wb = umya_spreadsheet::reader::xlsx::read(caminho).unwrap();
    let ws = match aba {
        Some(n) => wb.sheet_by_name(n).unwrap(),
        None => wb.active_sheet(),
    };
    let (cols, linhas) = ws.highest_column_and_row();
    (1..=linhas)
        .map(|r| (1..=cols).map(|c| ws.value((c, r))).collect())
        .collect()
}

/// Link da célula (coluna, linha), se houver.
pub fn link(caminho: &std::path::Path, aba: Option<&str>, col: u32, linha: u32) -> Option<String> {
    let wb = umya_spreadsheet::reader::xlsx::read(caminho).unwrap();
    let ws = match aba {
        Some(n) => wb.sheet_by_name(n).unwrap(),
        None => wb.active_sheet(),
    };
    ws.cell((col, linha))
        .and_then(|c| c.hyperlink())
        .map(|h| h.url().to_string())
}
