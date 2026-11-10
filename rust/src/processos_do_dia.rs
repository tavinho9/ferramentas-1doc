//! Extrator de processos do dia no 1Doc — varredura por requisição
//! (porta de processos_do_dia.py; as regras estão descritas lá).
//!
//! LISTAGEM: chama direto o endpoint que a própria interface usa para trocar
//! de aba e paginar (link.php?chamada=caixa), com `intervalo` filtrando a
//! data de EMISSÃO no servidor. DETALHE: a página de cada processo é baixada
//! por GET, em paralelo, e o kanban é lido do HTML.
//!
//! Critério de inclusão — todo processo exportado foi emitido no dia
//! escolhido e, além disso: COM QUADRO, se o dia é hoje, a etapa atual marca
//! menos de 24h (só vale o quadro do cabeçalho, nunca o do modelo oculto do
//! despacho); dia passado, entra com a etapa atual. SEM QUADRO: Proc.
//! Administrativo com etiqueta de fase de elaboração.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use chrono::{Local, NaiveDate, NaiveDateTime};
use regex::Regex;
use rust_xlsxwriter::{Color, Format, FormatAlign, FormatUnderline, Workbook};
use scraper::ElementRef;
use serde_json::{Map, Value, json};

use crate::ambiente::Ambiente;
use crate::erro::{Erro, Res};
use crate::http::{Repeticao, SessaoHttp};
use crate::sessao::{self, Credenciais};
use crate::texto::{corta, fatia, limpo};
use crate::{arquivos, config, datas, html, msg, paralelo, re};

pub const TIMEOUT: f64 = 15.0;

/// Caixas varridas. Com o filtro de data, cada uma custa ~1 requisição num dia comum.
pub const CAIXAS: [(&str, &str); 4] = [
    ("entrada", "Em aberto"),
    ("saida", "Caixa de saída"),
    ("arquivo", "Arquivados"),
    ("favoritos", "Favoritos"),
];
/// true = caixa do setor (meu=0); false = caixa pessoal (meu=1)
pub const CAIXA_DO_SETOR: bool = true;
/// "" = todos os tipos de documento; "3" = só Proc. Administrativo
pub const ITD: &str = "";
pub const WORKERS: usize = 4;
pub const PAUSA_REQ: f64 = 0.15;
pub const MAX_PAGINAS: u32 = 60;

pub const INCLUIR_SEM_QUADRO: bool = true;
pub const ETIQUETAS_ELABORACAO: &str = r"(?i)Elabora[çc][ãa]o de Document|Documenta[çc][ãa]o Preparat[óo]ria|Fase Preparat[óo]ria|Aditivo";
pub const ETIQUETAS_ABERTURA: &str = r"(?i)Abertura de Procedimento Licitat";
pub const OBJETO_DA_ABERTURA: bool = true;
pub const ID_DESCRICAO_OBJETO: &str = "campo_adicional_descricao_objeto";
/// Excel aceita até 32.767 caracteres por célula
pub const LIMITE_CELULA: usize = 32000;
pub const NOME_JSON: &str = "processos_do_dia.json";

pub fn nome_planilha(dia: NaiveDate) -> String {
    format!("processos novos {}.xlsx", dia.format("%d.%m"))
}

fn url_link(amb: &Ambiente) -> String {
    format!("{}/link.php", amb.url_1doc)
}

fn url_caixa(amb: &Ambiente) -> String {
    format!(
        "{}/?pg=painel/listar&meu={}&trocar=1&caixa=entrada",
        amb.url_1doc,
        if CAIXA_DO_SETOR { 0 } else { 1 }
    )
}

// ============================================================
// CRITÉRIOS DE TEMPO
// ============================================================

/// Texto relativo de movimentação ('3 horas', '45 minutos', '2 dias'). True
/// se indica menos de 1 dia. Formato desconhecido → True.
pub fn eh_sub_dia(texto: &str) -> bool {
    let t = texto.to_lowercase();
    let t = t.trim();
    if re!(r"\bdia(s)?\b|\bsemana(s)?\b|\bm[eê]s(es)?\b|\bano(s)?\b").is_match(t) {
        return false;
    }
    true
}

/// Tempo na etapa atual ('Está aqui 3 dias', 'Aqui há alguns segundos').
/// True se indica menos de 24h. Formato desconhecido → False (exclui).
pub fn etapa_dentro_24h(texto: &str) -> bool {
    let t = texto.to_lowercase();
    if re!(r"\bdia(s)?\b|\bsemana(s)?\b|\bm[eê]s(es)?\b|\bano(s)?\b").is_match(&t) {
        return false;
    }
    re!(r"\bsegundo(s)?\b|\bminuto(s)?\b|\bhora(s)?\b").is_match(&t)
}

/// Mapeamento sigla → nome da secretaria (Prefeitura de Montes Claros)
pub const SIGLAS_SECRETARIAS: [(&str, &str); 32] = [
    ("AD", "Assinatura Digital"),
    ("AC", "Autorização para Compras"),
    (
        "AMASBE",
        "Agência Municipal de Água e Saneamento Básico de Montes Claros",
    ),
    ("CGM", "Controladoria Geral do Município"),
    ("CIMP", "Comissão de Implantação"),
    ("CM", "Câmara Municipal"),
    ("ESURB", "ESURB"),
    ("GAB", "Gabinete do Prefeito"),
    ("GABP", "Prefeito"),
    ("GABVP", "Gabinete do Vice-Prefeito"),
    ("PGM", "Procuradoria Geral do Município"),
    ("SA", "Secretaria de Administração"),
    (
        "SABEAS",
        "Secretaria de Ambiente, Bem-Estar Animal e Sustentabilidade",
    ),
    ("SAE", "Secretaria de Aceleração Econômica"),
    (
        "SAPDR",
        "Secretaria de Agricultura, Pecuária e Desenvolvimento Rural",
    ),
    ("SC", "Secretaria de Comunicação"),
    ("SCT", "Secretaria de Cultura e Turismo"),
    ("SDC", "Secretaria de Defesa Civil"),
    ("SDS", "Secretaria de Desenvolvimento Social"),
    ("SE", "Secretaria de Educação"),
    ("SEJ", "Secretaria de Esporte e Juventude e Lazer"),
    ("SEJL", "Secretaria de Esporte e Juventude e Lazer"),
    ("SELJ", "Secretaria de Esporte e Juventude e Lazer"),
    ("SF", "Secretaria de Finanças"),
    ("SGRI", "Secretaria de Governo e Relações Institucionais"),
    ("SIPE", "Secretaria de Inovação e Projetos Especiais"),
    ("SIPU", "Secretaria de Infraestrutura e Planejamento Urbano"),
    ("SPOT", "Secretaria de Planejamento, Orçamento e Tecnologia"),
    ("SS", "Secretaria de Saúde"),
    ("SSI", "Secretaria de Segurança Integrada"),
    ("SSU", "Secretaria de Serviços Urbanos"),
    (
        "SUPERMOC",
        "Superintendência de Administração de Estádios e Estabelecimentos de Montes Claros",
    ),
];

/// Remove espaços e hífens para comparação.
pub fn normalizar_sigla(texto: &str) -> String {
    re!(r"[\s\-]").replace_all(texto, "").to_uppercase()
}

/// Extrai a sigla (letras antes do primeiro hífen) e converte para nome completo.
pub fn resolver_secretaria(sigla_bruta: &str) -> String {
    let parte = sigla_bruta.split('-').next().unwrap_or("").trim();
    let chave = normalizar_sigla(parte);
    SIGLAS_SECRETARIAS
        .iter()
        .find(|(k, _)| *k == chave)
        .map(|(_, v)| v.to_string())
        .unwrap_or_else(|| sigla_bruta.trim().to_string())
}

// ============================================================
// CAMADA HTTP
// ============================================================

const MARCAS_APP: [&str; 5] = [
    "link_emissao_a",
    "caixa_parametro",
    "navega_caixa",
    "kanban_estagios",
    "emission_infos",
];

#[derive(Default)]
pub struct Contagem {
    pub listagem: AtomicUsize,
    pub detalhe: AtomicUsize,
    pub outras: AtomicUsize,
}

#[derive(Clone, Copy)]
pub enum Tipo {
    Listagem,
    Detalhe,
    Outras,
}

pub fn parece_login(html: &str) -> bool {
    let tem_senha = re!(r#"(?i)type\s*=\s*["']?password"#).is_match(html)
        || html.contains(r#"name="identifier""#);
    tem_senha && !MARCAS_APP.iter().any(|m| html.contains(m))
}

/// Texto da resposta como HTML. Se o servidor devolver JSON, junta as
/// strings de dentro que contenham marcação.
fn html_da_resposta(r: &crate::http::Resposta) -> String {
    let texto = r.texto();
    let ctype = r.content_type.to_lowercase();
    let ini = texto.trim_start();
    if ctype.contains("json") || ini.starts_with('{') || ini.starts_with('[') {
        let Ok(dados) = serde_json::from_str::<Value>(&texto) else {
            return texto;
        };
        fn coletar(x: &Value, pedacos: &mut Vec<String>) {
            match x {
                Value::String(s) if s.contains('<') => pedacos.push(s.clone()),
                Value::Object(o) => o.values().for_each(|v| coletar(v, pedacos)),
                Value::Array(a) => a.iter().for_each(|v| coletar(v, pedacos)),
                _ => {}
            }
        }
        let mut pedacos = Vec::new();
        coletar(&dados, &mut pedacos);
        return pedacos.join("\n");
    }
    texto
}

/// Uma execução: sessão HTTP, contagem de requisições e ajustes.
pub struct Ctx<'a> {
    pub amb: &'a Ambiente,
    pub http: SessaoHttp,
    pub contagem: Contagem,
    pub pausa: f64,
}

impl Ctx<'_> {
    fn baixar(
        &self,
        url: &str,
        params: &[(&str, String)],
        tipo: Tipo,
        xhr: bool,
        referer: Option<&str>,
    ) -> Res<String> {
        self.amb.checar_cancelamento()?;
        let mut cab = Vec::new();
        if xhr {
            cab.push(("X-Requested-With", "XMLHttpRequest".to_string())); // como o jQuery da interface
        }
        if let Some(r) = referer {
            cab.push(("Referer", r.to_string()));
        }
        let r = self.http.get(url, params, &cab)?.conferir()?;
        match tipo {
            Tipo::Listagem => &self.contagem.listagem,
            Tipo::Detalhe => &self.contagem.detalhe,
            Tipo::Outras => &self.contagem.outras,
        }
        .fetch_add(1, Ordering::SeqCst);
        let html = html_da_resposta(&r);
        if parece_login(&html) {
            return Err(Erro::SessaoExpirada(format!(
                "{} devolveu a tela de login",
                url.split('?').next().unwrap_or(url)
            )));
        }
        self.amb.esperar(self.pausa)?;
        Ok(html)
    }

    /// Abre a caixa do setor uma vez: fixa o contexto (meu/trocar) na sessão
    /// do servidor e prova que os cookies do navegador valem por requisição.
    fn preparar_sessao(&self) -> Res<()> {
        let html = self.baixar(&url_caixa(self.amb), &[], Tipo::Outras, false, None)?;
        if !html.contains("caixa_parametro") && !html.contains("tab-content") {
            return Err(Erro::LayoutDesconhecido(
                "a caixa do setor abriu, mas sem os campos caixa_parametro_* da interface clássica — a 'Nova 1Doc' foi ativada? Desligue a chave no topo da caixa.".into(),
            ));
        }
        msg!(
            self.amb,
            "✓ Sessão HTTP válida — cookies do login reaproveitados"
        );
        Ok(())
    }
}

// ============================================================
// LISTAGEM (link.php?chamada=caixa)
// ============================================================

/// Só os nós de texto filhos diretos, com espaços colapsados.
fn texto_direto(el: ElementRef<'_>) -> String {
    limpo(&html::textos_diretos(el).join(" "))
}

/// title ou, no navegador, data-original-title (tooltip do Bootstrap).
fn titulo(el: Option<ElementRef<'_>>) -> String {
    el.and_then(|e| {
        html::attr(e, "title")
            .filter(|t| !t.is_empty())
            .or_else(|| html::attr(e, "data-original-title").filter(|t| !t.is_empty()))
    })
    .unwrap_or("")
    .to_string()
}

pub fn params_caixa(caixa: &str, dia: &str, num_pagina: u32) -> Vec<(&'static str, String)> {
    let mut p = vec![
        ("chamada", "caixa".to_string()),
        ("pg", "painel/listar".into()),
        ("caixa", caixa.into()),
        ("num_pagina", num_pagina.to_string()),
        ("ordenar", "1".into()),
        ("ordem", "desc".into()),
        ("modo_visualizacao", "1".into()),
        ("intervalo", format!("{dia} - {dia}")),
        ("emissoes_arquivado_prazos", "0".into()),
    ];
    if !ITD.is_empty() {
        p.push(("itd", ITD.into()));
    }
    p
}

/// 'Página 1/15' → 15 (None se o contador não vier no fragmento).
pub fn ler_total_paginas(html: &str) -> Option<u32> {
    re!(r"P(?:á|&aacute;|a)gina\s*(\d+)\s*/\s*(\d+)")
        .captures(html)
        .and_then(|m| m[2].parse().ok())
}

/// Uma linha da listagem (as chaves do dicionário do script).
#[derive(Clone, Debug, Default)]
pub struct Linha {
    pub chave: String,
    pub numero: String,
    pub tipo_doc: String,
    pub itd: String,
    pub url: String,
    pub emissao: String,
    pub mov_data: String,
    pub mov_rel: String,
    pub prioridade: String,
    pub objeto: String,
    pub etiqueta: String,
    pub de: String,
    pub setor_de: String,
    pub secretaria: String,
    pub caixa: String,
}

fn urljoin(base: &str, href: &str) -> String {
    url::Url::parse(base)
        .and_then(|b| b.join(href))
        .map(|u| u.to_string())
        .unwrap_or_else(|_| href.to_string())
}

/// Uma entrada por linha da tabela (tr que contém a.link_emissao_a).
pub fn extrair_linhas(amb: &Ambiente, fonte: &str) -> Vec<Linha> {
    let doc = html::doc(fonte);
    let mut linhas = Vec::new();
    for a in doc.select(sel!("a.link_emissao_a")) {
        let tr = html::ancestral_tag(a, "tr");
        let href = html::attr_ou_vazio(a, "href");
        let Some(tr) = tr.filter(|_| !href.is_empty()) else {
            continue;
        };

        let texto_link = texto_direto(a); // "Proc. Administrativo 31.632/2026"
        let m = re!(r"\d[\d.]*/\d+").find(&texto_link);
        let numero = m
            .map(|m| m.as_str().to_string())
            .unwrap_or_else(|| texto_link.clone());
        let tipo_doc = m
            .map(|m| texto_link[..m.start()].trim().to_string())
            .unwrap_or_default();

        let emissao = html::um(a, sel!("small.data"))
            .map(|e| limpo(&html::texto(e, "")))
            .unwrap_or_default();

        // Última movimentação: o title traz a data absoluta; o texto, a relativa.
        let el = html::um(tr, sel!(".tt.data"));
        let mov_data = re!(r"\d{2}/\d{2}/\d{4}")
            .find(&titulo(el))
            .map(|m| m.as_str().to_string())
            .unwrap_or_default();
        let mov_rel = el.map(|e| limpo(&html::texto(e, ""))).unwrap_or_default();

        let el = html::um(tr, sel!("span.label[title='Prioridade']"))
            .or_else(|| html::um(tr, sel!(".tt.label")));
        let prioridade = el.map(|e| limpo(&html::texto(e, ""))).unwrap_or_default();

        // Objeto: 1º texto direto das células .link_emissao.td.ignora.normal,
        // fora remetente e destinatário.
        let (mut objeto, mut td_assunto) = (String::new(), None);
        for td in tr.select(sel!(".link_emissao.td.ignora.normal")) {
            if html::tem_classe(td, "celula_para")
                || html::ancestral(td, |e| html::tem_classe(e, "td_remetente")).is_some()
            {
                continue;
            }
            let t = texto_direto(td);
            if !t.is_empty() {
                objeto = t;
                td_assunto = Some(td);
                break;
            }
        }

        // Etiqueta do assunto (badge sob o objeto)
        let mut etiqueta = String::new();
        if let Some(td) = td_assunto {
            let b = html::um(td, sel!("span.badge"))
                .or_else(|| html::um(td, sel!("span[title]:not(.label)")));
            if let Some(b) = b {
                let t = html::attr(b, "data-original-title")
                    .filter(|t| !t.is_empty())
                    .or_else(|| html::attr(b, "title").filter(|t| !t.is_empty()))
                    .map(str::to_string)
                    .unwrap_or_else(|| html::texto(b, ""));
                etiqueta = limpo(&t);
            }
        }

        let (mut setor_de, mut de) = (String::new(), String::new());
        if let Some(rem) = html::um(tr, sel!(".td_remetente")) {
            let b = html::um(rem, sel!(".badge.badge_env.tt"))
                .or_else(|| html::um(rem, sel!("span.badge")));
            setor_de = b.map(|b| limpo(&html::texto(b, ""))).unwrap_or_default();
            // Nome completo no popover (span.pp[data-content]); o texto visível vem abreviado.
            let pp = html::um(rem, sel!("span.pp[data-content]"));
            let resto = html::texto_sem(rem, sel!("span.badge, i, .label"), "\n");
            let partes: Vec<&str> = resto
                .split('\n')
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .collect();
            let conteudo = pp
                .map(|p| html::attr_ou_vazio(p, "data-content"))
                .filter(|c| !c.trim().is_empty());
            de = corta(
                &conteudo
                    .map(limpo)
                    .unwrap_or_else(|| partes.first().map(|p| p.to_string()).unwrap_or_default()),
                60,
            );
        }

        let m_hash = re!(r"[?&]hash=([0-9A-Za-z]+)").captures(href);
        let m_itd = re!(r"[?&]itd=(\d+)").captures(href);
        let itd = m_itd.map(|m| m[1].to_string()).unwrap_or_default();
        let url = match &m_hash {
            Some(h) => format!(
                "{}/?pg=doc/ver&hash={}{}",
                amb.url_1doc,
                &h[1],
                if itd.is_empty() {
                    String::new()
                } else {
                    format!("&itd={itd}")
                }
            ),
            None => urljoin(&amb.base(), href),
        };
        let secretaria = if setor_de.is_empty() {
            String::new()
        } else {
            resolver_secretaria(&setor_de)
        };
        linhas.push(Linha {
            chave: m_hash.map(|h| h[1].to_string()).unwrap_or_else(|| {
                if numero.is_empty() {
                    href.to_string()
                } else {
                    numero.clone()
                }
            }),
            numero,
            tipo_doc,
            itd,
            url,
            emissao,
            mov_data,
            mov_rel,
            prioridade,
            objeto,
            etiqueta,
            de,
            setor_de,
            secretaria,
            caixa: String::new(),
        });
    }
    linhas
}

/// Linha emitida ou movimentada no dia pedido ou depois dele (rede de
/// segurança, caso o servidor ignore o filtro de data).
fn pode_ser_da_data(l: &Linha, dia: &str, hoje: NaiveDate) -> bool {
    let Some(alvo) = datas::data_br(dia) else {
        return true;
    };
    let (emissao, mov) = (datas::data_br(&l.emissao), datas::data_br(&l.mov_data));
    if emissao.is_some_and(|e| e >= alvo) || mov.is_some_and(|m| m >= alvo) {
        return true;
    }
    if mov.is_none() {
        return eh_sub_dia(&l.mov_rel) || alvo < hoje;
    }
    false
}

fn listar_caixa(
    ctx: &Ctx,
    caixa: &str,
    rotulo: &str,
    dia: &str,
    hoje: NaiveDate,
) -> Res<(Vec<Linha>, u32, Option<u32>)> {
    let (mut achados, mut vistos) = (Vec::new(), std::collections::HashSet::new());
    let (mut total_pag, mut lidas) = (None, 0);
    let referer = url_caixa(ctx.amb);
    for num in 1..=MAX_PAGINAS {
        let fonte = ctx.baixar(
            &url_link(ctx.amb),
            &params_caixa(caixa, dia, num),
            Tipo::Listagem,
            true,
            Some(&referer),
        )?;
        lidas += 1;
        let linhas = extrair_linhas(ctx.amb, &fonte);
        if total_pag.is_none() {
            total_pag = ler_total_paginas(&fonte);
        }
        let novas: Vec<&Linha> = linhas
            .iter()
            .filter(|l| !vistos.contains(&l.chave))
            .collect();
        if novas.is_empty() {
            break; // página vazia, ou o servidor devolvendo a última de novo
        }
        for l in &novas {
            vistos.insert(l.chave.clone());
        }
        for l in novas {
            if l.emissao.starts_with(dia) {
                let mut l = l.clone();
                l.caixa = rotulo.to_string();
                achados.push(l);
            }
        }
        if total_pag.is_some_and(|t| num >= t) {
            break;
        }
        if !linhas.iter().any(|l| pode_ser_da_data(l, dia, hoje)) {
            break;
        }
    }
    Ok((achados, lidas, total_pag))
}

fn data_emissao(l: &Linha) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(&corta(&l.emissao, 16), "%d/%m/%Y %H:%M")
        .unwrap_or(NaiveDateTime::MIN)
}

fn coletar_do_dia(
    ctx: &Ctx,
    dia: &str,
    caixas: &[(String, String)],
    hoje: NaiveDate,
) -> Res<(Vec<Linha>, Vec<String>)> {
    let mut por_chave: Vec<Linha> = Vec::new();
    let mut caixas_com_falha = Vec::new();
    for (caixa, rotulo) in caixas {
        let (achados, lidas, total) = match listar_caixa(ctx, caixa, rotulo, dia, hoje) {
            Ok(r) => r,
            Err(e) if e.eh_http() => {
                msg!(
                    ctx.amb,
                    "  ⚠ {rotulo}: falha na listagem ({})",
                    corta(&e.to_string(), 80)
                );
                caixas_com_falha.push(rotulo.clone());
                continue;
            }
            Err(e) => return Err(e),
        };
        let mut pags = if lidas == 1 {
            "1 requisição".to_string()
        } else {
            format!("{lidas} requisições")
        };
        if let Some(t) = total.filter(|t| *t > 0) {
            pags.push_str(&format!(", {t} pág. no filtro"));
        }
        msg!(
            ctx.amb,
            "  {rotulo:<15} {:>3} emitido(s) no dia  ({pags})",
            achados.len()
        );
        for l in achados {
            match por_chave.iter_mut().find(|x| x.chave == l.chave) {
                Some(x) => x.caixa.push_str(&format!(", {rotulo}")),
                None => por_chave.push(l),
            }
        }
    }
    por_chave.sort_by_key(|l| std::cmp::Reverse(data_emissao(l)));
    Ok((por_chave, caixas_com_falha))
}

// ============================================================
// DETALHE (etapa atual no kanban)
// ============================================================

/// HTML até o modelo oculto do despacho (div#parent_wa): o 1Doc renderiza
/// o kanban duas vezes, e a segunda cópia tem sempre "Aqui há alguns segundos".
fn antes_do_modelo(fonte: &str) -> &str {
    match re!(r#"\bid\s*=\s*["']?parent_wa\b"#).find(fonte) {
        Some(m) => &fonte[..m.start()],
        None => fonte,
    }
}

fn regex_atual() -> &'static Regex {
    re!(r#"(?i)data-current\s*=\s*["']?true"#)
}

/// (nome, tempo) de um nó data-current, ou None se não tiver título.
fn etapa(atual: ElementRef<'_>) -> Option<(String, String)> {
    let t = html::um(atual, sel!(".kanban_titulo p"))?;
    let nome = limpo(&html::texto(t, ""));
    if nome.is_empty() {
        return None;
    }
    Some((nome, tempo_da_etapa(atual)))
}

/// 'Esta aqui 1 hora' / 'Aqui há alguns segundos' ('' se não houver).
fn tempo_da_etapa(atual: ElementRef<'_>) -> String {
    for s in [
        sel!(".kanban_itens p.kanban_item"),
        sel!(".kanban_itens .kanban-time"),
    ] {
        if let Some(el) = html::um(atual, s) {
            let t = limpo(&html::texto(el, ""));
            if !t.is_empty() {
                return t;
            }
        }
    }
    String::new()
}

/// (nome, tempo) da etapa marcada data-current="true" no quadro do
/// cabeçalho, ou None. Caminho rápido: a primeira marca ANTES do modelo de
/// despacho, num recorte de 8 kB. Caminho lento: a página inteira,
/// ignorando qualquer marca dentro de #parent_wa.
pub fn ler_etapa_atual(fonte: &str) -> Option<(String, String)> {
    let antes = antes_do_modelo(fonte);
    if let Some(m) = regex_atual().find(antes) {
        let inicio = antes[..m.start()].rfind('<').unwrap_or(0);
        let doc = html::doc(fatia(antes, inicio, 8000));
        if let Some(a) = html::um_doc(&doc, sel!("[data-current='true']"))
            && let Some(achado) = etapa(a)
        {
            return Some(achado);
        }
    }
    if !regex_atual().is_match(fonte) {
        return None;
    }
    let doc = html::doc(fonte);
    for atual in doc.select(sel!("[data-current='true']")) {
        if html::ancestral(atual, |e| html::attr(e, "id") == Some("parent_wa")).is_none()
            && let Some(achado) = etapa(atual)
        {
            return Some(achado);
        }
    }
    None
}

/// Etiqueta do assunto no cabeçalho (badge dentro de h4.tit_assunto), sem o
/// prefixo "Assunto:". "" se não achar.
pub fn ler_etiqueta(fonte: &str) -> String {
    let Some(m) = re!(r#"(?i)class\s*=\s*["'][^"']*\btit_assunto\b"#).find(fonte) else {
        return String::new();
    };
    let inicio = fonte[..m.start()].rfind('<').unwrap_or(0);
    for trecho in [fatia(fonte, inicio, 4000), fonte] {
        let doc = html::doc(trecho);
        let b = html::um_doc(&doc, sel!(".tit_assunto"))
            .and_then(|h4| html::um(h4, sel!("span.badge")));
        if let Some(b) = b {
            let t = html::attr(b, "title")
                .filter(|t| !t.is_empty())
                .or_else(|| html::attr(b, "data-original-title").filter(|t| !t.is_empty()))
                .map(str::to_string)
                .unwrap_or_else(|| html::texto(b, ""));
            return limpo(&re!(r"^\s*Assunto:\s*").replace(&t, ""));
        }
    }
    String::new()
}

/// Texto do elemento #campo_adicional_descricao_objeto ("" se não houver).
/// Quebras de linha de `<br>` e de parágrafos são preservadas; espaços
/// internos de cada linha são normalizados.
pub fn ler_descricao_objeto(fonte: &str) -> String {
    if !re!(r#"\bid\s*=\s*["']?campo_adicional_descricao_objeto\b"#).is_match(fonte) {
        return String::new();
    }
    let doc = html::doc(fonte);
    let Some(el) = html::um_doc(&doc, sel!("#campo_adicional_descricao_objeto")) else {
        return String::new();
    };
    let valor = html::um(el, sel!("[data-valor]"));
    let small = html::um(el, sel!("small"));
    let txt = if let Some(v) =
        valor.filter(|v| !html::attr_ou_vazio(*v, "data-valor").trim().is_empty())
    {
        let t = html::attr_ou_vazio(v, "data-valor");
        if t.contains('<') {
            let frag = html::doc(&t.replace("<br", "\n<br"));
            html::texto_doc(&frag, "")
        } else {
            t.to_string()
        }
    } else if let Some(s) = small.filter(|s| html::tem_texto(*s)) {
        html::texto_com_quebras(s, false)
    } else if html::nome_tag(el) == "input" {
        html::attr_ou_vazio(el, "value").to_string()
    } else if html::nome_tag(el) == "textarea" {
        html::texto(el, "")
    } else {
        html::texto_com_quebras(el, true)
    };
    let txt = crate::texto::linhas(&txt)
        .into_iter()
        .map(limpo)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    let txt = re!(r"(?i)^\s*(?:Descri[çc][ãa]o do )?objeto\s*\*?\s*:\s*").replace(&txt, "");
    corta(&txt, LIMITE_CELULA)
}

pub const ETIQUETAS_PREPARATORIA: &str = r"(?i)Fase Preparat[óo]ria";
pub const ETAPAS_COMO_ELABORACAO: &str = r"(?i)^\s*Etapa Final\s*$";

/// Texto da coluna Etapa: fase de elaboração → "Elaboração"; abertura →
/// "<etapa> — Abertura"; outra fase com quadro → "<etapa> — <etiqueta>".
pub fn rotulo_etapa(nome: &str, etiqueta: &str, sem_quadro: bool) -> String {
    if !sem_quadro && re!(ETIQUETAS_ABERTURA).is_match(etiqueta) {
        return format!("{nome} — Abertura");
    }
    if sem_quadro
        || re!(ETIQUETAS_PREPARATORIA).is_match(etiqueta)
        || re!(ETAPAS_COMO_ELABORACAO).is_match(nome)
    {
        return "Elaboração".into();
    }
    if etiqueta.is_empty() {
        nome.to_string()
    } else {
        format!("{nome} — {etiqueta}")
    }
}

/// (entra na planilha?, motivo se não entra) para documento sem quadro.
pub fn classificar_sem_quadro(l: &Linha, etiqueta: &str) -> (bool, String) {
    if !INCLUIR_SEM_QUADRO {
        return (false, "sem quadro de etapas".into());
    }
    if l.itd != "3" && !l.tipo_doc.starts_with("Proc. Administrativo") {
        let t = if l.tipo_doc.is_empty() {
            "?"
        } else {
            &l.tipo_doc
        };
        return (
            false,
            format!("sem quadro — não é Proc. Administrativo ({t})"),
        );
    }
    if etiqueta.is_empty() {
        return (
            false,
            "sem quadro — etiqueta do assunto não encontrada".into(),
        );
    }
    if re!(ETIQUETAS_ABERTURA).is_match(etiqueta) || !re!(ETIQUETAS_ELABORACAO).is_match(etiqueta) {
        return (
            false,
            format!("sem quadro — etiqueta fora da elaboração: {etiqueta}"),
        );
    }
    (true, String::new())
}

/// Resultado do detalhe de um processo.
#[derive(Clone, Debug, Default)]
pub struct Resultado {
    pub linha: Linha,
    pub etapa: String,
    pub etapa_nome: String,
    pub tempo_etapa: String,
    pub sem_quadro: bool,
    pub incluir: bool,
    pub motivo: String,
    pub erro: String,
    pub titulo: String,
    pub objeto_fonte: String,
    pub abertura: bool,
}

/// exigir_24h=true (dia = hoje): etapa atual com menos de 24h. false (dia
/// passado): entra com a etapa atual, qualquer que seja o tempo.
fn processar_processo(ctx: &Ctx, l: &Linha, exigir_24h: bool) -> Res<Resultado> {
    let mut res = Resultado {
        linha: l.clone(),
        titulo: l.objeto.clone(),
        objeto_fonte: "título".into(),
        ..Default::default()
    };
    let r = (|| -> Res<()> {
        let fonte = ctx.baixar(&l.url, &[], Tipo::Detalhe, false, Some(&url_caixa(ctx.amb)))?;
        if !fonte.contains("tit_assunto") && !fonte.contains("kanban_estagios") {
            return Err(Erro::LayoutDesconhecido(
                "detalhe sem h4.tit_assunto nem kanban (layout mudou?)".into(),
            ));
        }
        let atual = ler_etapa_atual(&fonte);
        let etiqueta = Some(ler_etiqueta(&fonte))
            .filter(|e| !e.is_empty())
            .unwrap_or_else(|| l.etiqueta.clone());
        res.linha.etiqueta = etiqueta.clone();
        if !etiqueta.is_empty() && re!(ETIQUETAS_ABERTURA).is_match(&etiqueta) {
            res.abertura = true;
            if OBJETO_DA_ABERTURA {
                let descricao = ler_descricao_objeto(&fonte);
                if !descricao.is_empty() {
                    res.linha.objeto = descricao;
                    res.objeto_fonte = ID_DESCRICAO_OBJETO.into();
                }
            }
        }
        if let Some((nome, tempo)) = atual {
            let dentro = if exigir_24h && !tempo.is_empty() {
                etapa_dentro_24h(&tempo)
            } else {
                true
            };
            res.etapa = rotulo_etapa(&nome, &etiqueta, false);
            res.etapa_nome = nome;
            res.tempo_etapa = tempo;
            res.incluir = dentro;
            res.motivo = if dentro {
                String::new()
            } else {
                "etapa atual há 1 dia ou mais".into()
            };
        } else if fonte.contains("kanban_estagios") {
            res.motivo =
                "quadro sem etapa atual (saiu de uma etapa, não entrou na seguinte)".into();
        } else {
            let (entra, motivo) = classificar_sem_quadro(l, &etiqueta);
            res.sem_quadro = true;
            res.incluir = entra;
            res.motivo = motivo;
            res.etapa = rotulo_etapa("", &etiqueta, true);
        }
        Ok(())
    })();
    match r {
        Ok(()) => {}
        Err(e) if e.atravessa() || matches!(e, Erro::SessaoExpirada(_)) => return Err(e),
        // falha de leitura ≠ processo excluído: vai para o resumo
        Err(e) => res.erro = corta(&e.to_string(), 120),
    }
    Ok(res)
}

// ============================================================
// EXPORTAÇÃO
// ============================================================

pub fn exportar_xlsx(amb: &Ambiente, processos: &[&Resultado], arquivo: &Path) -> Res<PathBuf> {
    let gravar = |caminho: &Path| -> Res<()> {
        let mut wb = Workbook::new();
        let ws = wb.add_worksheet();
        ws.set_name("Processos Recentes").map_err(xl)?;
        let cab = Format::new()
            .set_bold()
            .set_font_color(Color::White)
            .set_font_name("Arial")
            .set_font_size(11)
            .set_background_color(Color::RGB(0x2F5496))
            .set_align(FormatAlign::Center)
            .set_align(FormatAlign::VerticalCenter)
            .set_text_wrap();
        for (c, t) in ["Nº Processo", "Prioridade", "Objeto", "Etapa", "Secretaria"]
            .iter()
            .enumerate()
        {
            ws.write_string_with_format(0, c as u16, *t, &cab)
                .map_err(xl)?;
        }
        let dado = Format::new().set_font_name("Arial").set_font_size(10);
        let link = dado
            .clone()
            .set_font_color(Color::RGB(0x0563C1))
            .set_underline(FormatUnderline::Single);
        let quebra = dado.clone().set_align(FormatAlign::Top).set_text_wrap();
        for (i, p) in processos.iter().enumerate() {
            let r = (i + 1) as u32;
            let l = &p.linha;
            if l.url.is_empty() {
                crate::planilha::texto(ws, r, 0, &l.numero, &dado)?;
            } else {
                // Abre o processo no 1Doc (no navegador padrão — exige estar logado nele)
                crate::planilha::link(ws, r, 0, &l.url, &l.numero, &link)?;
            }
            crate::planilha::texto(ws, r, 1, &l.prioridade, &dado)?;
            crate::planilha::texto(ws, r, 2, &l.objeto, &quebra)?;
            crate::planilha::texto(ws, r, 3, &p.etapa, &quebra)?;
            crate::planilha::texto(ws, r, 4, &l.secretaria, &dado)?;
        }
        for (c, larg) in [30.0, 14.0, 55.0, 40.0, 50.0].iter().enumerate() {
            ws.set_column_width(c as u16, *larg).map_err(xl)?;
        }
        ws.autofilter(0, 0, processos.len() as u32, 4).map_err(xl)?;
        ws.set_freeze_panes(1, 0).map_err(xl)?;
        wb.save(caminho).map_err(xl)
    };
    let salvo = salvar(amb, arquivo, &gravar)?;
    msg!(amb, "  Processos exportados: {}", processos.len());
    Ok(salvo)
}

fn xl(e: rust_xlsxwriter::XlsxError) -> Erro {
    Erro::Outro(e.to_string())
}

/// Coleta completa (inclusive os excluídos, com o motivo).
pub fn exportar_json(
    amb: &Ambiente,
    resultados: &[Resultado],
    dia: &str,
    arquivo: &Path,
    caixas: &[(String, String)],
) -> Res<()> {
    let ou_null = |s: &str| if s.is_empty() { Value::Null } else { json!(s) };
    let itens: Vec<Value> = resultados
        .iter()
        .map(|r| {
            let l = &r.linha;
            let (data, hora) = l.emissao.split_once(' ').unwrap_or((&l.emissao, ""));
            let mut m = Map::new();
            m.insert("numero".into(), json!(l.numero));
            m.insert("data".into(), json!(data));
            m.insert("hora".into(), json!(hora));
            m.insert("assunto".into(), json!(l.objeto));
            m.insert("titulo".into(), json!(r.titulo));
            m.insert("objeto_fonte".into(), json!(r.objeto_fonte));
            m.insert(
                "tipo".into(),
                json!(if l.etiqueta.is_empty() {
                    &l.tipo_doc
                } else {
                    &l.etiqueta
                }),
            );
            m.insert("prioridade".into(), ou_null(&l.prioridade));
            m.insert("de".into(), json!(l.de));
            m.insert("setor_de".into(), json!(l.setor_de));
            m.insert("secretaria".into(), json!(l.secretaria));
            m.insert("caixa".into(), json!(l.caixa));
            m.insert("url".into(), json!(l.url));
            m.insert("etapa".into(), json!(r.etapa_nome));
            m.insert("fase".into(), ou_null(&l.etiqueta));
            m.insert("tempo_etapa".into(), json!(r.tempo_etapa));
            m.insert("sem_quadro".into(), json!(r.sem_quadro));
            m.insert("abertura".into(), json!(r.abertura));
            m.insert("incluir".into(), json!(r.incluir));
            m.insert("motivo".into(), ou_null(&r.motivo));
            m.insert("erro".into(), ou_null(&r.erro));
            Value::Object(m)
        })
        .collect();
    let mut dados = Map::new();
    dados.insert("dia".into(), json!(dia));
    dados.insert("gerado_em".into(), json!(sessao::agora_iso()));
    dados.insert(
        "caixas".into(),
        json!(caixas.iter().map(|(_, r)| r).collect::<Vec<_>>()),
    );
    dados.insert("total".into(), json!(itens.len()));
    dados.insert(
        "numeros".into(),
        json!(
            itens
                .iter()
                .map(|i| i["numero"].clone())
                .collect::<Vec<_>>()
        ),
    );
    dados.insert("itens".into(), Value::Array(itens));
    let texto = arquivos::json_indentado(&Value::Object(dados));
    salvar(amb, arquivo, &|c: &Path| {
        std::fs::write(c, &texto).map_err(Erro::from)
    })?;
    Ok(())
}

fn absoluto(p: &Path) -> PathBuf {
    std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf())
}

/// Grava no caminho pedido; se falhar (ex.: Área de Trabalho no OneDrive),
/// tenta na pasta de dados do programa.
fn salvar(amb: &Ambiente, arquivo: &Path, gravar: &dyn Fn(&Path) -> Res<()>) -> Res<PathBuf> {
    let r = (|| {
        if let Some(p) = arquivo.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(p)?;
        }
        gravar(arquivo)
    })();
    match r {
        Ok(()) => {
            msg!(amb, "✓ Salvo em: {}", absoluto(arquivo).display());
            Ok(absoluto(arquivo))
        }
        Err(e) if e.atravessa() => Err(e),
        Err(e) => {
            let reserva = amb
                .pasta_dados()
                .join(arquivo.file_name().unwrap_or_default());
            msg!(amb, "⚠ Falha ao salvar em '{}': {e}", arquivo.display());
            gravar(&reserva)?;
            msg!(
                amb,
                "✓ Salvo na pasta de dados do programa: {}",
                absoluto(&reserva).display()
            );
            Ok(absoluto(&reserva))
        }
    }
}

// ============================================================
// EXECUÇÃO
// ============================================================

pub struct Opcoes {
    pub caixas: Option<Vec<(String, String)>>,
    pub pasta_saida: Option<PathBuf>,
    pub abrir: Option<bool>,
    pub hoje: Option<NaiveDate>,
    pub pausa: f64,
}

impl Default for Opcoes {
    fn default() -> Self {
        Opcoes {
            caixas: None,
            pasta_saida: None,
            abrir: None,
            hoje: None,
            pausa: PAUSA_REQ,
        }
    }
}

pub fn caixas_padrao() -> Vec<(String, String)> {
    CAIXAS
        .iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect()
}

/// Roda a extração para o dia `alvo` com as credenciais da sessão. Devolve
/// o caminho da planilha, ou None.
pub fn executar(
    amb: &Ambiente,
    alvo: NaiveDate,
    cred: &Credenciais,
    op: Opcoes,
) -> Res<Option<PathBuf>> {
    let hoje = op.hoje.unwrap_or_else(datas::hoje); // fixado uma vez: não muda se cruzar a meia-noite
    let caixas = op.caixas.clone().unwrap_or_else(caixas_padrao);
    let cfg = config::carregar(amb);
    let pasta_saida = op
        .pasta_saida
        .clone()
        .unwrap_or_else(|| config::pasta_saida(amb, &cfg));
    let abrir = op.abrir.unwrap_or(cfg.abrir_planilha);
    if alvo > hoje {
        return Err(Erro::encerrar(format!(
            "✗ {} ainda não chegou.",
            datas::br(alvo)
        )));
    }
    if caixas.is_empty() {
        return Err(Erro::encerrar("✗ Marque ao menos uma caixa."));
    }
    let dia = datas::br(alvo);
    let exigir_24h = alvo == hoje;

    let igual = "=".repeat(60);
    msg!(amb, "{igual}");
    msg!(amb, "EXTRATOR DE PROCESSOS DO DIA — 1Doc (por requisição)");
    msg!(amb, "{}", "-".repeat(60));
    msg!(
        amb,
        "Emitidos em {dia}  |  execução: {}",
        Local::now().format("%d/%m/%Y %H:%M")
    );
    msg!(
        amb,
        "Caixas: {}",
        caixas
            .iter()
            .map(|(_, r)| r.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    if !exigir_24h {
        msg!(
            amb,
            "Dia passado: processos com quadro entram com a etapa atual,"
        );
        msg!(amb, "qualquer que seja o tempo nela (sem a janela de 24h).");
    }
    msg!(amb, "{igual}");

    let t0 = Instant::now();
    let ctx = Ctx {
        amb,
        http: SessaoHttp::nova(
            amb,
            &cred.cookies,
            &cred.ua,
            TIMEOUT,
            Repeticao::padrao(0.6),
        ),
        contagem: Contagem::default(),
        pausa: op.pausa,
    };

    let mut resultados: Vec<Resultado> = Vec::new();
    let mut caixas_com_falha = Vec::new();
    let r = (|| -> Res<()> {
        ctx.preparar_sessao()?;
        msg!(
            amb,
            "\n--- Listagem (filtro de emissão no servidor: {dia}) ---"
        );
        let (todos, falhas) = coletar_do_dia(&ctx, &dia, &caixas, hoje)?;
        caixas_com_falha = falhas;
        msg!(amb, "✓ Processos únicos emitidos em {dia}: {}", todos.len());
        if !todos.is_empty() {
            msg!(
                amb,
                "\n--- Detalhe: etapa no kanban ou etiqueta, se não houver quadro ({WORKERS} em paralelo) ---"
            );
            let n = todos.len();
            paralelo::mapear(
                &todos,
                WORKERS,
                |p| processar_processo(&ctx, p, exigir_24h),
                |i, r| {
                    let status = if !r.erro.is_empty() {
                        "✗"
                    } else if r.incluir {
                        "✓"
                    } else {
                        "⏸"
                    };
                    let etapa_tela = if !r.tempo_etapa.is_empty() {
                        format!("{} ({})", r.etapa_nome, r.tempo_etapa)
                    } else {
                        [&r.etapa_nome, &r.linha.etiqueta, &r.etapa]
                            .into_iter()
                            .find(|s| !s.is_empty())
                            .cloned()
                            .unwrap_or_default()
                    };
                    let info = [&r.erro, if r.incluir { &etapa_tela } else { &r.motivo }]
                        .into_iter()
                        .find(|s| !s.is_empty())
                        .unwrap_or(&etapa_tela)
                        .clone();
                    msg!(
                        amb,
                        "  [{}/{n}] [{status}] {} | {}",
                        i + 1,
                        r.linha.numero,
                        corta(&info, 70)
                    );
                    resultados.push(r);
                    Ok(())
                },
            )?;
        }
        Ok(())
    })();
    match r {
        Ok(()) => {}
        Err(Erro::SessaoExpirada(e)) => {
            sessao::esquecer(amb);
            msg!(amb, "\n✗ Sessão HTTP recusada: {e}");
            msg!(
                amb,
                "  A sessão do 1Doc expirou ou caiu. Execute de novo: o navegador vai"
            );
            msg!(
                amb,
                "  abrir para o login. Se persistir, o 1Doc mudou o login ou o endpoint."
            );
            return Ok(None);
        }
        Err(Erro::LayoutDesconhecido(e)) => {
            msg!(amb, "\n✗ Interface não reconhecida: {e}");
            return Ok(None);
        }
        Err(e) => return Err(e),
    }

    let incluidos: Vec<&Resultado> = resultados.iter().filter(|r| r.incluir).collect();
    let com_quadro = incluidos.iter().filter(|r| !r.sem_quadro).count();
    let sem_quadro = incluidos.iter().filter(|r| r.sem_quadro).count();
    let falhas: Vec<&Resultado> = resultados.iter().filter(|r| !r.erro.is_empty()).collect();
    let fora: Vec<&Resultado> = resultados
        .iter()
        .filter(|r| !r.incluir && r.erro.is_empty())
        .collect();
    let abertura: Vec<&&Resultado> = incluidos.iter().filter(|r| r.abertura).collect();
    let abertura_sem_desc: Vec<&&&Resultado> = abertura
        .iter()
        .filter(|r| r.objeto_fonte != ID_DESCRICAO_OBJETO)
        .collect();

    msg!(amb);
    let caminho_xlsx = exportar_xlsx(amb, &incluidos, &pasta_saida.join(nome_planilha(alvo)))?;
    exportar_json(
        amb,
        &resultados,
        &dia,
        &pasta_saida.join(NOME_JSON),
        &caixas,
    )?;

    msg!(amb, "\n{igual}");
    msg!(amb, "RESUMO");
    let rotulo_quadro = if exigir_24h {
        "com quadro, etapa atual < 24h:"
    } else {
        "com quadro, etapa atual:"
    };
    msg!(
        amb,
        "  {:<37}{}",
        format!("Emitidos em {dia} (únicos):"),
        resultados.len()
    );
    msg!(
        amb,
        "  Exportados:                          {}",
        incluidos.len()
    );
    msg!(amb, "    {rotulo_quadro:<35}{com_quadro}");
    msg!(amb, "    sem quadro, fase de elaboração:    {sem_quadro}");
    if OBJETO_DA_ABERTURA && !abertura.is_empty() {
        msg!(
            amb,
            "    de abertura, objeto do campo:      {}/{}",
            abertura.len() - abertura_sem_desc.len(),
            abertura.len()
        );
        if !abertura_sem_desc.is_empty() {
            let nums: Vec<&str> = abertura_sem_desc
                .iter()
                .map(|r| r.linha.numero.as_str())
                .collect();
            msg!(
                amb,
                "      ⚠ campo vazio ou ausente, ficou o título: {}",
                nums.join(", ")
            );
        }
    }
    msg!(amb, "  Não exportados:                      {}", fora.len());
    // Agrupa pelo motivo sem o detalhe (o que vem depois de ":" ou "(")
    let motivos = fora.iter().map(|r| {
        re!(r"[:(]")
            .split(&r.motivo)
            .next()
            .unwrap_or("")
            .trim()
            .to_string()
    });
    for (motivo, qtd) in paralelo::mais_comuns(motivos) {
        msg!(amb, "    {motivo}: {qtd}");
    }
    for r in &fora {
        if r.motivo.starts_with("sem quadro — etiqueta") {
            let e = if r.linha.etiqueta.is_empty() {
                "sem etiqueta"
            } else {
                &r.linha.etiqueta
            };
            msg!(amb, "      · {}  [{e}]", r.linha.numero);
        }
    }
    let sem_atual: Vec<&str> = fora
        .iter()
        .filter(|r| r.motivo.starts_with("quadro sem etapa atual"))
        .map(|r| r.linha.numero.as_str())
        .collect();
    if !sem_atual.is_empty() {
        msg!(
            amb,
            "      quadro sem etapa atual: {}",
            sem_atual.join(", ")
        );
    }
    if !falhas.is_empty() {
        let nums: Vec<&str> = falhas.iter().map(|r| r.linha.numero.as_str()).collect();
        msg!(
            amb,
            "  ✗ Falha ao ler o detalhe:            {} → {}",
            falhas.len(),
            nums.join(", ")
        );
    }
    if !caixas_com_falha.is_empty() {
        msg!(
            amb,
            "  ⚠ Caixas NÃO lidas (resultado incompleto): {}",
            caixas_com_falha.join(", ")
        );
    }
    msg!(
        amb,
        "  Requisições: {} de listagem + {} de detalhe + {} de preparo",
        ctx.contagem.listagem.load(Ordering::SeqCst),
        ctx.contagem.detalhe.load(Ordering::SeqCst),
        ctx.contagem.outras.load(Ordering::SeqCst)
    );
    msg!(amb, "  Tempo total: {:.1}s", t0.elapsed().as_secs_f64());
    msg!(amb, "{igual}");

    if abrir {
        amb.abrir_arquivo(&caminho_xlsx);
    }
    Ok(Some(caminho_xlsx))
}

/// Dia digitado na linha de comando: vazio/"hoje", "ontem" ou uma data.
pub fn ler_dia(txt: &str, hoje: NaiveDate) -> Option<NaiveDate> {
    let t = txt.trim().to_lowercase();
    match t.as_str() {
        "" | "hoje" => Some(hoje),
        "ontem" => Some(datas::ontem(hoje)),
        _ => datas::interpretar_data(&t, hoje),
    }
}
