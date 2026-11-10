//! Leitura das páginas do 1Doc para a busca: listagem das caixas e página
//! do processo (kanban, despachos, eventos "Mudança de etapa").
//! Os registros são objetos JSON com as mesmas chaves do busca.py: é o que
//! vai para o cache (1doc_cache.json), compartilhado com a versão Python.

use std::sync::LazyLock;

use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, Utc};
use scraper::{ElementRef, Html, Node};
use serde_json::{Map, Value, json};

use crate::erro::{Erro, Res};
use crate::texto::{corta, normtxt, sem_acento};
use crate::{html, re};

pub const VERSAO: &str = "1.1"; // mudou o formato do detalhe: cache de versão anterior é descartado
pub const SEM_QUADRO: &str = " (sem quadro de etapas padrão)";
pub const GUARDAR_DESPACHOS: usize = 10;
pub const PRIORIDADES: [&str; 5] = ["Urgente", "Alta", "Média", "Media", "Baixa"];

// =============================================================================
// Datas (horário de Brasília, sem horário de verão desde 2019)
// =============================================================================

fn tz_br() -> chrono::FixedOffset {
    chrono::FixedOffset::west_opt(3 * 3600).unwrap()
}

pub fn agora_br() -> NaiveDateTime {
    let n = Utc::now().with_timezone(&tz_br()).naive_local();
    n.with_nanosecond(0).unwrap_or(n)
}

use chrono::Timelike;

/// `datetime.fromtimestamp(v, TZ_BR)` sem fuso.
pub fn de_timestamp(v: &str) -> Option<NaiveDateTime> {
    if v.is_empty() || !v.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    DateTime::from_timestamp(v.parse().ok()?, 0).map(|d| d.with_timezone(&tz_br()).naive_local())
}

/// `dt_br`: "DD/MM/AAAA[ HH:MM[:SS]]" em qualquer ponto do texto.
pub fn dt_br(s: &str) -> Option<NaiveDateTime> {
    let m = re!(r"(\d{2})/(\d{2})/(\d{4})(?:\s+(\d{2}):(\d{2})(?::(\d{2}))?)?").captures(s)?;
    let n = |i: usize| {
        m.get(i)
            .map(|x| x.as_str().parse::<u32>().unwrap_or(99))
            .unwrap_or(0)
    };
    NaiveDate::from_ymd_opt(m[3].parse().ok()?, n(2), n(1))?.and_hms_opt(n(4), n(5), n(6))
}

/// `dt.isoformat(timespec="seconds")` ou "".
pub fn iso(dt: Option<NaiveDateTime>) -> String {
    dt.map(|d| d.format("%Y-%m-%dT%H:%M:%S").to_string())
        .unwrap_or_default()
}

/// `de_iso`: ISO (com T ou espaço, só a data também) ou, se não, `dt_br`.
pub fn de_iso(s: &str) -> Option<NaiveDateTime> {
    if s.is_empty() {
        return None;
    }
    for f in [
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%dT%H:%M",
        "%Y-%m-%d %H:%M",
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S%.f",
    ] {
        if let Ok(d) = NaiveDateTime::parse_from_str(s, f) {
            return Some(d);
        }
    }
    if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return d.and_hms_opt(0, 0, 0);
    }
    dt_br(s)
}

/// `de_iso` de um valor JSON (string ou nulo).
pub fn de_iso_v(v: Option<&Value>) -> Option<NaiveDateTime> {
    v.and_then(Value::as_str).and_then(de_iso)
}

// =============================================================================
// Textos e números
// =============================================================================

/// `txt(el)`: texto com espaços colapsados ("" se não houver elemento).
pub fn txt(el: Option<ElementRef<'_>>) -> String {
    el.map(|e| {
        re!(r"\s+")
            .replace_all(&html::texto(e, " "), " ")
            .trim()
            .to_string()
    })
    .unwrap_or_default()
}

fn t1(el: ElementRef<'_>) -> String {
    txt(Some(el))
}

/// Primeiro nó de texto direto (o objeto vem antes do badge do assunto).
pub fn primeiro_texto(el: Option<ElementRef<'_>>) -> String {
    let Some(el) = el else { return String::new() };
    for n in el.children() {
        if let Node::Text(t) = n.value()
            && !t.trim().is_empty()
        {
            return re!(r"\s+").replace_all(t, " ").trim().to_string();
        }
    }
    String::new()
}

/// Primeira linha de texto de `el` depois de remover os elementos `seletor`.
pub fn texto_sem(el: Option<ElementRef<'_>>, seletor: &scraper::Selector) -> String {
    let Some(el) = el else { return String::new() };
    let t = html::texto_sem(el, seletor, "\n");
    t.split('\n')
        .map(str::trim)
        .find(|s| !s.is_empty())
        .map(|s| corta(s, 60))
        .unwrap_or_default()
}

/// Nome completo (span.pp[data-content]); o texto visível é abreviado ('Maria R.').
pub fn nome_pessoa(el: Option<ElementRef<'_>>) -> String {
    let Some(e) = el else { return String::new() };
    if let Some(pp) = html::um(e, sel!("span.pp[data-content]")) {
        let c = html::attr_ou_vazio(pp, "data-content");
        if !c.trim().is_empty() {
            return re!(r"\s+").replace_all(c, " ").trim().to_string();
        }
    }
    texto_sem(el, sel!(".badge, i, .label"))
}

/// 'SPOT &raquo; SPOT-DCL &raquo; SPOT-DCL-GP-CPPFI' -> 'SPOT-DCL-GP-CPPFI'.
fn ultimo_segmento(caminho: &str) -> String {
    re!(r"\s*(?:»|&raquo;)\s*")
        .split(caminho.trim())
        .last()
        .unwrap_or("")
        .trim()
        .to_string()
}

fn titulo_ou_original(b: ElementRef<'_>) -> String {
    html::attr(b, "data-original-title")
        .filter(|t| !t.is_empty())
        .or_else(|| html::attr(b, "title").filter(|t| !t.is_empty()))
        .unwrap_or("")
        .to_string()
}

/// Códigos de setor dos badges, sem perder os escondidos.
pub fn codigos_badges(el: Option<ElementRef<'_>>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if let Some(el) = el {
        for b in el.select(sel!("span.badge")) {
            for c in codigos_badge(b) {
                if !c.is_empty() && !out.contains(&c) {
                    out.push(c);
                }
            }
        }
    }
    out
}

pub fn codigos_badge(b: ElementRef<'_>) -> Vec<String> {
    let t = t1(b);
    let tit = titulo_ou_original(b).trim().to_string();
    if re!(r"^\+\d+$").is_match(&t) && !tit.is_empty() {
        return re!(r"\s+/\s+")
            .split(&tit)
            .filter(|p| !p.trim().is_empty())
            .map(ultimo_segmento)
            .collect();
    }
    if re!(r"(\.\.\.|…)$").is_match(&t) && (tit.contains('»') || tit.contains("&raquo;")) {
        return vec![ultimo_segmento(&tit)];
    }
    vec![
        re!(r"\s+-\s+")
            .splitn(&t, 2)
            .next()
            .unwrap_or("")
            .trim()
            .to_string(),
    ]
}

/// Texto + nomes completos (span.pp[data-content]) + caminho completo dos badges (title).
fn texto_e_atributos(el: ElementRef<'_>) -> String {
    let mut extra: Vec<String> = el
        .select(sel!("span.pp[data-content]"))
        .map(|p| html::attr_ou_vazio(p, "data-content").to_string())
        .collect();
    extra.extend(el.select(sel!(".badge")).map(titulo_ou_original));
    let extra: Vec<String> = extra.into_iter().filter(|x| !x.is_empty()).collect();
    format!("{} {}", t1(el), extra.join(" "))
}

/// Comparação de códigos de setor e nomes: só alfanumérico maiúsculo.
pub fn normcod(s: &str) -> String {
    re!(r"[^A-Z0-9]")
        .replace_all(&sem_acento(s).to_uppercase(), "")
        .to_string()
}

/// Código inteiro ('SPOT-DCL-GP-CPPFI') ou só o último nível ('CPPFI').
pub fn casa_setor(codigo: &str, pedido: &str) -> bool {
    let (c, p) = (normcod(codigo), normcod(pedido));
    !p.is_empty() && (c == p || normcod(codigo.rsplit('-').next().unwrap_or("")) == p)
}

pub fn contem(valor: &str, padroes: &[String]) -> bool {
    let v = normtxt(valor);
    padroes.iter().any(|p| v.contains(&normtxt(p)))
}

pub fn igual(valor: &str, padroes: &[String]) -> bool {
    let v = normtxt(valor);
    padroes.iter().any(|p| normtxt(p) == v)
}

pub fn norm_numero(s: &str) -> Option<String> {
    let m = re!(r"^\s*(\d{1,3}(?:\.\d{3})+|\d{1,6})\s*/\s*(\d{4})\s*$").captures(s)?;
    let d = m[1].replace('.', "");
    let d = d.trim_start_matches('0');
    Some(format!(
        "{}/{}",
        crate::texto::milhar(if d.is_empty() { "0" } else { d }),
        &m[2]
    ))
}

static RE_NUM_LIVRE: LazyLock<fancy_regex::Regex> = LazyLock::new(|| {
    fancy_regex::Regex::new(r"(?<![\d/.])(\d{1,3}(?:\.\d{3})+|\d{1,6})\s*/\s*(20\d{2})(?!\d)")
        .unwrap()
});

pub fn numeros_de_texto(texto: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for m in RE_NUM_LIVRE.captures_iter(texto).flatten() {
        if let Some(n) = norm_numero(&format!("{}/{}", &m[1], &m[2]))
            && !out.contains(&n)
        {
            out.push(n);
        }
    }
    out
}

pub fn numeros_de_arquivo(caminho: &str) -> Res<Vec<String>> {
    let p = std::path::Path::new(caminho);
    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    if ext == "xlsx" || ext == "xlsm" {
        let wb = umya_spreadsheet::reader::xlsx::read(p)
            .map_err(|e| Erro::outro(format!("{caminho}: {e}")))?;
        let mut partes = Vec::new();
        for ws in wb.sheet_collection() {
            let (cols, linhas) = ws.highest_column_and_row();
            for r in 1..=linhas {
                for c in 1..=cols {
                    if let Some(cel) = ws.cell((c, r)) {
                        let v = cel.value().to_string();
                        if !v.is_empty() {
                            partes.push(v);
                        }
                    }
                }
            }
        }
        return Ok(numeros_de_texto(&partes.join("\n")));
    }
    let bytes = std::fs::read(p).map_err(|e| Erro::outro(format!("{caminho}: {e}")))?;
    Ok(numeros_de_texto(&String::from_utf8_lossy(&bytes)))
}

/// Permanência na etapa ('Esta aqui 3 dias'): (delta, tolerância em dias, texto).
#[derive(Clone, Debug)]
pub struct Permanencia {
    pub delta: Duration,
    pub tol: i64,
    pub texto: String,
}

pub fn permanencia(t: &str) -> Option<Permanencia> {
    let p = |delta: Duration, tol: i64| {
        Some(Permanencia {
            delta,
            tol,
            texto: t.to_string(),
        })
    };
    let Some(m) = re!(r"(?i)(\d+)\s*(minuto|hora|dia|semana|m[eê]s|meses|ano)").captures(t) else {
        if re!(r"(?i)segundo|instante|agora").is_match(t) {
            return p(Duration::zero(), 1);
        }
        return None;
    };
    let n: i64 = m[1].parse().unwrap_or(0);
    let u = sem_acento(&m[2].to_lowercase());
    if u.starts_with("minuto") {
        p(Duration::minutes(n), 1)
    } else if u.starts_with("hora") {
        p(Duration::hours(n), 1)
    } else if u.starts_with("semana") {
        p(Duration::weeks(n), 7)
    } else if u.starts_with("mes") {
        p(Duration::days(30 * n), 31)
    } else if u.starts_with("ano") {
        p(Duration::days(365 * n), 366)
    } else {
        p(Duration::days(n), 1)
    }
}

// =============================================================================
// Listagem das caixas
// =============================================================================

/// Total de páginas ('Página 1/21') e total de itens da caixa
/// (`num_tab_entrada').html('(210)')`).
pub fn meta_listagem(
    doc: &Html,
    fonte: &str,
    caixa: &str,
) -> (Option<i64>, Option<i64>, Option<i64>) {
    let texto = html::texto_doc(doc, " ");
    let m = re!(r"P[áa]gina\s+(\d+)\s*/\s*(\d+)").captures(&texto);
    let mut total = None;
    for c in re!(r#"num_tab_(\w+)['"]\)\.html\(['"]\((\d[\d.]*)\)"#).captures_iter(fonte) {
        if &c[1] == caixa {
            total = c[2].replace('.', "").parse().ok();
        }
    }
    (
        m.as_ref().and_then(|m| m[1].parse().ok()),
        m.as_ref().and_then(|m| m[2].parse().ok()),
        total,
    )
}

fn consulta(href: &str) -> Vec<(String, String)> {
    let q = href.split_once('?').map(|(_, q)| q).unwrap_or("");
    let q = q.split('#').next().unwrap_or("");
    url::form_urlencoded::parse(q.as_bytes())
        .into_owned()
        .filter(|(_, v)| !v.is_empty())
        .collect()
}

fn param(q: &[(String, String)], k: &str) -> String {
    q.iter()
        .find(|(c, _)| c == k)
        .map(|(_, v)| v.clone())
        .unwrap_or_default()
}

fn filhos_td<'a>(tr: ElementRef<'a>) -> Vec<ElementRef<'a>> {
    tr.children()
        .filter_map(ElementRef::wrap)
        .filter(|e| html::nome_tag(*e) == "td")
        .collect()
}

pub fn parse_listagem(doc: &Html, rotulo_caixa: &str, itds: &[String], base: &str) -> Vec<Value> {
    let mut out = Vec::new();
    for a in doc.select(sel!("a.link_emissao_a")) {
        let href = html::attr_ou_vazio(a, "href");
        let q = consulta(href);
        let (itd, hash) = (param(&q, "itd"), param(&q, "hash"));
        if !itds.is_empty() && !itds.contains(&itd) {
            continue;
        }
        let texto_a = t1(a);
        let Some(m) = re!(r"^(.*?)\s*([\d.]+/\d{4})\s+(\d{2}/\d{2}/\d{4})\s+(\d{2}:\d{2})")
            .captures(&texto_a)
        else {
            continue;
        };
        let Some(tr) = html::ancestral_tag(a, "tr") else {
            continue;
        };
        let tds = filhos_td(tr);
        let td_para = html::um(tr, sel!("td.celula_para")).or_else(|| tds.get(3).copied());
        let resumo = html::um(tr, sel!(".resumo"));
        let td_ass = match resumo {
            Some(r) => html::ancestral_tag(r, "td"),
            None => tds.get(4).copied(),
        };
        let tipo_el = td_ass.and_then(|t| {
            html::um(
                t,
                sel!(r#"span[title]:not([title="Prioridade"]), span.badge"#),
            )
        });
        let rem = html::um(tr, sel!("div.td_remetente"));
        let setor_de = rem.and_then(|r| html::um(r, sel!("span.badge")));
        let mut prio = String::new();
        if let Some(td) = td_ass {
            for s in td.select(sel!("span.label")) {
                let t = t1(s);
                if PRIORIDADES.contains(&t.as_str()) {
                    prio = t;
                    break;
                }
            }
        }
        let ult = html::um(
            tr,
            sel!(r#"small[title^="Última movimentação"], small[title^="Ultima movimentação"]"#),
        );
        let tipo = tipo_el
            .map(|t| {
                let v = titulo_ou_original(t);
                if v.is_empty() { t1(t) } else { v }
            })
            .unwrap_or_default();
        let mut o = Map::new();
        o.insert("numero".into(), json!(&m[2]));
        o.insert("tipo_doc".into(), json!(m[1].trim()));
        o.insert("itd".into(), json!(itd));
        o.insert("hash".into(), json!(hash));
        o.insert(
            "emissao".into(),
            json!(iso(dt_br(&format!("{} {}", &m[3], &m[4])))),
        );
        o.insert("assunto".into(), json!(primeiro_texto(td_ass)));
        o.insert("tipo".into(), json!(tipo));
        o.insert("prioridade".into(), json!(prio));
        o.insert(
            "de".into(),
            json!(if rem.is_some() {
                nome_pessoa(rem)
            } else {
                String::new()
            }),
        );
        o.insert("setor_de".into(), json!(txt(setor_de)));
        // Coluna "Para" da caixa = todos os setores envolvidos, não o setor atual.
        o.insert(
            "para".into(),
            json!(if td_para.is_some() {
                codigos_badges(td_para).join(" | ")
            } else {
                String::new()
            }),
        );
        o.insert(
            "ult_mov".into(),
            json!(
                ult.map(|u| iso(dt_br(html::attr_ou_vazio(u, "title"))))
                    .unwrap_or_default()
            ),
        );
        o.insert("resumo".into(), json!(corta(&txt(resumo), 160)));
        o.insert("caixa".into(), json!(rotulo_caixa));
        o.insert(
            "url".into(),
            json!(if hash.is_empty() {
                String::new()
            } else {
                format!("{base}?pg=doc/ver&hash={hash}&itd={itd}")
            }),
        );
        out.push(Value::Object(o));
    }
    out
}

// =============================================================================
// Página do processo
// =============================================================================

#[derive(Clone, Debug, PartialEq)]
struct Bloco {
    titulo: String,
    itens: String,
    atual: bool,
    setores: Vec<String>,
    ids_setor: Vec<String>,
}

fn kanban(doc: &Html) -> Vec<Bloco> {
    let mut blocos: Vec<Bloco> = doc
        .select(sel!(".kanban_estagios"))
        .map(|b| Bloco {
            titulo: txt(html::um(b, sel!(".kanban_titulo"))),
            itens: txt(html::um(b, sel!(".kanban_itens"))),
            atual: html::attr(b, "data-current") == Some("true"),
            setores: b
                .select(sel!(".kanban_footer .badge_env"))
                .map(t1)
                .collect(),
            ids_setor: re!(r"\d+")
                .find_iter(html::attr_ou_vazio(b, "data-restrito_para_id_setor"))
                .map(|m| m.as_str().to_string())
                .collect(),
        })
        .collect();
    // O 1Doc renderiza o quadro duas vezes; desduplica pelos TÍTULOS.
    let t: Vec<&String> = blocos.iter().map(|b| &b.titulo).collect();
    let meia = t.len() / 2;
    if t.len() >= 2 && t.len().is_multiple_of(2) && t[..meia] == t[meia..] {
        let (a, o) = blocos.split_at(meia);
        blocos = a
            .iter()
            .zip(o)
            .map(|(a, o)| Bloco {
                titulo: a.titulo.clone(),
                itens: if a.itens.is_empty() {
                    o.itens.clone()
                } else {
                    a.itens.clone()
                },
                atual: a.atual || o.atual,
                setores: if a.setores.is_empty() {
                    o.setores.clone()
                } else {
                    a.setores.clone()
                },
                ids_setor: if a.ids_setor.is_empty() {
                    o.ids_setor.clone()
                } else {
                    a.ids_setor.clone()
                },
            })
            .collect();
    }
    blocos
}

/// Carimbo unix do contêiner .timeline_conteudo (precisão de segundos).
fn data_tc(el: ElementRef<'_>) -> Option<NaiveDateTime> {
    let tc = if html::tem_classe(el, "timeline_conteudo") {
        Some(el)
    } else {
        html::ancestral(el, |e| html::tem_classe(e, "timeline_conteudo"))
    };
    de_timestamp(
        tc.map(|t| html::attr_ou_vazio(t, "data-datamk"))
            .unwrap_or(""),
    )
}

struct Evento {
    data: Option<NaiveDateTime>,
    de: String,
    para: String,
    autor: String,
    setor: String,
}

/// Linha do tempo: mudanças de etapa e processos derivados.
fn eventos(doc: &Html) -> (Vec<Evento>, Vec<String>) {
    struct Bruto {
        i: usize,
        data: Option<NaiveDateTime>,
        tipo: &'static str,
        etapa: String,
        autor: String,
        setor: String,
    }
    let (mut brutos, mut derivados) = (Vec::new(), Vec::<String>::new());
    for (i, m) in doc.select(sel!(".desp_notificacao .marcou")).enumerate() {
        let quem = txt(html::um(m, sel!(".marcou_quem")));
        if let Some(md) = re!(r"(?i)emitiu\s+o\s+(.+?)\s+([\d.]+/\d{4})\s+a partir deste documento")
            .captures(&quem)
            && let Some(n) = norm_numero(&md[2])
            && !derivados.contains(&n)
        {
            derivados.push(n);
        }
        if !html::tem_classe(m, "marcou_fast-forward") {
            continue;
        }
        let forte = Some(txt(html::um(m, sel!(".marcou_quem strong"))))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| {
                re!(r"^(mudou da etapa|para)\s*|\s*\.$")
                    .replace_all(&quem, "")
                    .to_string()
            });
        let dt = data_tc(m).or_else(|| dt_br(&txt(html::um(m, sel!(".despacho_data")))));
        let tipo = if re!(r"(?i)^mudou da etapa").is_match(&quem) {
            "de"
        } else if re!(r"(?i)^para\b").is_match(&quem) {
            "para"
        } else {
            "?"
        };
        brutos.push(Bruto {
            i,
            data: dt,
            tipo,
            etapa: forte,
            autor: txt(html::um(m, sel!(".marcou_nome"))),
            setor: txt(html::um(m, sel!(".badge_env"))),
        });
    }
    brutos.sort_by_key(|e| (e.data.unwrap_or(NaiveDateTime::MIN), e.i));
    let (mut mud, mut pend): (Vec<Evento>, Option<Evento>) = (Vec::new(), None);
    for e in brutos {
        if e.tipo == "de" {
            if let Some(p) = pend.take() {
                mud.push(p); // 'de' sem 'para' correspondente: destino vazio
            }
            pend = Some(Evento {
                data: e.data,
                de: e.etapa,
                para: String::new(),
                autor: e.autor,
                setor: e.setor,
            });
        } else if e.tipo == "para" {
            match pend.take() {
                Some(mut p) if p.data == e.data => {
                    p.para = e.etapa;
                    mud.push(p);
                }
                outro => {
                    if let Some(p) = outro {
                        mud.push(p);
                    }
                    mud.push(Evento {
                        data: e.data,
                        de: String::new(),
                        para: e.etapa,
                        autor: e.autor,
                        setor: e.setor,
                    });
                }
            }
        }
    }
    if let Some(p) = pend {
        mud.push(p);
    }
    (mud, derivados)
}

fn despachos(doc: &Html) -> Vec<Value> {
    let mut out: Vec<(String, usize, Map<String, Value>)> = Vec::new();
    for (i, tb) in doc.select(sel!("table.despacho")).enumerate() {
        let th = html::um(tb, sel!("th")).unwrap_or(tb);
        let d_el = html::um(th, sel!(r#".despacho_data[title^="Em "]"#))
            .or_else(|| html::um(th, sel!(".despacho_data")));
        let mut dt = d_el.and_then(|d| {
            let t = html::attr_ou_vazio(d, "title");
            dt_br(&if t.is_empty() { t1(d) } else { t.to_string() })
        });
        if dt.is_none() {
            dt = data_tc(tb); // recurso: carimbo unix do contêiner da linha do tempo
        }
        let rotulo = match html::um(th, sel!(".despacho_tit")) {
            Some(tit) => match html::um(tit, sel!(".icon-fast-forward")) {
                Some(ff) => txt(ff.parent().and_then(ElementRef::wrap)),
                None => t1(tit),
            },
            None => String::new(),
        };
        let forte = txt(html::um(th, sel!("strong")));
        let mnum = re!(r"Despacho\s+(\d+)")
            .captures(&forte)
            .map(|m| m[1].to_string())
            .unwrap_or_default();
        let (rf_de, rf_para) = (html::um(tb, sel!(".rf_de")), html::um(tb, sel!(".rf_para")));
        let setor_de = rf_de.and_then(|r| html::um(r, sel!(".badge_env")));
        let mut paras = Vec::new();
        if let Some(rp) = rf_para {
            for b in rp.select(sel!(".badge_env")) {
                let cod = codigos_badge(b).into_iter().next().unwrap_or_else(|| t1(b));
                let st = html::um(b, sel!("i[title]"))
                    .map(|i| html::attr_ou_vazio(i, "title"))
                    .filter(|t| !t.is_empty());
                paras.push(match st {
                    Some(s) => format!("{cod} ({s})"),
                    None => cod,
                });
            }
        }
        let corpo = html::um(tb, sel!(".texto_original"))
            .or_else(|| html::um(tb, sel!("td.celula_despacho")));
        let lab = html::um(th, sel!(".despacho_data .label"));
        let para = if !paras.is_empty() {
            paras.join(" | ")
        } else if rf_para.is_some() {
            "(envolvidos internos)".into()
        } else {
            String::new()
        };
        let mut o = Map::new();
        let data = iso(dt);
        o.insert("data".into(), json!(data));
        o.insert("n".into(), json!(mnum));
        o.insert("situacao".into(), json!(txt(lab)));
        o.insert("rotulo_etapa".into(), json!(rotulo));
        o.insert(
            "autor".into(),
            json!(if rf_de.is_some() {
                nome_pessoa(rf_de)
            } else {
                String::new()
            }),
        );
        o.insert("setor_de".into(), json!(txt(setor_de)));
        o.insert("para".into(), json!(para));
        o.insert("texto".into(), json!(corta(&txt(corpo), 400)));
        o.insert(
            "bruto".into(),
            json!(corta(&normcod(&texto_e_atributos(tb)), 6000)),
        );
        out.push((data, i, o));
    }
    out.sort_by(|a, b| (&a.0, a.1).cmp(&(&b.0, b.1)));
    out.into_iter().map(|(_, _, o)| Value::Object(o)).collect()
}

/// O rótulo do despacho vem truncado ('Validação de ...'): casa por prefixo com o quadro.
pub fn resolver_etapa(rotulo: &str, titulos: &[String]) -> String {
    let r = rotulo.trim();
    let base = normtxt(&re!(r"(\.\.\.|…)\s*$").replace(r, ""))
        .trim()
        .to_string();
    if base.is_empty() {
        return r.to_string();
    }
    if let Some(t) = titulos.iter().find(|t| normtxt(t) == base) {
        return t.clone();
    }
    let cands: Vec<&String> = titulos
        .iter()
        .filter(|t| normtxt(t).starts_with(&base))
        .collect();
    if cands.len() == 1 {
        cands[0].clone()
    } else {
        r.to_string()
    }
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

#[allow(clippy::too_many_arguments)]
fn transicao(
    data: &str,
    de: &str,
    para: &str,
    rotulo: &str,
    despacho: &str,
    autor: &str,
    setor_de: &str,
    destinatario: &str,
    mesma_etapa: bool,
    estimada: bool,
    fonte: &str,
) -> Value {
    let mut o = Map::new();
    for (k, v) in [
        ("data", data),
        ("de", de),
        ("para", para),
        ("rotulo", rotulo),
        ("despacho", despacho),
        ("autor", autor),
        ("setor_de", setor_de),
        ("destinatario", destinatario),
    ] {
        o.insert(k.into(), json!(v));
    }
    o.insert("mesma_etapa".into(), json!(mesma_etapa));
    o.insert("estimada".into(), json!(estimada));
    o.insert("fonte".into(), json!(fonte));
    Value::Object(o)
}

/// Histórico de etapas: eventos "Mudança de etapa" (se houver) ou rótulos dos despachos.
fn transicoes(
    desp: &[Value],
    titulos: &[String],
    ev: &[Evento],
    emissao: Option<NaiveDateTime>,
) -> Vec<Value> {
    if ev.is_empty() {
        let (mut out, mut anterior) = (Vec::new(), String::new());
        for d in desp {
            let rot = s(d, "rotulo_etapa");
            if rot.is_empty() {
                continue;
            }
            let destino = resolver_etapa(&rot, titulos);
            let mesma = !anterior.is_empty() && normtxt(&anterior) == normtxt(&destino);
            out.push(transicao(
                &s(d, "data"),
                &anterior,
                &destino,
                &rot,
                &s(d, "n"),
                &s(d, "autor"),
                &s(d, "setor_de"),
                &s(d, "para"),
                mesma,
                false,
                "despacho",
            ));
            anterior = destino;
        }
        return out;
    }
    let mut out = Vec::new();
    let primeiro = &ev[0];
    if !primeiro.de.is_empty()
        && let Some(em) = emissao
    {
        // etapa inicial: entrou na emissão (não há evento para isso)
        out.push(transicao(
            &iso(Some(em)),
            "",
            &resolver_etapa(&primeiro.de, titulos),
            "",
            "",
            "",
            "",
            "",
            false,
            false,
            "emissão (etapa inicial)",
        ));
    }
    for e in ev {
        let para = if e.para.is_empty() {
            String::new()
        } else {
            resolver_etapa(&e.para, titulos)
        };
        let de = if e.de.is_empty() {
            String::new()
        } else {
            resolver_etapa(&e.de, titulos)
        };
        // despacho 'via etapa' gravado no mesmo instante (até 2 min) e com rótulo da etapa de destino
        let par = desp.iter().find(|d| {
            let dd = de_iso(&s(d, "data"));
            let rot = s(d, "rotulo_etapa");
            matches!((dd, e.data), (Some(dd), Some(ed)) if (dd - ed).num_seconds().abs() <= 120)
                && !rot.is_empty()
                && normtxt(&resolver_etapa(&rot, titulos)) == normtxt(&para)
        });
        let pv = |k: &str| par.map(|p| s(p, k)).unwrap_or_default();
        out.push(transicao(
            &iso(e.data),
            &de,
            &para,
            &pv("rotulo_etapa"),
            &pv("n"),
            &if e.autor.is_empty() {
                pv("autor")
            } else {
                e.autor.clone()
            },
            &if e.setor.is_empty() {
                pv("setor_de")
            } else {
                e.setor.clone()
            },
            &pv("para"),
            !de.is_empty() && normtxt(&de) == normtxt(&para),
            false,
            "evento",
        ));
    }
    out
}

/// Data de entrada na etapa atual → (data, fonte, observação).
fn entrada_etapa(
    etapa: &str,
    atipico: &str,
    titulos: &[String],
    trans: &[Value],
    perm: Option<&Permanencia>,
    emissao: Option<NaiveDateTime>,
    agora: NaiveDateTime,
) -> (Option<NaiveDateTime>, String, String) {
    let saiu = atipico.starts_with("sem etapa atual") && etapa.contains(" – ");
    let alvo = normtxt(if saiu {
        etapa.split_once(" – ").map(|x| x.1).unwrap_or("")
    } else {
        etapa
    });
    let est = perm.map(|p| agora - p.delta);
    let coerente = |dt: Option<NaiveDateTime>| match (perm, dt, est) {
        (Some(p), Some(dt), Some(est)) => {
            ((agora.date() - dt.date()).num_days() - (agora.date() - est.date()).num_days()).abs()
                <= p.tol
        }
        _ => true,
    };
    let (mut cand, mut fonte_cand) = (None, String::new());
    if !etapa.is_empty()
        && (atipico.is_empty() || saiu)
        && trans.last().is_some_and(|t| normtxt(&s(t, "para")) == alvo)
    {
        let mut i = trans.len() - 1;
        while i > 0 && normtxt(&s(&trans[i - 1], "para")) == alvo {
            i -= 1; // reenvio para a mesma etapa: vale o 1º
        }
        cand = de_iso(&s(&trans[i], "data"));
        fonte_cand = trans[i]
            .get("fonte")
            .and_then(Value::as_str)
            .unwrap_or("despacho")
            .to_string();
    }
    if let Some(c) = cand {
        if coerente(Some(c)) {
            return (Some(c), fonte_cand, String::new());
        }
        let p = perm.unwrap();
        return (
            est,
            "quadro (estimada)".into(),
            format!(
                "último registro de entrada na etapa ({}, {fonte_cand}) diverge do quadro ('{}'); prevaleceu o quadro",
                c.format("%d/%m/%Y %H:%M"),
                p.texto
            ),
        );
    }
    if !etapa.is_empty()
        && atipico.is_empty()
        && trans.is_empty()
        && titulos.first().is_some_and(|t| normtxt(t) == alvo)
        && emissao.is_some()
        && coerente(emissao)
    {
        return (emissao, "emissão (etapa inicial)".into(), String::new());
    }
    if let (Some(est), Some(p)) = (est, perm) {
        let tol = format!(" (±{} dia{})", p.tol, if p.tol > 1 { "s" } else { "" });
        return (
            Some(est),
            "quadro (estimada)".into(),
            format!(
                "sem despacho com rótulo desta etapa; estimada por '{}'{tol}",
                p.texto
            ),
        );
    }
    (None, String::new(), String::new())
}

pub fn sessao_caiu(fonte: &str, doc: &Html) -> bool {
    if let Some(e) = html::um_doc(doc, sel!("input#sis_usr_email"))
        && !html::attr_ou_vazio(e, "value").trim().is_empty()
    {
        return false;
    }
    re!(r#"(?i)name=["']?senha|formLogin|login\.php"#).is_match(fatia_bytes(fonte, 20000))
}

/// `html[:20000]` em caracteres.
pub fn fatia_bytes(s: &str, n: usize) -> &str {
    crate::texto::fatia(s, 0, n)
}

pub fn parse_detalhe(fonte: &str, agora: NaiveDateTime) -> Res<Value> {
    let doc = html::doc(fonte);
    if sessao_caiu(fonte, &doc) {
        return Err(Erro::SessaoExpirada(String::new()));
    }
    let numero = txt(html::um_doc(
        &doc,
        sel!(".emission_infos_item_atual .nd_num"),
    ));
    let tipo_doc = txt(html::um_doc(&doc, sel!(".emission_infos_item_atual .nd")));
    let pai = html::um_doc(&doc, sel!(".emission_infos_item_from small"));
    let tit = html::um_doc(&doc, sel!("h4.tit_assunto"));
    let tipo_el = tit.and_then(|t| html::um(t, sel!("span[title]")));
    let sit: Vec<ElementRef> = doc.select(sel!("small.situacao_geral")).collect();
    let prio_el = html::um_doc(
        &doc,
        sel!(r#".emission_infos_item_atual [title="Prioridade"]"#),
    );

    let iq = html::um_doc(&doc, sel!("a.ir_quadro"));
    let mut quadro = if iq.is_some() {
        re!(r"(?i)^\s*Ir ao quadro:?\s*")
            .replace(&txt(iq), "")
            .to_string()
    } else {
        String::new()
    };
    if quadro.is_empty() {
        let todo = html::texto_doc(&doc, "\n");
        quadro = re!(r"Ir ao quadro:\s*([^\n]{5,90})")
            .captures(&todo)
            .map(|m| m[1].trim().to_string())
            .unwrap_or_default();
    }
    let re_etiqueta = re!(
        r"(?i)Procedimento Licitat|Fase Preparat[óo]ria|Elabora[çc][ãa]o|Abertura|Aditivo|Documenta[çc][ãa]o Preparat"
    );
    let etiqueta = doc
        .select(sel!(".badge"))
        .map(t1)
        .find(|t| re_etiqueta.is_match(t))
        .unwrap_or_default();
    let tipo = match tipo_el {
        Some(t) => re!(r"(?i)^Assunto:\s*")
            .replace(html::attr_ou_vazio(t, "title"), "")
            .to_string(),
        None => etiqueta.clone(),
    };

    // ---- ETAPA (regras calibradas; não altere sem recalibrar) ----
    let bl = kanban(&doc);
    let titulos: Vec<String> = bl.iter().map(|b| b.titulo.clone()).collect();
    let (mut etapa, mut desde, mut setor_etapa, mut atipico) =
        (String::new(), String::new(), String::new(), String::new());
    if let Some(b) = bl
        .iter()
        .find(|b| re!(r"(?i)^Est[áa] aqui").is_match(&b.itens))
    {
        etapa = b.titulo.clone();
        desde = b.itens.clone();
        setor_etapa = b.setores.join(" | ");
    }
    if etapa.is_empty() && !bl.is_empty() {
        match bl
            .iter()
            .rposition(|b| re!(r"(?i)^Ficou aqui").is_match(&b.itens))
        {
            Some(i_f) => {
                etapa = match bl.get(i_f + 1) {
                    Some(prox) => format!("{} – {}", bl[i_f].titulo, prox.titulo),
                    None => bl[i_f].titulo.clone(),
                };
                atipico = format!("sem etapa atual; saiu de {}", bl[i_f].titulo);
            }
            None => {
                if let Some(b) = bl.iter().find(|b| b.atual) {
                    etapa = b.titulo.clone();
                    setor_etapa = b.setores.join(" | ");
                    atipico = "etapa só por data-current".into();
                }
            }
        }
    }
    let mut etapa_v = Value::String(etapa.clone());
    if etapa.is_empty() {
        etapa_v = if etiqueta.is_empty() {
            Value::Null
        } else {
            json!(format!("{etiqueta}{SEM_QUADRO}"))
        };
        etapa = etapa_v.as_str().unwrap_or("").to_string();
        atipico = if bl.is_empty() {
            "sem quadro de etapas".into()
        } else {
            "sem etapa identificável".into()
        };
    }

    // ---- FASE: discriminada pelo QUADRO ----
    let fonte_fase = [quadro.as_str(), etiqueta.as_str(), tipo.as_str()]
        .iter()
        .filter(|x| !x.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(" | ");
    let nomes = titulos.join(" | ");
    let fase = if re!(r"(?i)Abertura de Procedimento Licitat").is_match(&fonte_fase) {
        "Abertura"
    } else if re!(r"(?i)Fase Preparat[óo]ria").is_match(&fonte_fase) {
        "Elaboração"
    } else if re!(r"(?i)Valida[çc][ãa]o de Requisi[çc][ãa]o|Consulta Or[çc]ament[áa]ria|Controle Interno|COMPAC").is_match(&nomes) {
        "Abertura"
    } else {
        "Elaboração"
    };

    let h_el = html::um_doc(&doc, sel!("input#hash_emissao"))
        .or_else(|| html::um_doc(&doc, sel!("input#movimenta_hash_pai")))
        .or_else(|| html::um_doc(&doc, sel!("input#hash")));
    let hash = h_el
        .map(|h| html::attr_ou_vazio(h, "value").trim().to_string())
        .unwrap_or_default();

    let mut emissao = html::um_doc(&doc, sel!("#emissao_principal[data-datamk]"))
        .and_then(|e| de_timestamp(html::attr_ou_vazio(e, "data-datamk")));
    if emissao.is_none() {
        emissao = html::um_doc(&doc, sel!("div.horario span")).and_then(|h| dt_br(&t1(h)));
    }

    // Setores da etapa atual por id, para cruzar com o setor logado
    let ultimo_nome = etapa.rsplit(" – ").next().unwrap_or("");
    let bloco_atual = bl
        .iter()
        .find(|b| normtxt(&b.titulo) == normtxt(ultimo_nome));
    let ids_etapa = bloco_atual.map(|b| b.ids_setor.clone()).unwrap_or_default();
    if let Some(b) = bloco_atual
        && setor_etapa.is_empty()
    {
        setor_etapa = b.setores.join(" | ");
    }

    // Situação do processo para o setor logado (rótulos ao lado do número)
    let mut status: Vec<String> = Vec::new();
    for x in doc.select(sel!(".emission_infos_item_atual span.label")) {
        let t = t1(x);
        if html::attr(x, "title") != Some("Prioridade") && !t.is_empty() && !status.contains(&t) {
            status.push(t);
        }
    }

    let desp = despachos(&doc);
    let (ev, derivados) = eventos(&doc);
    let mut trans = transicoes(&desp, &titulos, &ev, emissao);
    let perm = if desde.is_empty() {
        None
    } else {
        permanencia(&desde)
    };
    let (entrada, entrada_fonte, entrada_obs) = entrada_etapa(
        &etapa,
        &atipico,
        &titulos,
        &trans,
        perm.as_ref(),
        emissao,
        agora,
    );
    // Transição sintética quando a entrada não veio de evento nem de despacho rotulado
    if entrada.is_some()
        && entrada_fonte != "despacho"
        && entrada_fonte != "evento"
        && !etapa.is_empty()
        && atipico.is_empty()
    {
        let ultimo = trans.last().map(|t| s(t, "para")).unwrap_or_default();
        if normtxt(&ultimo) != normtxt(&etapa) || entrada_obs.starts_with("último registro") {
            let de = if normtxt(&ultimo) == normtxt(&etapa) {
                String::new()
            } else {
                ultimo
            };
            trans.push(transicao(
                &iso(entrada),
                &de,
                &etapa,
                "",
                "",
                "",
                "",
                "",
                false,
                entrada_fonte != "emissão (etapa inicial)",
                &entrada_fonte,
            ));
        }
    }

    let situacao = match sit.len() {
        0 => String::new(),
        1 => t1(sit[0]),
        _ => t1(sit[1]),
    };
    let n_desp = desp.len();
    let guardados: Vec<Value> = desp[n_desp.saturating_sub(GUARDAR_DESPACHOS)..].to_vec();
    let mut o = Map::new();
    o.insert(
        "numero".into(),
        if numero.is_empty() {
            Value::Null
        } else {
            json!(numero)
        },
    );
    o.insert("hash".into(), json!(hash));
    o.insert("tipo_doc".into(), json!(tipo_doc));
    o.insert("pai".into(), json!(txt(pai)));
    o.insert("emissao".into(), json!(iso(emissao)));
    o.insert("assunto".into(), json!(primeiro_texto(tit)));
    o.insert("tipo".into(), json!(tipo));
    o.insert("situacao".into(), json!(situacao));
    o.insert("prioridade".into(), json!(txt(prio_el)));
    o.insert("quadro".into(), json!(quadro));
    o.insert("etapas_quadro".into(), json!(titulos));
    o.insert("etapa".into(), etapa_v);
    o.insert("fase".into(), json!(fase));
    o.insert("setor_etapa".into(), json!(setor_etapa));
    o.insert("setor_etapa_ids".into(), json!(ids_etapa));
    o.insert(
        "permanencia".into(),
        json!(re!(r"(?i)^Est[áa] aqui\s*").replace(&desde, "").to_string()),
    );
    o.insert("atipico".into(), json!(atipico));
    o.insert("entrada".into(), json!(iso(entrada)));
    o.insert("entrada_fonte".into(), json!(entrada_fonte));
    o.insert("entrada_obs".into(), json!(entrada_obs));
    o.insert("status_setor".into(), json!(status.join(" | ")));
    o.insert("derivados".into(), json!(derivados));
    o.insert("n_despachos".into(), json!(n_desp));
    o.insert("despachos".into(), Value::Array(guardados));
    o.insert("transicoes".into(), Value::Array(trans));
    o.insert("coletado_em".into(), json!(iso(Some(agora))));
    o.insert("versao".into(), json!(VERSAO));
    Ok(Value::Object(o))
}
