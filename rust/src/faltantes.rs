//! Processos que estão no 1Doc ANTES da "Pesquisa de Mercado" e FALTAM na
//! planilha de acompanhamento (porta de faltantes.py; as regras estão
//! descritas lá).
//!
//! Proc. Administrativo das caixas do setor, emitido no período escolhido,
//! que: não está na planilha (nem ele, nem a origem, nem o processo novo
//! emitido a partir dele); é de abertura, fase preparatória ou elaboração
//! (etiqueta ou id do assunto) ou tem a Pesquisa de Mercado no fluxo; está
//! ANTES da Pesquisa de Mercado; e cujo último despacho não menciona
//! cancelamento. Saída: `<planilha>_faltantes_AAAA-MM-DD_HHMM.xlsx`.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

use chrono::{Local, NaiveDate, NaiveDateTime};
use rust_xlsxwriter::{
    Color, Format, FormatAlign, FormatUnderline, Table, TableColumn, TableStyle, Workbook,
};
use scraper::ElementRef;
use serde_json::{Value, json};
use unicode_normalization::UnicodeNormalization;

use crate::ambiente::Ambiente;
use crate::erro::{Erro, Res};
use crate::planilha::xl;
use crate::sessao::{self, Credenciais, NavegadorSobDemanda};
use crate::texto::{corta, ler_numero, limpo, norm, sem_acento};
use crate::verificadores::{self as v, Cache, CelulaLayout, Etapa, Http, INDET, Item, NAO, SIM};
use crate::{config, datas, html, msg, paralelo, re};

pub const CAIXAS_BUSCA: [(&str, &str); 4] = v::CAIXAS_INDICE;
pub const ANO_INICIAL: Option<i32> = None;
pub const MAX_PAGINAS_CAIXA: i64 = 300;
pub const N_ULTIMOS_DESPACHOS: usize = 1;
pub const JANELA_NEG: usize = 30;
const RE_ETIQ_ALVO: &str =
    r"abertura de procedimento|fase preparatoria|elaboracao de document\w* preparatori";
pub const IDS_ASSUNTO_ALVO: [&str; 3] = ["961721", "962809", "962265"];
pub const EXCLUIR_ETAPA_FINAL_SEM_NOVO: bool = false;
pub const VERIFICAR_NOVO_PROCESSO: bool = true;
pub const USAR_INDICE_ORIGEM: bool = true;

// ============================================================
// PERÍODO DE EMISSÃO
// ============================================================

/// Completa o período com o padrão: sem `de`, 1º/jan do ano mais antigo da
/// planilha; sem `ate`, ou com ele no futuro, hoje.
pub fn resolver_periodo(
    de: Option<NaiveDate>,
    ate: Option<NaiveDate>,
    itens: &[Item],
    hoje: NaiveDate,
) -> Res<(NaiveDate, NaiveDate)> {
    let de = de.unwrap_or_else(|| {
        let ano = ANO_INICIAL.unwrap_or_else(|| {
            itens
                .iter()
                .filter_map(|i| i.ano.parse().ok())
                .min()
                .unwrap_or(chrono::Datelike::year(&hoje))
        });
        NaiveDate::from_ymd_opt(ano, 1, 1).unwrap_or(hoje)
    });
    let ate = ate.unwrap_or(hoje).min(hoje);
    if de > ate {
        return Err(Erro::encerrar(format!(
            "✗ Período vazio: a data final ({}) é anterior ao início ({}). Informe também a data inicial.",
            datas::br(ate),
            datas::br(de)
        )));
    }
    Ok((de, ate))
}

// ============================================================
// LEITURA DO DETALHE (kanban e etiqueta)
// ============================================================

/// True = o detalhe é do processo procurado; None = página sem #nome_em.
pub fn confere(pagina: &str, item: &Item) -> Option<bool> {
    let nome = v::ler_nome_em(pagina)?;
    Some(ler_numero(&nome).is_some_and(|l| l.1 == item.digitos && l.2 == item.ano))
}

/// Etapas do quadro VISÍVEL, em ordem; [] se não houver quadro.
pub fn ler_quadro(fonte: &str) -> Vec<Etapa> {
    let Some(m) = re!(r#"class\s*=\s*["'][^"']*\bworkflow_v2_kanban\b"#).find(fonte) else {
        return Vec::new();
    };
    let inicio = fonte[..m.start()].rfind('<').unwrap_or(0);
    let doc = html::doc(crate::texto::fatia(fonte, inicio, 60000));
    let Some(quadro) = html::um_doc(&doc, sel!(".workflow_v2_kanban")) else {
        return Vec::new();
    };
    quadro
        .select(sel!(".kanban_estagios"))
        .map(|est| {
            let t = |s: &scraper::Selector| {
                html::um(est, s)
                    .map(|e| limpo(&html::texto(e, "")))
                    .unwrap_or_default()
            };
            let item = t(sel!(".kanban_itens .kanban_item"));
            Etapa {
                nome: t(sel!(".kanban_titulo")),
                atual: html::attr_ou_vazio(est, "data-current").to_lowercase() == "true",
                marca: if item.is_empty() {
                    t(sel!(".kanban_itens .kanban-time"))
                } else {
                    item
                },
                final_: html::tem_classe(est, "estagio_final"),
            }
        })
        .collect()
}

pub fn ler_id_assunto(fonte: &str) -> String {
    re!(r#"(?i)class\s*=\s*["'][^"']*\btit_assunto\b[^"']*["'][^>]*\bdata-id_assunto\s*=\s*["'](\d+)["']"#)
        .captures(fonte)
        .map(|m| m[1].to_string())
        .unwrap_or_default()
}

pub fn assunto_alvo(etiqueta: &str, id_assunto: &str) -> bool {
    IDS_ASSUNTO_ALVO.contains(&id_assunto) || re!(RE_ETIQ_ALVO).is_match(&norm(etiqueta))
}

fn titulo_ou(el: ElementRef<'_>) -> String {
    html::attr(el, "title")
        .filter(|t| !t.is_empty())
        .or_else(|| html::attr(el, "data-original-title").filter(|t| !t.is_empty()))
        .map(str::to_string)
        .unwrap_or_else(|| html::texto(el, ""))
}

/// Assunto do badge em h4.tit_assunto, sem o prefixo 'Assunto:'.
pub fn ler_etiqueta(fonte: &str) -> String {
    let mut i = fonte
        .find("tit_assunto\"")
        .or_else(|| fonte.find("tit_assunto "));
    while let Some(p) = i {
        let inicio = fonte[..p].rfind('<').unwrap_or(0);
        let doc = html::doc(crate::texto::fatia(fonte, inicio, 4000));
        if let Some(b) = html::um_doc(&doc, sel!(".tit_assunto .badge, .tit_assunto span[title]")) {
            return limpo(&re!(r"^\s*Assunto\s*:\s*").replace(&titulo_ou(b), ""));
        }
        i = fonte[p + 1..].find("tit_assunto").map(|q| q + p + 1);
    }
    String::new()
}

/// (veredito, etapa atual, observação).
pub fn avaliar(etapas: &[Etapa], etiqueta: &str) -> (&'static str, String, String) {
    let corte = norm(v::ETAPA_CORTE);
    if etapas.is_empty() {
        let obs = "Sem quadro de etapas".to_string();
        return (
            NAO,
            String::new(),
            if etiqueta.is_empty() {
                obs
            } else {
                format!("{obs} — {etiqueta}")
            },
        );
    }
    let nomes: Vec<&str> = etapas.iter().map(|e| e.nome.as_str()).collect();
    let atual_idx = etapas.iter().position(|e| e.atual);
    let atual = atual_idx.map(|i| nomes[i].to_string()).unwrap_or_default();
    let Some(idx_corte) = nomes.iter().position(|n| norm(n).contains(&corte)) else {
        let fluxo = nomes.join(" → ");
        return (
            NAO,
            atual,
            format!(
                "Fluxo sem {} ({})",
                v::ETAPA_CORTE,
                if etiqueta.is_empty() {
                    &fluxo
                } else {
                    etiqueta
                }
            ),
        );
    };
    let passou: Vec<&str> = etapas[idx_corte..]
        .iter()
        .filter(|e| !e.marca.is_empty() && !e.atual)
        .map(|e| e.nome.as_str())
        .collect();
    let Some(ai) = atual_idx else {
        if !passou.is_empty() {
            return (
                SIM,
                String::new(),
                format!("Quadro sem etapa atual; registro em {}", passou.join(", ")),
            );
        }
        return (
            INDET,
            String::new(),
            "Quadro sem etapa atual marcada".into(),
        );
    };
    if ai >= idx_corte {
        let obs = if ai == idx_corte {
            "Na etapa de corte".to_string()
        } else {
            format!("Além de {}", v::ETAPA_CORTE)
        };
        return (SIM, atual, obs);
    }
    if !passou.is_empty() {
        return (
            SIM,
            atual,
            format!("Retornou após passar por {}", passou.join(", ")),
        );
    }
    (NAO, atual, format!("Antes de {}", v::ETAPA_CORTE))
}

// ============================================================
// LISTAGEM
// ============================================================

#[derive(Clone, Debug, Default)]
pub struct LinhaLista {
    pub hash: String,
    pub numero: String,
    pub etiqueta: String,
    pub prioridade: String,
    pub emissao: Option<NaiveDate>,
    pub caixas: Vec<String>,
}

/// Linhas da listagem: hash, número, etiqueta (title do badge de assunto),
/// prioridade e data de emissão lida na linha.
pub fn linhas_listagem(fonte: &str) -> Vec<LinhaLista> {
    let doc = html::doc(fonte);
    let mut linhas = Vec::new();
    for a in doc.select(sel!("a.link_emissao_a")) {
        let h = v::hash_de(html::attr_ou_vazio(a, "href"));
        let tr = html::ancestral_tag(a, "tr");
        let num = ler_numero(&html::texto(a, " "));
        let (Some(h), Some(tr), Some(num)) = (h, tr, num) else {
            continue;
        };
        let (mut etiqueta, mut prioridade) = (String::new(), String::new());
        for el in tr.select(sel!("[title], [data-original-title]")) {
            let t_ = limpo(
                html::attr(el, "title")
                    .filter(|t| !t.is_empty())
                    .or_else(|| html::attr(el, "data-original-title"))
                    .unwrap_or(""),
            );
            if t_ == "Prioridade" {
                if prioridade.is_empty() {
                    prioridade = limpo(&html::texto(el, ""));
                }
            } else if etiqueta.is_empty()
                && (re!(RE_ETIQ_ALVO).is_match(&norm(&t_))
                    || re!(v::RE_ETIQ_INDICE).is_match(&norm(&t_)))
            {
                etiqueta = t_;
            }
        }
        let emissao = html::um(a, sel!("small.data")).and_then(|el| {
            let t = html::texto(el, " ");
            re!(r"(\d{2})/(\d{2})/(\d{4})").captures(&t).and_then(|m| {
                NaiveDate::from_ymd_opt(m[3].parse().ok()?, m[2].parse().ok()?, m[1].parse().ok()?)
            })
        });
        linhas.push(LinhaLista {
            hash: h,
            numero: num.0,
            etiqueta,
            prioridade,
            emissao,
            caixas: Vec::new(),
        });
    }
    linhas
}

/// O que todas as threads compartilham.
pub struct Ctx<'a> {
    pub amb: &'a Ambiente,
    pub http: Http,
    pub cache: Cache,
    pub driver: Option<NavegadorSobDemanda>,
    pub hoje: NaiveDate,
    pub de: NaiveDate,
    pub ate: NaiveDate,
    indice: Mutex<Option<HashMap<String, Vec<String>>>>,
    listagens: Mutex<HashMap<(String, NaiveDate, NaiveDate), Vec<LinhaLista>>>,
    pub planilha: HashSet<String>,
}

/// Todas as páginas de uma caixa (só Proc. Administrativos) emitidos de
/// `de` a `ate`. Guardada em ctx: cada caixa é listada uma vez por período.
fn listar_caixa(ctx: &Ctx, caixa: &str, de: NaiveDate, ate: NaiveDate) -> Res<Vec<LinhaLista>> {
    let chave = (caixa.to_string(), de, ate);
    let mut guard = ctx.listagens.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(l) = guard.get(&chave) {
        return Ok(l.clone());
    }
    let (mut linhas, mut pagina, mut total): (Vec<LinhaLista>, i64, Option<i64>) =
        (Vec::new(), 1, None);
    while pagina
        <= total
            .filter(|t| *t > 0)
            .unwrap_or(MAX_PAGINAS_CAIXA)
            .min(MAX_PAGINAS_CAIXA)
    {
        let params = [
            ("chamada", "caixa".to_string()),
            ("pg", "painel/listar".into()),
            ("caixa", caixa.into()),
            ("num_pagina", pagina.to_string()),
            ("ordenar", "1".into()),
            ("ordem", "desc".into()),
            ("modo_visualizacao", "1".into()),
            ("intervalo", datas::texto_periodo(de, ate, " - ")),
            ("emissoes_arquivado_prazos", "0".into()),
            ("itd", v::ID_DOCUMENTO.into()),
        ];
        let fonte = v::html_listagem(&ctx.http.baixar(
            &format!("{}/link.php", ctx.amb.url_1doc),
            &params,
            true,
            Some(&v::url_painel(ctx.amb)),
        )?);
        let achadas = linhas_listagem(&fonte);
        if achadas.is_empty() {
            break;
        }
        let n = achadas.len();
        linhas.extend(
            achadas
                .into_iter()
                .filter(|l| l.emissao.is_none_or(|e| de <= e && e <= ate)),
        );
        match v::total_paginas(&fonte) {
            Some(t) => total = Some(t),
            None if total.is_none() && n < 10 => break, // sem contador: página curta = última
            None => {}
        }
        pagina += 1;
    }
    guard.insert(chave, linhas.clone());
    Ok(linhas)
}

/// origem (hash e número) → [hashes dos processos emitidos a partir dela].
fn montar_indice(ctx: &Ctx) -> Res<HashMap<String, Vec<String>>> {
    let mut guard = ctx.indice.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(i) = guard.as_ref() {
        return Ok(i.clone());
    }
    let (de, ate) = (ctx.de, ctx.hoje); // até hoje: o processo novo vem depois da elaboração
    msg!(
        ctx.amb,
        "\n  … montando o índice de origem (caixas do setor, emissões de {})",
        datas::texto_periodo(de, ate, " a ")
    );
    let (mut candidatos, mut falhas): (Vec<(String, String)>, Vec<String>) =
        (Vec::new(), Vec::new());
    for (caixa, rotulo) in v::CAIXAS_INDICE {
        match listar_caixa(ctx, caixa, de, ate) {
            Ok(l) => {
                for ln in l {
                    if re!(v::RE_ETIQ_INDICE).is_match(&norm(&ln.etiqueta)) {
                        match candidatos.iter_mut().find(|(h, _)| *h == ln.hash) {
                            Some(c) => c.1 = ln.etiqueta,
                            None => candidatos.push((ln.hash, ln.etiqueta)),
                        }
                    }
                }
            }
            Err(e) if e.eh_http() => {
                falhas.push(format!("{rotulo} ({})", corta(&e.to_string(), 40)))
            }
            Err(e) => return Err(e),
        }
    }
    let faltam: Vec<String> = candidatos
        .iter()
        .map(|c| c.0.clone())
        .filter(|h| !ctx.cache.contem(&format!("origem:{h}")))
        .collect();
    paralelo::mapear(
        &faltam,
        v::WORKERS,
        |h| {
            let pagina = match ctx.http.detalhe(h) {
                Ok(p) => p,
                Err(e) if e.eh_http() => return Ok(()),
                Err(e) => return Err(e),
            };
            let (o_hash, o_num) = v::ler_origem(&pagina);
            ctx.cache.por(
                &format!("origem:{h}"),
                json!([o_hash.unwrap_or_default(), o_num.unwrap_or_default()]),
            );
            Ok(())
        },
        |_, _| Ok(()),
    )?;
    let origem = |h: &str| -> (String, String) {
        let v = ctx
            .cache
            .get(&format!("origem:{h}"))
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default();
        let s = |i: usize| v.get(i).and_then(Value::as_str).unwrap_or("").to_string();
        (s(0), s(1))
    };
    let mut indice: HashMap<String, Vec<String>> = HashMap::new();
    let mut com_origem = 0;
    for (h, _) in &candidatos {
        let (o_hash, o_num) = origem(h);
        if !o_hash.is_empty() {
            com_origem += 1;
        }
        let chaves: HashSet<String> = [o_hash, o_num]
            .into_iter()
            .filter(|x| !x.is_empty())
            .collect();
        for k in chaves {
            indice.entry(k).or_default().push(h.clone());
        }
    }
    let aviso = if falhas.is_empty() {
        String::new()
    } else {
        format!("; ⚠ falha em {}", falhas.join(", "))
    };
    msg!(
        ctx.amb,
        "  ✓ índice: {} processo(s) de abertura/fase preparatória nas caixas, {} lido(s) agora, {com_origem} com origem registrada{aviso}\n",
        candidatos.len(),
        faltam.len()
    );
    *guard = Some(indice.clone());
    Ok(indice)
}

// ============================================================
// NOVO PROCESSO EMITIDO A PARTIR DA ELABORAÇÃO
// ============================================================

#[derive(Clone, Debug)]
pub struct Novo {
    pub numero: String,
    pub ano: String,
    pub hash: String,
    pub url: String,
    pub veredito: &'static str,
    pub etapa: String,
    pub obs: String,
    pub vinculo: &'static str,
    pub via: Option<Box<Novo>>,
}

/// O neto conta pelo filho, se for mais adiantado.
pub fn efetivo(n: &Novo) -> &Novo {
    match &n.via {
        Some(via) if v::ordem(via.veredito) < v::ordem(n.veredito) => via,
        _ => n,
    }
}

type Avaliar<'a> = dyn FnMut(&str, &str, bool, &mut Vec<Novo>) -> Res<()> + 'a;

/// Processos novos emitidos a partir de `elab` (número, ano, hash).
fn buscar_novos(
    ctx: &Ctx,
    numero: &str,
    ano: &str,
    hash: &str,
    fonte: &str,
    nivel: u32,
) -> Res<Vec<Novo>> {
    let mut achados: Vec<Novo> = Vec::new();
    let vistos: std::cell::RefCell<HashSet<String>> =
        std::cell::RefCell::new(HashSet::from([hash.to_string()]));
    let chave_cache = format!("novo:{numero}");

    let mut avaliar_candidato =
        |h: &str, pagina: &str, citado: bool, achados: &mut Vec<Novo>| -> Res<()> {
            if !vistos.borrow_mut().insert(h.to_string()) {
                return Ok(());
            }
            let Some(nome) = ler_numero(&v::ler_nome_em(pagina).unwrap_or_default()) else {
                return Ok(());
            };
            if nome.0 == numero {
                return Ok(());
            }
            let (o_hash, o_num) = v::ler_origem(pagina);
            let vinculo = if o_hash.as_deref() == Some(hash) || o_num.as_deref() == Some(numero) {
                "origem no 1Doc"
            } else if o_hash.is_none() && citado {
                "citado em despacho"
            } else {
                return Ok(());
            };
            let etapas = ler_quadro(pagina);
            if vinculo == "citado em despacho" && v::sem_etapa_corte(&etapas) {
                return Ok(()); // citação fraca só vale para processo de abertura
            }
            let (veredito, etapa, obs) = avaliar(&etapas, &ler_etiqueta(pagina));
            let mut novo = Novo {
                numero: nome.0.clone(),
                ano: nome.2.clone(),
                hash: h.to_string(),
                url: v::url_detalhe(ctx.amb, h),
                veredito,
                etapa,
                obs,
                vinculo,
                via: None,
            };
            if v::sem_etapa_corte(&etapas) && nivel < 2 {
                // elaboração → fase prep. → abertura
                let netos = buscar_novos(ctx, &novo.numero, &novo.ano, h, pagina, nivel + 1)?;
                novo.via = netos.into_iter().next().map(Box::new);
            }
            achados.push(novo);
            Ok(())
        };

    let por_numero =
        |numero_c: &str, citado: bool, achados: &mut Vec<Novo>, aval: &mut Avaliar| -> Res<()> {
            let Some(item) = Item::do_numero(numero_c) else {
                return Ok(());
            };
            if let Some(r) = v::abrir(
                &ctx.http,
                &ctx.cache,
                ctx.driver.as_ref(),
                &item,
                None,
                &confere,
            )? {
                aval(&r.hash, &r.pagina, citado, achados)?;
            }
            Ok(())
        };
    let por_hash =
        |h: &str, citado: bool, achados: &mut Vec<Novo>, aval: &mut Avaliar| -> Res<()> {
            if vistos.borrow().contains(h) {
                return Ok(());
            }
            match ctx.http.detalhe(h) {
                Ok(p) => aval(h, &p, citado, achados),
                Err(e) if e.eh_http() => Ok(()),
                Err(e) => Err(e),
            }
        };

    let do_cache: Vec<String> = ctx
        .cache
        .get(&chave_cache)
        .and_then(|v| v.as_array().cloned())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    for n in do_cache {
        por_numero(&n, true, &mut achados, &mut avaliar_candidato)?;
    }
    let (hashes, numeros) = v::citacoes(fonte, numero, ano);
    for h in hashes {
        por_hash(&h, true, &mut achados, &mut avaliar_candidato)?;
    }
    for n in numeros {
        if !achados.iter().any(|a| a.numero == n) {
            por_numero(&n, true, &mut achados, &mut avaliar_candidato)?;
        }
    }
    if USAR_INDICE_ORIGEM {
        let indice = montar_indice(ctx)?;
        let alvos: Vec<String> = indice
            .get(hash)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .chain(indice.get(numero).cloned().unwrap_or_default())
            .collect();
        for h in alvos {
            por_hash(&h, false, &mut achados, &mut avaliar_candidato)?;
        }
    }
    if !achados.is_empty() {
        let mut nums: Vec<String> = achados
            .iter()
            .map(|a| a.numero.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        nums.sort();
        ctx.cache.por(&chave_cache, json!(nums));
    }
    achados.sort_by_key(|a| (v::ordem(efetivo(a).veredito), a.vinculo != "origem no 1Doc"));
    Ok(achados)
}

// ============================================================
// CAMPOS DO DETALHE (prioridade, objeto, secretaria, despachos)
// ============================================================

/// Texto de HTML com as entidades resolvidas (`html.unescape`).
fn desescapar(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    html::texto_doc(
        &html::doc(&format!("<p>{}</p>", s.replace('<', "&lt;"))),
        "",
    )
}

/// 1ª ocorrência = rótulo do cabeçalho do processo (Urgente, Média...).
pub fn ler_prioridade(fonte: &str) -> String {
    re!(r#"(?i)<span\b[^>]*\btitle\s*=\s*["']Prioridade["'][^>]*>\s*([^<]*?)\s*<"#)
        .captures(fonte)
        .map(|m| limpo(&desescapar(&m[1])))
        .unwrap_or_default()
}

/// Texto do h4.tit_assunto sem o badge da etiqueta.
pub fn ler_objeto(fonte: &str) -> String {
    let Some(m) = re!(r#"(?i)<h4\b[^>]*class\s*=\s*["'][^"']*\btit_assunto\b"#).find(fonte) else {
        return String::new();
    };
    let doc = html::doc(crate::texto::fatia(fonte, m.start(), 4000));
    let Some(h4) = html::um_doc(&doc, sel!("h4.tit_assunto")) else {
        return String::new();
    };
    limpo(&html::texto_sem(h4, sel!(".badge, span[title]"), " "))
}

/// Campo 'Descrição do objeto' do formulário da Abertura.
pub fn ler_objeto_abertura(fonte: &str) -> String {
    let Some(m) =
        re!(r#"(?i)<div\b[^>]*\bid\s*=\s*["']campo_adicional_descricao_objeto["']"#).find(fonte)
    else {
        return String::new();
    };
    let trecho = match re!(r#"(?i)\bid\s*=\s*["']campo_adicional_"#).find_at(fonte, m.end()) {
        Some(p) => &fonte[m.start()..p.start()],
        None => crate::texto::fatia(fonte, m.start(), 20000),
    };
    let doc = html::doc(trecho);
    let campo = html::um_doc(&doc, sel!("#campo_adicional_descricao_objeto"))
        .unwrap_or_else(|| html::raiz(&doc));
    let valor = html::um(campo, sel!("[data-valor]"))
        .map(|s| limpo(html::attr_ou_vazio(s, "data-valor")))
        .unwrap_or_default();
    if !valor.is_empty() {
        return valor;
    }
    html::um(campo, sel!("small"))
        .map(|s| limpo(&html::texto(s, " ")))
        .unwrap_or_default()
}

/// Processo na fase de abertura: etiqueta de Abertura ou fluxo com a etapa de corte.
pub fn eh_abertura(etapas: &[Etapa], etiqueta: &str) -> bool {
    re!(r"abertura de procedimento").is_match(&norm(etiqueta)) || !v::sem_etapa_corte(etapas)
}

/// Secretaria do setor de origem (1º nível do title do badge do setor).
pub fn ler_secretaria(fonte: &str) -> String {
    let alvo = re!(r#"(?i)<input\b[^>]*\bid\s*=\s*["']id_setor_origem["'][^>]*>"#)
        .find(fonte)
        .and_then(|m| {
            re!(r#"(?i)\bvalue\s*=\s*["']([^"']*)["']"#)
                .captures(m.as_str())
                .map(|v| v[1].to_string())
        });
    let mut primeiro: Option<String> = None;
    for b in
        re!(r#"(?i)<span\b[^>]*\bdata-id_setor\s*=\s*["'](\d+)["'][^>]*>"#).captures_iter(fonte)
    {
        let Some(t) = re!(r#"(?i)\btitle\s*=\s*["']([^"']*)["']"#).captures(&b[0]) else {
            continue;
        };
        let des = desescapar(&t[1]);
        let nome = limpo(des.split('»').next().unwrap_or(""));
        if alvo.is_none() || alvo.as_deref() == Some(&b[1]) {
            return nome;
        }
        primeiro.get_or_insert(nome);
    }
    primeiro.unwrap_or_default()
}

#[derive(Clone, Debug, Default)]
pub struct Despacho {
    pub rotulo: String,
    pub data: String,
    pub texto: String,
}

/// Despachos em ordem cronológica; sem despacho nenhum, o texto de abertura.
pub fn ler_despachos(fonte: &str) -> Vec<Despacho> {
    let doc = html::doc(fonte);
    let mut itens = Vec::new();
    for t in doc.select(sel!("table")) {
        if !html::tem_classe(t, "despacho") {
            continue;
        }
        let Some(cel) = html::um(t, sel!("td.celula_despacho")) else {
            continue;
        };
        let lixo: Vec<_> = cel
            .select(sel!(".dropdown-menu, .btn-group, script, style"))
            .map(|e| e.id())
            .collect();
        let fora =
            |e: ElementRef| lixo.contains(&e.id()) || e.ancestors().any(|a| lixo.contains(&a.id()));
        let mut partes = vec![html::texto_pulando(cel, " ", &|e| lixo.contains(&e.id()))];
        partes.extend(
            cel.select(sel!("[title], [data-original-title]"))
                .filter(|e| !fora(*e))
                .map(|e| {
                    html::attr(e, "title")
                        .filter(|t| !t.is_empty())
                        .or_else(|| html::attr(e, "data-original-title"))
                        .unwrap_or("")
                        .to_string()
                }),
        );
        let cab = html::um(t, sel!(".rf_header strong"));
        let data = html::um(t, sel!("abbr.despacho_data[title]"))
            .map(|d| html::attr_ou_vazio(d, "title"))
            .unwrap_or("");
        let data = re!(r"(\d{2}/\d{2}/\d{4})(?:\s+(\d{2}:\d{2}))?")
            .captures(data)
            .map(|m| {
                [m.get(1), m.get(2)]
                    .into_iter()
                    .flatten()
                    .map(|g| g.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default();
        itens.push(Despacho {
            rotulo: cab.map(|c| limpo(&html::texto(c, " "))).unwrap_or_default(),
            data,
            texto: limpo(&partes.join(" ")),
        });
    }
    if !itens.is_empty() {
        return itens;
    }
    if let Some(m) = re!(r#"class\s*=\s*["'][^"']*\bemissao_conteudo\b"#).find(fonte) {
        // html[m.start() - 200 : m.start() + 30000], com o fatiamento do Python
        let chars_antes = fonte[..m.start()].chars().count();
        let trecho: String = if chars_antes >= 200 {
            let ini = fonte[..m.start()]
                .char_indices()
                .rev()
                .nth(199)
                .map(|(i, _)| i)
                .unwrap_or(0);
            crate::texto::fatia(fonte, ini, 30200).to_string()
        } else {
            let total = fonte.chars().count();
            let ini = total.saturating_sub(200 - chars_antes);
            let fim = chars_antes + 30000;
            fonte
                .chars()
                .skip(ini)
                .take(fim.saturating_sub(ini))
                .collect()
        };
        let frag = html::doc(&trecho);
        if let Some(orig) = html::um_doc(&frag, sel!(".texto_original")) {
            return vec![Despacho {
                rotulo: "Abertura (sem despachos)".into(),
                data: String::new(),
                texto: limpo(&html::texto(orig, " ")),
            }];
        }
    }
    Vec::new()
}

/// Posição (em caracteres, no texto original) da 1ª menção a cancelamento
/// sem negação imediatamente antes, ou None.
pub fn posicao_cancelamento(texto: &str) -> Option<usize> {
    let t: String = texto.nfc().collect();
    let base: String = t
        .chars()
        .map(|ch| {
            let s = sem_acento(&ch.to_string());
            if s.is_empty() { ch.to_string() } else { s }
        })
        .collect::<String>()
        .to_lowercase();
    let chars: Vec<(usize, char)> = base.char_indices().collect();
    let idx_de = |byte: usize| {
        chars
            .iter()
            .position(|(i, _)| *i == byte)
            .unwrap_or(chars.len())
    };
    for m in re!(r"\bcancel\w*").find_iter(&base) {
        let pos = idx_de(m.start());
        let ini = pos.saturating_sub(JANELA_NEG);
        let antes: String = chars[ini..pos].iter().map(|(_, c)| *c).collect();
        let antes = norm(&antes);
        if !re!(r"\b(nao ha|nao houve|nao havera|nao se trata de|nao e caso de|sem|nenhum|nenhuma|inexiste|inexistencia de)( \w+)? ?$").is_match(&antes) {
            return Some(pos);
        }
    }
    None
}

pub fn menciona_cancelamento(texto: &str) -> bool {
    posicao_cancelamento(texto).is_some()
}

/// Despacho (entre os N últimos) que menciona cancelamento, ou None.
pub fn cancelamento_em(despachos: &[Despacho]) -> Option<&Despacho> {
    despachos[despachos.len().saturating_sub(N_ULTIMOS_DESPACHOS)..]
        .iter()
        .find(|d| menciona_cancelamento(&d.texto))
}

/// Trecho de ~2*raio caracteres em volta de `pos`, cortado em palavra.
pub fn trecho_em_volta(texto: &str, pos: usize, raio: usize) -> String {
    let t: Vec<char> = texto.nfc().collect();
    let (ini, fim) = (pos.saturating_sub(raio), (pos + raio).min(t.len()));
    let mut trecho: String = t[ini..fim].iter().collect();
    if ini > 0 {
        trecho = format!(
            "… {}",
            trecho.split_once(' ').map(|x| x.1).unwrap_or(&trecho)
        );
    }
    if fim < t.len() {
        trecho = format!("{} …", crate::texto::ate_ultimo_espaco(&trecho));
    }
    trecho
}

// ============================================================
// CLASSIFICAÇÃO DE CADA PROCESSO DAS CAIXAS
// ============================================================

pub const FALTA: &str = "Faltando na planilha";
pub const FORA: &str = "Fora do escopo (assunto)";
pub const PASSOU: &str = "Na Pesquisa de Mercado ou além";
pub const PASSOU_NOVO: &str = "Processo novo na Pesquisa de Mercado ou além";
pub const CANCELADO: &str = "Cancelamento no último despacho";
pub const VINCULADO: &str = "Vinculado a processo da planilha";
pub const CONCLUIDO: &str = "Etapa Final sem processo novo";
pub const INDETERM: &str = "Indeterminado";
pub const SEM_ASSUNTO: &str = "Assunto não lido";
pub const ERRO: &str = "Erro ao abrir";

#[derive(Clone, Debug, Default)]
pub struct Classificado {
    pub linha: LinhaLista,
    pub ano: String,
    pub url: String,
    pub situacao: String,
    pub objeto: String,
    pub etapa: String,
    pub secretaria: String,
    pub ult_data: String,
    pub ult_texto: String,
    pub obs: String,
    pub origem_num: String,
    pub novos: Vec<String>,
    pub trecho_cancel: String,
}

fn classificar(ctx: &Ctx, ln: &LinhaLista) -> Res<Classificado> {
    let ano = ler_numero(&ln.numero).map(|n| n.2).unwrap_or_default();
    let mut res = Classificado {
        linha: ln.clone(),
        ano,
        url: v::url_detalhe(ctx.amb, &ln.hash),
        situacao: ERRO.into(),
        ..Default::default()
    };
    // Já passou da Pesquisa numa execução anterior: estado terminal.
    if let Some(passou) = ctx
        .cache
        .get(&format!("passou:{}", ln.hash))
        .filter(crate::arquivos::verdadeiro)
    {
        let (sit, obs) = match &passou {
            Value::Array(a) => (
                a.first().and_then(Value::as_str).unwrap_or("").to_string(),
                a.get(1)
                    .map(|x| {
                        x.as_str()
                            .map(str::to_string)
                            .unwrap_or_else(|| x.to_string())
                    })
                    .unwrap_or_default(),
            ),
            outro => (
                PASSOU.to_string(),
                outro
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| outro.to_string()),
            ),
        };
        res.situacao = if sit == PASSOU || sit == PASSOU_NOVO {
            sit
        } else {
            PASSOU.into()
        };
        res.obs = obs;
        return Ok(res);
    }
    let chave_etiq = format!("etiq:{}", ln.hash);
    if let Some(e) = ctx.cache.texto(&chave_etiq)
        && let Some(resto) = e.strip_prefix("FORA|")
        && !resto.trim().is_empty()
        && !assunto_alvo(resto, "")
    {
        res.situacao = FORA.into();
        res.linha.etiqueta = resto.to_string();
        return Ok(res);
    }
    let fonte = match ctx.http.detalhe(&ln.hash) {
        Ok(f) => f,
        Err(e) if e.eh_http() => {
            res.obs = corta(&e.to_string(), 80);
            return Ok(res);
        }
        Err(e) => return Err(e),
    };
    let etiqueta = Some(ler_etiqueta(&fonte))
        .filter(|e| !e.is_empty())
        .unwrap_or_else(|| ln.etiqueta.clone());
    let etapas = ler_quadro(&fonte);
    let tem_corte = !v::sem_etapa_corte(&etapas);
    let objeto = Some(if eh_abertura(&etapas, &etiqueta) {
        ler_objeto_abertura(&fonte)
    } else {
        String::new()
    })
    .filter(|o| !o.is_empty())
    .unwrap_or_else(|| ler_objeto(&fonte));
    res.linha.etiqueta = etiqueta.clone();
    res.objeto = objeto;
    res.secretaria = ler_secretaria(&fonte);
    res.linha.prioridade = Some(ler_prioridade(&fonte))
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| ln.prioridade.clone());
    let id_assunto = ler_id_assunto(&fonte);
    if !tem_corte && !assunto_alvo(&etiqueta, &id_assunto) {
        if !etiqueta.is_empty() || !id_assunto.is_empty() {
            ctx.cache
                .por(&chave_etiq, json!(format!("FORA|{etiqueta}"))); // o assunto não muda
            res.situacao = FORA.into();
        } else {
            res.situacao = SEM_ASSUNTO.into();
            res.obs = "Assunto não lido na página (etiqueta e id vazios)".into();
        }
        return Ok(res);
    }
    let marcar_passou = |sit: &str, obs: &str| {
        ctx.cache
            .por(&format!("passou:{}", ln.hash), json!([sit, obs]))
    };
    let (veredito, etapa, obs) = avaliar(&etapas, &etiqueta);
    res.etapa = if !etapa.is_empty() {
        etapa
    } else if etapas.is_empty() {
        "(sem quadro de etapas)".into()
    } else {
        String::new()
    };
    res.obs = obs.clone();
    if veredito == SIM {
        res.situacao = PASSOU.into();
        marcar_passou(PASSOU, &obs);
        return Ok(res);
    }
    if veredito == INDET {
        res.situacao = INDETERM.into();
        return Ok(res);
    }
    let despachos = ler_despachos(&fonte);
    if let Some(u) = despachos.last() {
        res.ult_data = u.data.clone();
        res.ult_texto = u.texto.clone();
    }
    if let Some(d) = cancelamento_em(&despachos) {
        let onde = [&d.rotulo, &d.data]
            .into_iter()
            .find(|s| !s.is_empty())
            .cloned()
            .unwrap_or_else(|| "último despacho".into());
        res.situacao = CANCELADO.into();
        res.obs = format!("Cancelamento citado em {onde}");
        res.trecho_cancel =
            trecho_em_volta(&d.texto, posicao_cancelamento(&d.texto).unwrap_or(0), 150);
        return Ok(res);
    }
    let (_o_hash, o_num) = v::ler_origem(&fonte);
    res.origem_num = o_num.clone().unwrap_or_default();
    if let Some(o) = o_num.filter(|o| ctx.planilha.contains(o)) {
        res.situacao = VINCULADO.into();
        res.obs = format!("Emitido a partir de {o}, que está na planilha");
        return Ok(res);
    }
    if !tem_corte && VERIFICAR_NOVO_PROCESSO {
        let novos = buscar_novos(ctx, &ln.numero, &res.ano, &ln.hash, &fonte, 1)?;
        let mut todos = Vec::new();
        for n in &novos {
            todos.push(n.numero.clone());
            if let Some(v) = &n.via {
                todos.push(v.numero.clone());
            }
        }
        res.novos = todos.clone();
        let na_pl: Vec<&String> = todos.iter().filter(|n| ctx.planilha.contains(*n)).collect();
        if !na_pl.is_empty() {
            res.situacao = VINCULADO.into();
            res.obs = format!(
                "Processo novo {} está na planilha",
                na_pl
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            return Ok(res);
        }
        if let Some(n0) = novos.first() {
            let alvo = efetivo(n0);
            let cadeia = if !std::ptr::eq(alvo, n0) {
                format!("{} → {}", n0.numero, alvo.numero)
            } else {
                alvo.numero.clone()
            };
            if alvo.veredito == SIM {
                res.situacao = PASSOU_NOVO.into();
                res.obs = format!(
                    "Novo processo {cadeia}: {}",
                    if alvo.etapa.is_empty() {
                        &alvo.obs
                    } else {
                        &alvo.etapa
                    }
                );
                marcar_passou(PASSOU_NOVO, &res.obs);
                return Ok(res);
            }
            res.obs = format!(
                "Novo processo {cadeia} ({}), etapa {}",
                n0.vinculo,
                if alvo.etapa.is_empty() {
                    "-"
                } else {
                    &alvo.etapa
                }
            );
        } else {
            let atual = etapas.iter().find(|e| e.atual);
            if EXCLUIR_ETAPA_FINAL_SEM_NOVO && atual.is_some_and(|a| a.final_) {
                res.situacao = CONCLUIDO.into();
                res.obs = "Etapa Final, nenhum processo novo encontrado".into();
                return Ok(res);
            }
            res.obs = format!("{obs}; nenhum processo novo emitido encontrado");
        }
    }
    res.situacao = FALTA.into();
    Ok(res)
}

/// Elaboração e processo novo dela, ambos faltando: fica só o novo.
pub fn deduplicar(mut faltantes: Vec<Classificado>) -> (Vec<Classificado>, Vec<String>) {
    let nums: HashSet<String> = faltantes.iter().map(|r| r.linha.numero.clone()).collect();
    let mut sai: HashSet<String> = HashSet::new();
    for r in faltantes.iter_mut() {
        if nums.contains(&r.origem_num) {
            sai.insert(r.origem_num.clone());
            r.obs = format!("Origem {} também fora da planilha; {}", r.origem_num, r.obs);
        }
    }
    for r in &faltantes {
        if !sai.contains(&r.linha.numero)
            && r.novos
                .iter()
                .any(|n| nums.contains(n) && *n != r.linha.numero)
        {
            sai.insert(r.linha.numero.clone());
        }
    }
    let mut absorvidos: Vec<String> = sai.iter().cloned().collect();
    absorvidos.sort();
    (
        faltantes
            .into_iter()
            .filter(|r| !sai.contains(&r.linha.numero))
            .collect(),
        absorvidos,
    )
}

// ============================================================
// EXPORTAÇÃO
// ============================================================

pub const COLUNAS: [&str; 9] = [
    "Nº Processo",
    "Prioridade",
    "Objeto",
    "Etapa",
    "Secretaria",
    "Caixa",
    "Último despacho",
    "Trecho do último despacho",
    "Observação",
];
pub const TRECHO_MAX: usize = 300;

fn colunas_cancel() -> Vec<&'static str> {
    let mut c: Vec<&str> = COLUNAS[..7].to_vec();
    c.extend(["Trecho com a menção a cancelamento", "Observação"]);
    c
}

enum DataDesp {
    Nada,
    Data(NaiveDateTime),
    Texto(String),
}

/// '05/11/2026 10:00' → data e hora; o texto original se não casar; nada se vazio.
fn data_despacho(txt: &str) -> DataDesp {
    if txt.is_empty() {
        return DataDesp::Nada;
    }
    let t = txt.trim();
    if let Ok(d) = NaiveDateTime::parse_from_str(t, "%d/%m/%Y %H:%M") {
        return DataDesp::Data(d);
    }
    if let Ok(d) = NaiveDate::parse_from_str(t, "%d/%m/%Y") {
        return DataDesp::Data(d.and_hms_opt(0, 0, 0).unwrap());
    }
    DataDesp::Texto(txt.to_string())
}

fn trecho_ultimo(r: &Classificado) -> String {
    let t = &r.ult_texto;
    if crate::texto::compr(t) > TRECHO_MAX {
        return format!(
            "{} …",
            crate::texto::ate_ultimo_espaco(&corta(t, TRECHO_MAX))
        );
    }
    t.clone()
}

fn preencher_aba(
    ws: &mut rust_xlsxwriter::Worksheet,
    linhas: &[&Classificado],
    colunas: &[&str],
    trecho_de: &dyn Fn(&Classificado) -> String,
    nome_tabela: &str,
) -> Res<usize> {
    let ordem = |p: &str| match norm(p).as_str() {
        "urgente" => 0,
        "alta" => 1,
        "media" => 2,
        "baixa" => 3,
        _ => 9,
    };
    let mut linhas: Vec<&Classificado> = linhas.to_vec();
    linhas.sort_by_key(|r| {
        (
            ordem(&r.linha.prioridade),
            r.ano.parse::<i64>().unwrap_or(0),
            r.linha
                .numero
                .split('/')
                .next()
                .unwrap_or("")
                .replace('.', "")
                .parse::<i64>()
                .unwrap_or(0),
        )
    });
    // Valores (texto de cada célula para o layout) e datas
    let mut grade: Vec<Vec<(String, Option<NaiveDateTime>)>> = Vec::new();
    for r in &linhas {
        let data = data_despacho(&r.ult_data);
        let (dtxt, dval) = match &data {
            DataDesp::Nada => (String::new(), None),
            DataDesp::Texto(t) => (t.clone(), None),
            DataDesp::Data(d) => {
                let f = if chrono::Timelike::hour(d) != 0 || chrono::Timelike::minute(d) != 0 {
                    "%d/%m/%Y %H:%M"
                } else {
                    "%d/%m/%Y"
                };
                (d.format(f).to_string(), Some(*d))
            }
        };
        let vals = vec![
            r.linha.numero.clone(),
            r.linha.prioridade.clone(),
            r.objeto.clone(),
            r.etapa.clone(),
            r.secretaria.clone(),
            r.linha.caixas.join(", "),
            dtxt,
            trecho_de(r),
            r.obs.clone(),
        ];
        grade.push(
            vals.into_iter()
                .enumerate()
                .map(|(i, t)| (t, if i == 6 { dval } else { None }))
                .collect(),
        );
    }
    let ultima = linhas.len() as u32 + 1;
    let n_cols = colunas.len() as u32;
    let celula = |r: u32, c: u32| -> CelulaLayout {
        if r == 1 {
            return CelulaLayout {
                texto: colunas[(c - 1) as usize].to_string(),
                tamanho: 11.0,
                negrito: true,
            };
        }
        CelulaLayout {
            texto: grade[(r - 2) as usize][(c - 1) as usize].0.clone(),
            tamanho: 11.0,
            negrito: false,
        }
    };
    let (larguras, alturas) = v::calcular_layout(1, ultima, n_cols, &celula);

    let cab = Format::new()
        .set_bold()
        .set_font_color(Color::White)
        .set_background_color(Color::RGB(0x1F4E78))
        .set_text_wrap()
        .set_align(FormatAlign::VerticalCenter);
    let dado = Format::new().set_text_wrap().set_align(FormatAlign::Top);
    let link = dado
        .clone()
        .set_font_color(Color::RGB(0x0563C1))
        .set_underline(FormatUnderline::Single);
    for (i, linha) in grade.iter().enumerate() {
        let l = (i + 1) as u32;
        for (j, (txt, dt)) in linha.iter().enumerate() {
            let c = j as u16;
            if j == 0 {
                crate::planilha::link(ws, l, c, &linhas[i].url, txt, &link)?;
            } else if let Some(d) = dt {
                let f = if chrono::Timelike::hour(d) != 0 || chrono::Timelike::minute(d) != 0 {
                    "dd/mm/yyyy hh:mm"
                } else {
                    "dd/mm/yyyy"
                };
                ws.write_datetime_with_format(l, c, d, &dado.clone().set_num_format(f))
                    .map_err(xl)?;
            } else {
                crate::planilha::texto(
                    ws,
                    l,
                    c,
                    &crate::planilha::sem_controle(txt),
                    &if txt.is_empty() {
                        Format::new()
                    } else {
                        dado.clone()
                    },
                )?;
            }
        }
    }
    for c in 1..=n_cols {
        ws.set_column_width((c - 1) as u16, larguras[c as usize])
            .map_err(xl)?;
    }
    for (r, h) in alturas {
        ws.set_row_height(r - 1, h).map_err(xl)?;
    }
    // Filtro como Tabela (ListObject) com linha de cabeçalho declarada: no
    // LibreOffice vira intervalo de banco de dados com rótulos de coluna. A
    // tabela precisa de ao menos uma linha de dados: sem linhas, cobre a 2ª, vazia.
    let cols: Vec<TableColumn> = colunas
        .iter()
        .map(|t| TableColumn::new().set_header(*t).set_header_format(&cab))
        .collect();
    let tabela = Table::new()
        .set_name(nome_tabela)
        .set_style(TableStyle::Light1)
        .set_banded_rows(false)
        .set_banded_columns(false)
        .set_columns(&cols);
    ws.add_table(0, 0, ultima.max(2) - 1, (n_cols - 1) as u16, &tabela)
        .map_err(xl)?;
    ws.set_freeze_panes(1, 1).map_err(xl)?;
    Ok(linhas.len())
}

/// Aba 'Faltantes' e, se houver, aba 'Cancelados (conferir)'.
pub fn exportar_faltantes(
    faltantes: &[Classificado],
    destino: &Path,
    cancelados: &[Classificado],
) -> Res<usize> {
    let mut wb = Workbook::new();
    let ws = wb.add_worksheet();
    ws.set_name("Faltantes").map_err(xl)?;
    let f: Vec<&Classificado> = faltantes.iter().collect();
    let n = preencher_aba(ws, &f, &COLUNAS, &trecho_ultimo, "Faltantes")?;
    if !cancelados.is_empty() {
        let ws = wb.add_worksheet();
        ws.set_name("Cancelados (conferir)").map_err(xl)?;
        let c: Vec<&Classificado> = cancelados.iter().collect();
        let cc = colunas_cancel();
        preencher_aba(
            ws,
            &c,
            &cc,
            &|r| {
                if r.trecho_cancel.is_empty() {
                    trecho_ultimo(r)
                } else {
                    r.trecho_cancel.clone()
                }
            },
            "Cancelados",
        )?;
    }
    if let Some(p) = destino.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(p)?;
    }
    wb.save(destino).map_err(xl)?;
    Ok(n)
}

// ============================================================
// PRINCIPAL
// ============================================================

pub struct Opcoes {
    pub pasta_saida: Option<PathBuf>,
    pub abrir: Option<bool>,
    pub de: Option<NaiveDate>,
    pub ate: Option<NaiveDate>,
    pub pausa: f64,
    /// sem navegador invisível para o Localizar (testes)
    pub sem_navegador: bool,
}

impl Default for Opcoes {
    fn default() -> Self {
        Opcoes {
            pasta_saida: None,
            abrir: None,
            de: None,
            ate: None,
            pausa: v::PAUSA_REQ,
            sem_navegador: false,
        }
    }
}

/// Roda a verificação da planilha `caminho` (aba `aba`, None = a ativa),
/// olhando os processos emitidos de `de` a `ate`. Devolve o caminho da
/// planilha gerada, ou None se nenhuma foi gerada.
pub fn executar(
    amb: &Ambiente,
    caminho: &Path,
    aba: Option<&str>,
    cred: &Credenciais,
    op: Opcoes,
) -> Res<Option<PathBuf>> {
    let cfg = config::carregar(amb);
    let pasta_saida = op
        .pasta_saida
        .clone()
        .unwrap_or_else(|| config::pasta_saida(amb, &cfg));
    let abrir = op.abrir.unwrap_or(cfg.abrir_planilha);
    if caminho.as_os_str().is_empty() || !caminho.is_file() {
        return Err(Erro::encerrar(format!(
            "✗ Arquivo não encontrado: '{}'",
            caminho.display()
        )));
    }
    let pl = v::ler_planilha(caminho, aba)?;
    let na_planilha: HashSet<String> = pl.itens.iter().map(|i| i.numero.clone()).collect();
    let hoje = datas::hoje();
    let inicio_padrao = op.de.is_none();
    let (de, ate) = resolver_periodo(op.de, op.ate, &pl.itens, hoje)?;
    let igual = "=".repeat(64);
    msg!(amb, "{igual}");
    msg!(
        amb,
        "Planilha: {}  |  aba: {}",
        caminho.file_name().unwrap_or_default().to_string_lossy(),
        pl.aba
    );
    msg!(
        amb,
        "Coluna do número: {}  |  processos na planilha: {}",
        crate::planilha::letra(pl.col),
        na_planilha.len()
    );
    if !pl.ignoradas.is_empty() {
        let l: Vec<String> = pl
            .ignoradas
            .iter()
            .take(8)
            .map(|(r, _)| format!("linha {r}"))
            .collect();
        msg!(
            amb,
            "  ⚠ {} célula(s) sem número reconhecível: {}",
            pl.ignoradas.len(),
            l.join(", ")
        );
    }
    let padrao = if inicio_padrao && ANO_INICIAL.is_none() {
        "  (início padrão: 1º/jan do ano mais antigo da planilha)"
    } else {
        ""
    };
    msg!(
        amb,
        "Período de emissão: {}{padrao}",
        datas::texto_periodo(de, ate, " a ")
    );
    msg!(amb, "{igual}");

    let t0 = Instant::now();
    let ctx = Ctx {
        amb,
        http: Http::nova(amb, cred, op.pausa),
        cache: Cache::carregar(amb),
        // só abre se o Localizar por HTTP falhar
        driver: if op.sem_navegador {
            None
        } else {
            Some(NavegadorSobDemanda::novo(cred, None))
        },
        hoje,
        de,
        ate,
        indice: Mutex::new(None),
        listagens: Mutex::new(HashMap::new()),
        planilha: na_planilha.clone(),
    };
    let mut linhas: Vec<LinhaLista> = Vec::new();
    let (mut resultados, mut falhas_caixa, mut nada_a_verificar) = (Vec::new(), Vec::new(), false);
    let r = (|| -> Res<()> {
        ctx.http.baixar(&v::url_painel(amb), &[], false, None)?; // fixa o contexto do setor e testa os cookies
        msg!(amb, "✓ Sessão HTTP válida");
        msg!(
            amb,
            "\nCaixas do setor (Proc. Administrativos emitidos de {}):",
            datas::texto_periodo(de, ate, " a ")
        );
        for (caixa, rotulo) in CAIXAS_BUSCA {
            let achadas = match listar_caixa(&ctx, caixa, de, ate) {
                Ok(a) => a,
                Err(e) if e.eh_http() => {
                    falhas_caixa.push(rotulo.to_string());
                    msg!(amb, "  ✗ {rotulo}: {}", corta(&e.to_string(), 60));
                    continue;
                }
                Err(e) => return Err(e),
            };
            for ln in &achadas {
                let i = match linhas.iter().position(|x| x.hash == ln.hash) {
                    Some(i) => i,
                    None => {
                        linhas.push(LinhaLista {
                            caixas: Vec::new(),
                            ..ln.clone()
                        });
                        linhas.len() - 1
                    }
                };
                if !linhas[i].caixas.iter().any(|c| c == rotulo) {
                    linhas[i].caixas.push(rotulo.to_string());
                }
            }
            msg!(amb, "  {rotulo:<16} {:>5} processo(s)", achadas.len());
        }
        let fora: Vec<LinhaLista> = linhas
            .iter()
            .filter(|l| !na_planilha.contains(&l.numero))
            .cloned()
            .collect();
        nada_a_verificar = fora.is_empty() && falhas_caixa.len() < CAIXAS_BUSCA.len();
        msg!(
            amb,
            "  {:<16} {:>5}  |  já na planilha: {}  |  a verificar: {}\n",
            "únicos",
            linhas.len(),
            linhas.len() - fora.len(),
            fora.len()
        );
        let n = fora.len();
        paralelo::mapear(
            &fora,
            v::WORKERS,
            |ln| classificar(&ctx, ln),
            |i, r| {
                let i = i + 1;
                if [FORA, PASSOU, PASSOU_NOVO].contains(&r.situacao.as_str()) {
                    if i % 100 == 0 || i == n {
                        msg!(amb, "  [{i:>4}/{n}] … verificados");
                    }
                } else {
                    let icone = match r.situacao.as_str() {
                        FALTA => "✚",
                        CANCELADO => "×",
                        INDETERM | SEM_ASSUNTO => "?",
                        ERRO => "✗",
                        _ => "·",
                    };
                    let etapa = if r.etapa.is_empty() { "-" } else { &r.etapa };
                    msg!(
                        amb,
                        "  [{i:>4}/{n}] {icone} {:<12} {:<34} {:<28} {}",
                        r.linha.numero,
                        corta(&r.situacao, 34),
                        corta(etapa, 28),
                        corta(&r.obs, 60)
                    );
                }
                resultados.push(r);
                Ok(())
            },
        )
    })();
    ctx.cache.salvar(amb);
    if let Some(d) = &ctx.driver {
        d.fechar();
    }
    match r {
        Ok(()) => {}
        Err(Erro::SessaoExpirada(e)) => {
            sessao::esquecer(amb);
            msg!(
                amb,
                "\n✗ Sessão recusada: {e}. Execute de novo: o navegador vai abrir para o login."
            );
        }
        Err(e) => return Err(e),
    }

    if nada_a_verificar {
        // período sem processo, ou todos já na planilha: não é erro
        msg!(amb, "\n{igual}");
        if !linhas.is_empty() {
            msg!(
                amb,
                "○ Todos os processos emitidos de {} nas caixas ({}) já estão na planilha; nenhuma planilha exportada.",
                datas::texto_periodo(de, ate, " a "),
                linhas.len()
            );
        } else {
            msg!(
                amb,
                "○ Nenhum Proc. Administrativo emitido de {} nas caixas do setor; nenhuma planilha exportada.",
                datas::texto_periodo(de, ate, " a ")
            );
        }
        if !falhas_caixa.is_empty() {
            msg!(
                amb,
                "  ⚠ caixas não lidas: {} (resultado incompleto)",
                falhas_caixa.join(", ")
            );
        }
        msg!(amb, "{igual}");
        return Ok(None);
    }
    if resultados.is_empty() {
        return Err(Erro::encerrar(
            "✗ Nenhum processo verificado; planilha não gerada.",
        ));
    }

    let (faltantes, absorvidos) = deduplicar(
        resultados
            .iter()
            .filter(|r| r.situacao == FALTA)
            .cloned()
            .collect(),
    );
    let cancelados: Vec<Classificado> = resultados
        .iter()
        .filter(|r| r.situacao == CANCELADO)
        .cloned()
        .collect();
    let nome = caminho.file_stem().unwrap_or_default().to_string_lossy();
    let destino = pasta_saida.join(format!(
        "{nome}_faltantes_{}.xlsx",
        Local::now().format("%Y-%m-%d_%H%M")
    ));
    let gerou = !faltantes.is_empty() || !cancelados.is_empty();
    let n = if gerou {
        exportar_faltantes(&faltantes, &destino, &cancelados)?
    } else {
        0
    };

    let conta = |sit: &str| resultados.iter().filter(|r| r.situacao == sit).count();
    let lista = |sit: &str| {
        resultados
            .iter()
            .filter(|r| r.situacao == sit)
            .map(|r| r.linha.numero.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    msg!(amb, "\n{igual}");
    let abs = if absorvidos.is_empty() {
        String::new()
    } else {
        format!(
            "  (+{} elaboração(ões) representada(s) pelo processo novo: {})",
            absorvidos.len(),
            absorvidos.join(", ")
        )
    };
    msg!(amb, "FALTANDO NA PLANILHA: {}{abs}", faltantes.len());
    msg!(amb, "Verificados e deixados de fora:");
    for sit in [PASSOU, PASSOU_NOVO, CANCELADO, VINCULADO, CONCLUIDO, FORA] {
        if conta(sit) > 0 {
            msg!(amb, "  {sit:<42} {:>4}", conta(sit));
        }
    }
    for sit in [CANCELADO, INDETERM, SEM_ASSUNTO, ERRO] {
        if conta(sit) > 0 {
            msg!(amb, "  {sit} — {}", lista(sit));
        }
    }
    if !falhas_caixa.is_empty() {
        msg!(
            amb,
            "  ⚠ caixas não lidas: {} (resultado incompleto)",
            falhas_caixa.join(", ")
        );
    }
    msg!(amb, "Tempo: {:.0}s", t0.elapsed().as_secs_f64());
    if gerou {
        let c = if cancelados.is_empty() {
            String::new()
        } else {
            format!(
                "  ({} cancelado(s) na aba 'Cancelados (conferir)')",
                cancelados.len()
            )
        };
        msg!(amb, "✓ Planilha: {}{c}", destino.display());
        if n == 0 {
            msg!(
                amb,
                "○ Nenhum processo faltando; a planilha traz só os cancelados para conferência."
            );
        }
        if abrir {
            amb.abrir_arquivo(&destino);
        }
    } else {
        msg!(
            amb,
            "○ Nenhum processo faltando; nenhuma planilha exportada."
        );
    }
    msg!(amb, "{igual}");
    Ok(if gerou { Some(destino) } else { None })
}
