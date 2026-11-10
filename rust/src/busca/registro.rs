//! Registro consolidado (listagem + detalhe), filtros e ordenação.

use chrono::NaiveDateTime;
use serde_json::Value;

use super::args::Opcoes;
use super::parse::{GUARDAR_DESPACHOS, casa_setor, contem, de_iso, de_iso_v, igual, normcod};
use crate::re;
use crate::texto::{corta, normtxt};

/// Uma transição de etapa, com a data já lida.
#[derive(Clone, Debug)]
pub struct Transicao {
    pub data: Option<NaiveDateTime>,
    pub de: String,
    pub para: String,
    pub mesma_etapa: bool,
    pub estimada: bool,
    pub bruto: Value,
}

impl Transicao {
    pub fn campo(&self, k: &str) -> String {
        self.bruto
            .get(k)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    }
}

/// O registro de um processo (o dict `r` do `consolidar`).
#[derive(Clone, Debug, Default)]
pub struct Registro {
    pub numero: String,
    pub itd: String,
    pub tipo_doc: String,
    pub url: String,
    pub emissao: Option<NaiveDateTime>,
    pub prioridade: String,
    pub assunto: String,
    pub tipo: String,
    pub quadro: String,
    pub fase: String,
    pub etapa: String,
    pub etapas_quadro: Vec<String>,
    pub setor_etapa: String,
    pub permanencia: String,
    pub entrada: Option<NaiveDateTime>,
    pub entrada_fonte: String,
    pub para: String,
    pub ult_mov: Option<NaiveDateTime>,
    pub ult_data: Option<NaiveDateTime>,
    pub ult_de: String,
    pub ult_para: String,
    pub ult_texto: String,
    pub situacao: String,
    pub status_setor: String,
    pub pai: String,
    pub derivados: String,
    pub caixa: String,
    pub transicoes: Vec<Transicao>,
    pub despachos: Vec<Value>,
    pub tem_detalhe: bool,
    pub obs: Vec<String>,
    pub etapa_meu_setor: Option<bool>,
    pub dias_etapa: Option<i64>,
    pub dias_sem_mov: Option<i64>,
    pub n_mudancas: i64,
    pub historico: String,
    pub cita_alvo: Option<bool>,
    /// índices das transições no período do filtro de mudança
    pub trans_periodo: Vec<usize>,
}

/// Valor de um campo, para ordenar e escrever.
#[derive(Clone, Debug, PartialEq)]
pub enum Campo {
    Nada,
    Texto(String),
    Data(NaiveDateTime),
    Int(i64),
    Bool(bool),
}

impl Registro {
    pub fn campo(&self, nome: &str) -> Campo {
        let t = |s: &String| Campo::Texto(s.clone());
        let d = |x: Option<NaiveDateTime>| x.map(Campo::Data).unwrap_or(Campo::Nada);
        let i = |x: Option<i64>| x.map(Campo::Int).unwrap_or(Campo::Nada);
        let b = |x: Option<bool>| x.map(Campo::Bool).unwrap_or(Campo::Nada);
        match nome {
            "numero" => t(&self.numero),
            "itd" => t(&self.itd),
            "tipo_doc" => t(&self.tipo_doc),
            "url" => t(&self.url),
            "emissao" => d(self.emissao),
            "prioridade" => t(&self.prioridade),
            "assunto" => t(&self.assunto),
            "tipo" => t(&self.tipo),
            "quadro" => t(&self.quadro),
            "fase" => t(&self.fase),
            "etapa" => t(&self.etapa),
            "setor_etapa" => t(&self.setor_etapa),
            "permanencia" => t(&self.permanencia),
            "entrada" => d(self.entrada),
            "entrada_fonte" => t(&self.entrada_fonte),
            "para" => t(&self.para),
            "ult_mov" => d(self.ult_mov),
            "ult_data" => d(self.ult_data),
            "ult_de" => t(&self.ult_de),
            "ult_para" => t(&self.ult_para),
            "ult_texto" => t(&self.ult_texto),
            "situacao" => t(&self.situacao),
            "status_setor" => t(&self.status_setor),
            "pai" => t(&self.pai),
            "derivados" => t(&self.derivados),
            "caixa" => t(&self.caixa),
            "tem_detalhe" => Campo::Bool(self.tem_detalhe),
            "etapa_meu_setor" => b(self.etapa_meu_setor),
            "dias_etapa" => i(self.dias_etapa),
            "dias_sem_mov" => i(self.dias_sem_mov),
            "n_mudancas" => Campo::Int(self.n_mudancas),
            "historico" => t(&self.historico),
            "cita_alvo" => b(self.cita_alvo),
            _ => Campo::Nada,
        }
    }
}

fn s(v: Option<&Value>, k: &str) -> String {
    v.and_then(|v| v.get(k))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

/// `a or b` entre textos.
fn ou(a: String, b: String) -> String {
    if a.is_empty() { b } else { a }
}

pub fn cita_alvo(despachos: &[Value], alvo: &str, pessoas: &[String]) -> bool {
    let a = normcod(alvo);
    for d in despachos {
        let b = d.get("bruto").and_then(Value::as_str).unwrap_or("");
        if !a.is_empty() && b.contains(&a) {
            return true;
        }
        if !a.is_empty() {
            // badge truncado ("SPOT-DCL-GP-CPPF..."): prefixo longo do alvo também vale
            let alvo_txt = format!("{} {}", s(Some(d), "setor_de"), s(Some(d), "para"));
            for m in re!(r"(?i)([A-Z0-9\-/]{8,})(\.\.\.|…)").captures_iter(&alvo_txt) {
                if a.starts_with(&normcod(&m[1])) {
                    return true;
                }
            }
        }
        if pessoas.iter().any(|p| {
            let n = normcod(p);
            !n.is_empty() && b.contains(&n)
        }) {
            return true;
        }
    }
    false
}

pub fn posicao_etapa(etapa: &str, titulos: &[String]) -> Option<f64> {
    let ts: Vec<String> = titulos.iter().map(|t| normtxt(t)).collect();
    let n = normtxt(etapa);
    if let Some(i) = ts.iter().position(|t| *t == n) {
        return Some(i as f64);
    }
    let partes: Vec<&str> = re!(r"\s+[–-]\s+").splitn(etapa, 2).collect(); // atípico "Saiu – Próxima"
    if partes.len() == 2
        && let Some(i) = ts.iter().position(|t| *t == normtxt(partes[0]))
    {
        return Some(i as f64 + 0.5);
    }
    None
}

pub fn indice_alvo(alvo: &str, titulos: &[String]) -> Option<f64> {
    let a = normtxt(alvo);
    titulos
        .iter()
        .position(|t| normtxt(t) == a)
        .or_else(|| titulos.iter().position(|t| normtxt(t).contains(&a)))
        .map(|i| i as f64)
}

pub fn consolidar(
    item: Option<&Value>,
    det: Option<&Value>,
    agora: NaiveDateTime,
    op: &Opcoes,
) -> Registro {
    let item = item.filter(|v| v.is_object());
    let det = det.filter(|v| v.as_object().is_some_and(|o| !o.is_empty()));
    let desp: Vec<Value> = det
        .and_then(|d| d.get("despachos"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let ult = desp.last();
    let trans: Vec<Transicao> = det
        .and_then(|d| d.get("transicoes"))
        .and_then(Value::as_array)
        .map(|v| {
            v.iter()
                .map(|t| Transicao {
                    data: de_iso_v(t.get("data")),
                    de: s(Some(t), "de"),
                    para: s(Some(t), "para"),
                    mesma_etapa: t
                        .get("mesma_etapa")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    estimada: t.get("estimada").and_then(Value::as_bool).unwrap_or(false),
                    bruto: t.clone(),
                })
                .collect()
        })
        .unwrap_or_default();
    let lista_s = |v: Option<&Value>, k: &str| -> Vec<String> {
        v.and_then(|v| v.get(k))
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    };
    let item_tem_numero = !s(item, "numero").is_empty();
    let mut r = Registro {
        numero: ou(s(det, "numero"), s(item, "numero")),
        itd: ou(s(det, "itd"), s(item, "itd")),
        tipo_doc: ou(s(det, "tipo_doc"), s(item, "tipo_doc")),
        url: ou(s(item, "url"), s(det, "url")),
        emissao: de_iso(&s(item, "emissao")).or_else(|| de_iso(&s(det, "emissao"))),
        // a listagem prevalece nos campos que o pré-filtro julga por ela
        prioridade: if item_tem_numero {
            s(item, "prioridade")
        } else {
            s(det, "prioridade")
        },
        assunto: ou(s(det, "assunto"), s(item, "assunto")),
        tipo: ou(s(det, "tipo"), s(item, "tipo")),
        quadro: s(det, "quadro"),
        fase: s(det, "fase"),
        etapa: s(det, "etapa"),
        etapas_quadro: lista_s(det, "etapas_quadro"),
        setor_etapa: s(det, "setor_etapa"),
        permanencia: s(det, "permanencia"),
        entrada: de_iso(&s(det, "entrada")),
        entrada_fonte: s(det, "entrada_fonte"),
        para: ou(s(item, "para"), s(ult, "para")),
        ult_mov: de_iso(&s(item, "ult_mov")).or_else(|| de_iso(&s(ult, "data"))),
        ult_data: de_iso(&s(ult, "data")),
        ult_de: [s(ult, "autor"), s(ult, "setor_de")]
            .into_iter()
            .filter(|x| !x.is_empty())
            .collect::<Vec<_>>()
            .join(" — "),
        ult_para: s(ult, "para"),
        ult_texto: corta(&s(ult, "texto"), 200),
        situacao: s(det, "situacao"),
        status_setor: s(det, "status_setor"),
        pai: s(det, "pai"),
        derivados: lista_s(det, "derivados").join(", "),
        caixa: s(item, "caixa"),
        transicoes: trans,
        despachos: desp.clone(),
        tem_detalhe: det.is_some(),
        obs: [s(det, "atipico"), s(det, "entrada_obs")]
            .into_iter()
            .filter(|x| !x.is_empty())
            .collect(),
        ..Default::default()
    };
    // Etapa reservada ao setor logado? (data-restrito_para_id_setor contém o id do setor da sessão)
    let ids = lista_s(det, "setor_etapa_ids");
    r.etapa_meu_setor = if !op.id_setor.is_empty() && !r.etapas_quadro.is_empty() {
        Some(ids.contains(&op.id_setor))
    } else {
        None
    };
    r.dias_etapa = r.entrada.map(|e| (agora.date() - e.date()).num_days());
    r.dias_sem_mov = r.ult_mov.map(|e| (agora.date() - e.date()).num_days());
    let mud: Vec<&Transicao> = r.transicoes.iter().filter(|t| !t.mesma_etapa).collect();
    r.n_mudancas = mud.len() as i64;
    r.historico = mud
        .iter()
        .filter_map(|t| {
            t.data.map(|d| {
                format!(
                    "{} {}{}",
                    d.format("%d/%m/%y %H:%M"),
                    t.para,
                    if t.estimada { " (estim.)" } else { "" }
                )
            })
        })
        .collect::<Vec<_>>()
        .join(" → ");
    let alvo =
        op.a.cita_setor
            .clone()
            .or_else(|| op.a.excecao_cita_setor.clone())
            .unwrap_or_default();
    if !alvo.is_empty() || !op.a.pessoa.is_empty() {
        let n = (op.a.ultimos_despachos.max(1) as usize).min(GUARDAR_DESPACHOS);
        r.cita_alvo = Some(
            !desp.is_empty()
                && cita_alvo(&desp[desp.len().saturating_sub(n)..], &alvo, &op.a.pessoa),
        );
    }
    r
}

pub type Pred = Box<dyn Fn(&mut Registro) -> bool + Send + Sync>;

pub struct Filtro {
    pub descricao: String,
    /// "posicao" (afastável pela --excecao-cita-setor) | "geral"
    pub grupo: &'static str,
    /// "lista" (avaliável só com a listagem) | "detalhe"
    pub fonte: &'static str,
    pub pred: Pred,
}

fn periodo(de: Option<NaiveDateTime>, ate: Option<NaiveDateTime>) -> String {
    let f = |d: NaiveDateTime| d.format("%d/%m/%Y").to_string();
    match (de, ate) {
        (Some(d), Some(a)) => format!("entre {} e {}", f(d), f(a)),
        (Some(d), None) => format!("desde {}", f(d)),
        (None, Some(a)) => format!("até {}", f(a)),
        (None, None) => String::new(),
    }
}

fn no_intervalo(
    v: Option<NaiveDateTime>,
    de: Option<NaiveDateTime>,
    ate: Option<NaiveDateTime>,
) -> bool {
    v.is_some_and(|v| de.is_none_or(|d| v >= d) && ate.is_none_or(|a| v <= a))
}

pub fn montar_filtros(op: &Opcoes) -> Vec<Filtro> {
    let a = &op.a;
    let mut f: Vec<Filtro> = Vec::new();
    let mut add = |descricao: String, grupo: &'static str, fonte: &'static str, pred: Pred| {
        f.push(Filtro {
            descricao,
            grupo,
            fonte,
            pred,
        })
    };
    let exata = a.etapa_exata;
    let casa = move |v: &str, p: &[String]| if exata { igual(v, p) } else { contem(v, p) };
    if !a.etapa.is_empty() {
        let e = a.etapa.clone();
        add(
            format!("etapa {} {}", if exata { "=" } else { "∋" }, e.join(" OU ")),
            "posicao",
            "detalhe",
            Box::new(move |r| casa(&r.etapa, &e)),
        );
    }
    if !a.excluir_etapa.is_empty() {
        let e = a.excluir_etapa.clone();
        add(
            format!("etapa ∌ {}", e.join(" / ")),
            "posicao",
            "detalhe",
            Box::new(move |r| !casa(&r.etapa, &e)),
        );
    }
    if let Some(fase) = a.fase.clone() {
        add(
            format!("fase = {fase}"),
            "posicao",
            "detalhe",
            Box::new(move |r| igual(&r.fase, std::slice::from_ref(&fase))),
        );
    }
    if !a.quadro.is_empty() {
        let q = a.quadro.clone();
        add(
            format!("quadro/assunto ∋ {}", q.join(" OU ")),
            "posicao",
            "detalhe",
            Box::new(move |r| contem(&format!("{} | {}", r.quadro, r.tipo), &q)),
        );
    }
    if let Some(alvo) = a.antes_de.clone() {
        let d = format!("ainda não chegou a '{alvo}' (quadros sem essa etapa: só fase Elaboração)");
        add(
            d,
            "posicao",
            "detalhe",
            Box::new(move |r| match indice_alvo(&alvo, &r.etapas_quadro) {
                None => r.fase == "Elaboração", // quadro sem essa etapa: a fase preparatória precede a de abertura
                Some(ia) => posicao_etapa(&r.etapa, &r.etapas_quadro).is_some_and(|p| p < ia),
            }),
        );
    }
    if let Some(alvo) = a.a_partir_de.clone() {
        let d = format!("já chegou a '{alvo}' (etapa atual igual ou posterior no quadro)");
        add(
            d,
            "posicao",
            "detalhe",
            Box::new(move |r| {
                let ia = indice_alvo(&alvo, &r.etapas_quadro);
                let p = posicao_etapa(&r.etapa, &r.etapas_quadro);
                matches!((ia, p), (Some(ia), Some(p)) if p >= ia)
            }),
        );
    }
    if !a.passou_por.is_empty() {
        let e = a.passou_por.clone();
        add(
            format!("passou por {}", e.join(" OU ")),
            "posicao",
            "detalhe",
            Box::new(move |r| {
                contem(&r.etapa, &e) || r.transicoes.iter().any(|t| contem(&t.para, &e))
            }),
        );
    }
    if !a.nao_passou_por.is_empty() {
        let e = a.nao_passou_por.clone();
        add(
            format!("nunca passou por {}", e.join(" / ")),
            "posicao",
            "detalhe",
            Box::new(move |r| {
                !contem(&r.etapa, &e) && !r.transicoes.iter().any(|t| contem(&t.para, &e))
            }),
        );
    }
    if !a.setor_etapa.is_empty() {
        let e = a.setor_etapa.clone();
        add(
            format!("setor da etapa ∋ {}", e.join(" OU ")),
            "geral",
            "detalhe",
            Box::new(move |r| {
                e.iter()
                    .any(|s| r.setor_etapa.split(" | ").any(|p| casa_setor(p, s)))
            }),
        );
    }
    if let Some(n) = a.dias_etapa_min {
        add(
            format!("dias na etapa ≥ {n}"),
            "geral",
            "detalhe",
            Box::new(move |r| r.dias_etapa.is_some_and(|d| d >= n)),
        );
    }
    if let Some(n) = a.dias_etapa_max {
        add(
            format!("dias na etapa ≤ {n}"),
            "geral",
            "detalhe",
            Box::new(move |r| r.dias_etapa.is_some_and(|d| d <= n)),
        );
    }
    if op.entrada_de.is_some() || op.entrada_ate.is_some() {
        let (de, ate) = (op.entrada_de, op.entrada_ate);
        add(
            format!("entrou na etapa atual {}", periodo(de, ate)),
            "geral",
            "detalhe",
            Box::new(move |r| no_intervalo(r.entrada, de, ate)),
        );
    }
    if op.mudanca_de.is_some()
        || op.mudanca_ate.is_some()
        || !a.mudanca_para.is_empty()
        || !a.mudanca_origem.is_empty()
    {
        let (de, ate, para, orig) = (
            op.mudanca_de,
            op.mudanca_ate,
            a.mudanca_para.clone(),
            a.mudanca_origem.clone(),
        );
        let mut desc = "mudou de etapa".to_string();
        if de.is_some() || ate.is_some() {
            desc.push_str(&format!(" {}", periodo(de, ate)));
        }
        if !para.is_empty() {
            desc.push_str(&format!(" para {}", para.join(" OU ")));
        }
        if !orig.is_empty() {
            desc.push_str(&format!(" saindo de {}", orig.join(" OU ")));
        }
        add(
            desc,
            "geral",
            "detalhe",
            Box::new(move |r| {
                r.trans_periodo = r
                    .transicoes
                    .iter()
                    .enumerate()
                    .filter(|(_, t)| {
                        !t.mesma_etapa
                            && t.data.is_some()
                            && no_intervalo(t.data, de, ate)
                            && (para.is_empty() || contem(&t.para, &para))
                            && (orig.is_empty() || contem(&t.de, &orig))
                    })
                    .map(|(i, _)| i)
                    .collect();
                !r.trans_periodo.is_empty()
            }),
        );
    }
    if op.emissao_de.is_some() || op.emissao_ate.is_some() {
        let (de, ate) = (op.emissao_de, op.emissao_ate);
        add(
            format!("emissão {}", periodo(de, ate)),
            "geral",
            "lista",
            Box::new(move |r| no_intervalo(r.emissao, de, ate)),
        );
    }
    if op.ult_mov_de.is_some() || op.ult_mov_ate.is_some() {
        let (de, ate) = (op.ult_mov_de, op.ult_mov_ate);
        add(
            format!("última movimentação {}", periodo(de, ate)),
            "geral",
            "lista",
            Box::new(move |r| no_intervalo(r.ult_mov, de, ate)),
        );
    }
    if let Some(n) = a.sem_mov_min {
        add(
            format!("dias sem movimentação ≥ {n}"),
            "geral",
            "lista",
            Box::new(move |r| r.dias_sem_mov.is_some_and(|d| d >= n)),
        );
    }
    if let Some(n) = a.sem_mov_max {
        add(
            format!("dias sem movimentação ≤ {n}"),
            "geral",
            "lista",
            Box::new(move |r| r.dias_sem_mov.is_some_and(|d| d <= n)),
        );
    }
    if !a.prioridade.is_empty() {
        let p = a.prioridade.clone();
        add(
            format!("prioridade ∈ {}", p.join(", ")),
            "geral",
            "lista",
            Box::new(move |r| igual(&r.prioridade, &p)),
        );
    }
    if !a.com.is_empty() {
        let c = a.com.clone();
        add(
            format!("setores envolvidos (caixa) ∋ {}", c.join(" OU ")),
            "geral",
            "lista",
            Box::new(move |r| {
                c.iter()
                    .any(|s| r.para.split(" | ").any(|p| casa_setor(p, s)))
            }),
        );
    }
    if a.etapa_do_meu_setor {
        add(
            "etapa atual reservada ao setor logado (id em data-restrito_para_id_setor)".into(),
            "posicao",
            "detalhe",
            Box::new(|r| r.etapa_meu_setor == Some(true)),
        );
    }
    if !a.objeto.is_empty() {
        let o = a.objeto.clone();
        add(
            format!("objeto ∋ {}", o.join(" OU ")),
            "geral",
            "detalhe",
            Box::new(move |r| contem(&r.assunto, &o)),
        );
    }
    if !a.situacao.is_empty() {
        let o = a.situacao.clone();
        add(
            format!("situação ∋ {}", o.join(" OU ")),
            "geral",
            "detalhe",
            Box::new(move |r| contem(&r.situacao, &o)),
        );
    }
    if a.cita_setor.is_some() || (!a.pessoa.is_empty() && a.excecao_cita_setor.is_none()) {
        let quem: Vec<String> = std::iter::once(a.cita_setor.clone().unwrap_or_default())
            .chain(a.pessoa.iter().cloned())
            .filter(|x| !x.is_empty())
            .collect();
        add(
            format!(
                "últimos {} despacho(s) citam {}",
                a.ultimos_despachos,
                quem.join(" / ")
            ),
            "geral",
            "detalhe",
            Box::new(|r| r.cita_alvo == Some(true)),
        );
    }
    f
}

pub fn avaliar(r: &mut Registro, filtros: &[Filtro], excecao: bool, so_lista: bool) -> bool {
    let mut reprovou_posicao = false;
    for f in filtros {
        if so_lista && f.fonte != "lista" {
            continue;
        }
        if !(f.pred)(r) {
            if f.grupo == "posicao" && excecao {
                reprovou_posicao = true;
            } else {
                return false;
            }
        }
    }
    if reprovou_posicao {
        if r.cita_alvo == Some(true) {
            r.obs
                .push("incluído pela exceção: últimos despachos citam o setor-alvo".into());
            return true;
        }
        return false;
    }
    true
}

/// Chave de ordenação de um campo.
#[derive(Clone, Debug, PartialEq, PartialOrd)]
enum Chave {
    Etapa(i64, f64, String),
    Num(i64, i64),
    Int(i64),
    Data(NaiveDateTime),
    Texto(String),
    Bool(bool),
}

fn chave_campo(r: &Registro, campo: &str) -> Option<Chave> {
    match campo {
        "etapa" => {
            let p = posicao_etapa(&r.etapa, &r.etapas_quadro).unwrap_or(99.0);
            Some(Chave::Etapa(
                if r.fase == "Elaboração" { 0 } else { 1 },
                p,
                normtxt(&r.etapa),
            ))
        }
        "prioridade" => {
            let n = match normtxt(&r.prioridade).as_str() {
                "urgente" => 0,
                "alta" => 1,
                "media" => 2,
                "baixa" => 3,
                _ => 9,
            };
            Some(Chave::Int(n))
        }
        "numero" => {
            let num = if r.numero.is_empty() {
                "0/0"
            } else {
                &r.numero
            };
            let (n, a) = num.split_once('/').unwrap_or((num, "0"));
            Some(Chave::Num(
                a.parse().unwrap_or(0),
                n.replace('.', "").parse().unwrap_or(0),
            ))
        }
        _ => match r.campo(campo) {
            Campo::Nada => None,
            Campo::Texto(t) => {
                let n = normtxt(&t);
                if n.is_empty() {
                    None
                } else {
                    Some(Chave::Texto(n))
                }
            }
            Campo::Data(d) => Some(Chave::Data(d)),
            Campo::Int(i) => Some(Chave::Int(i)),
            Campo::Bool(b) => Some(Chave::Bool(b)),
        },
    }
}

pub fn ordenar(regs: Vec<Registro>, spec: &str) -> Vec<Registro> {
    let mut out = regs;
    let partes: Vec<&str> = spec
        .split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    for parte in partes.into_iter().rev() {
        let (campo, sentido) = parte.split_once(':').unwrap_or((parte, ""));
        let desc = sentido.to_lowercase() == "desc";
        let (mut com, sem): (Vec<Registro>, Vec<Registro>) = out
            .into_iter()
            .partition(|r| chave_campo(r, campo).is_some());
        com.sort_by(|x, y| {
            let (a, b) = (chave_campo(x, campo), chave_campo(y, campo));
            let o = a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal);
            if desc { o.reverse() } else { o }
        });
        out = com;
        out.extend(sem); // vazios sempre por último
    }
    out
}
