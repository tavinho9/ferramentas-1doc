//! Verifica se os processos de uma planilha já chegaram à "Pesquisa de
//! Mercado" (ou a etapa posterior) no 1Doc (porta de verificar_planilha.py;
//! as regras estão descritas lá).
//!
//! Saída: `<planilha>_verificada_AAAA-MM-DD_HHMM.xlsx`, com SÓ os processos
//! que chegaram à Pesquisa de Mercado ou passaram dela: a aba original,
//! formatação preservada, sem as linhas dos demais, mais as colunas
//! Etapa atual (1Doc) | Processo pai | Processo filho | Objeto (abertura) |
//! Observação; o número, o pai e o filho viram link para o 1Doc;
//! autofiltro em todas as colunas e cabeçalho congelado.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

use chrono::Local;
use scraper::ElementRef;
use serde_json::{Value, json};
use umya_spreadsheet::{Hyperlink, Worksheet};

use crate::ambiente::Ambiente;
use crate::erro::{Erro, Res};
use crate::sessao::{self, Credenciais, NavegadorSobDemanda};
use crate::texto::{corta, ler_numero, limpo, norm};
use crate::verificadores::{self as v, Cache, CelulaLayout, Etapa, Http, INDET, Item, NAO, SIM};
use crate::{config, html, msg, paralelo, re, sel};

/// True = a elaboração entra assim que o processo novo existir, em qualquer etapa.
pub const INCLUIR_NOVO_ANTES_CORTE: bool = true;
/// Quantos níveis descer na cadeia de emissões (DFD → Elaboração → Fase Preparatória → Abertura = 3).
pub const PROFUNDIDADE_MAX: u32 = 3;
pub const USAR_INDICE_ORIGEM: bool = true;
pub const VERIFICAR_NOVO_PROCESSO: bool = true;
pub const MAX_PAGINAS_INDICE: i64 = 300;
pub const CAMPO_OBJETO: &str = "descricao_objeto";

// ============================================================
// IDENTIFICAÇÃO E QUADRO
// ============================================================

/// 'Proc. Administrativo 31.243/2026' ou None: #nome_em ou, sem ele, o
/// cabeçalho h2.emission_infos_item_atual.
pub fn ler_identificacao(fonte: &str) -> Option<String> {
    if let Some(n) = v::ler_nome_em(fonte).filter(|n| !n.is_empty()) {
        return Some(n);
    }
    let m = re!(r#"(?i)class\s*=\s*["'][^"']*\bemission_infos_item_atual\b"#).find(fonte)?;
    let inicio = fonte[..m.start()].rfind('<').unwrap_or(0);
    let trecho = match fonte[m.end()..].find("</h2>") {
        Some(f) => &fonte[inicio..m.end() + f + 5],
        None => crate::texto::fatia(fonte, inicio, 3000),
    };
    let doc = html::doc(trecho);
    let (tipo, num) = (
        html::um_doc(&doc, sel!(".nd"))?,
        html::um_doc(&doc, sel!(".nd_num"))?,
    );
    Some(format!(
        "{} {}",
        limpo(&html::texto(tipo, " ")),
        limpo(&html::texto(num, " "))
    ))
}

/// True = o detalhe é do processo procurado; None = página sem identificação.
pub fn confere(pagina: &str, item: &Item) -> Option<bool> {
    let nome = ler_identificacao(pagina)?;
    Some(ler_numero(&nome).is_some_and(|l| l.1 == item.digitos && l.2 == item.ano))
}

/// Etapas do quadro VISÍVEL; a marca junta o tempo da etapa atual e o das percorridas.
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
            let marcas: Vec<String> = est
                .select(sel!(
                    ".kanban_itens .kanban_item, .kanban_itens .kanban-time"
                ))
                .map(|el| limpo(&html::texto(el, "")))
                .filter(|m| !m.is_empty())
                .collect();
            Etapa {
                nome: html::um(est, sel!(".kanban_titulo"))
                    .map(|t| limpo(&html::texto(t, "")))
                    .unwrap_or_default(),
                atual: html::attr_ou_vazio(est, "data-current").to_lowercase() == "true",
                marca: marcas.join("; "),
                final_: html::tem_classe(est, "estagio_final"),
            }
        })
        .collect()
}

/// O h4.tit_assunto do cabeçalho (objeto + badge do assunto), como HTML.
fn h4_assunto(fonte: &str) -> Option<String> {
    for m in re!(r#"(?i)class\s*=\s*["'][^"']*\btit_assunto\b"#).find_iter(fonte) {
        let inicio = fonte[..m.start()].rfind('<').unwrap_or(0);
        let trecho = match fonte[m.end()..].find("</h4>") {
            Some(f) => &fonte[inicio..m.end() + f + 5],
            None => crate::texto::fatia(fonte, inicio, 8000),
        };
        let doc = html::doc(trecho);
        if let Some(h4) = html::um_doc(&doc, sel!(".tit_assunto")) {
            return Some(h4.html());
        }
    }
    None
}

/// Assunto do badge em h4.tit_assunto, sem o prefixo 'Assunto:'.
pub fn ler_etiqueta(fonte: &str) -> String {
    let Some(h4) = h4_assunto(fonte) else {
        return String::new();
    };
    let doc = html::doc(&h4);
    let Some(b) = html::um_doc(&doc, sel!(".tit_assunto"))
        .and_then(|h| html::um(h, sel!(".badge, span[title]")))
    else {
        return String::new();
    };
    let t = html::attr(b, "title")
        .filter(|t| !t.is_empty())
        .or_else(|| html::attr(b, "data-original-title").filter(|t| !t.is_empty()))
        .map(str::to_string)
        .unwrap_or_else(|| html::texto(b, ""));
    limpo(&re!(r"^\s*Assunto\s*:\s*").replace(&t, ""))
}

/// Campo adicional "Objeto" do processo (#campo_adicional_descricao_objeto),
/// não o título. Vazio se o processo não tiver o campo.
pub fn ler_objeto(fonte: &str) -> String {
    let campos: Vec<regex::Captures> = re!(r#"id\s*=\s*["']campo_adicional_([A-Za-z0-9_]+)["']"#)
        .captures_iter(fonte)
        .collect();
    let mut blocos: Vec<(String, String)> = Vec::new();
    for (k, m) in campos.iter().enumerate() {
        let ini_m = m.get(0).unwrap();
        let inicio = fonte[..ini_m.start()].rfind('<').unwrap_or(0);
        let trecho = match campos.get(k + 1) {
            Some(p) => fonte[inicio..p.get(0).unwrap().start()].to_string(),
            None => {
                let fim_m = ini_m.end();
                let resto = crate::texto::fatia(fonte, fim_m, 20000);
                format!("{}{}", &fonte[inicio..fim_m], resto)
            }
        };
        let nome = m[1].to_string();
        match blocos.iter_mut().find(|(n, _)| *n == nome) {
            Some(b) => b.1 = trecho,
            None => blocos.push((nome, trecho)),
        }
    }
    let mut trecho = blocos
        .iter()
        .find(|(n, _)| n == CAMPO_OBJETO)
        .map(|b| b.1.clone());
    if trecho.is_none() {
        // outro modelo de formulário: o campo cujo rótulo é "Objeto"
        for (_, tr) in &blocos {
            let doc = html::doc(tr);
            if html::um_doc(&doc, sel!("[data-valor], span[for]"))
                .is_some_and(|sp| norm(&html::texto(sp, " ")) == "objeto")
            {
                trecho = Some(tr.clone());
                break;
            }
        }
    }
    let Some(trecho) = trecho else {
        return String::new();
    };
    let doc = html::doc(&trecho);
    let mut texto = html::um_doc(&doc, sel!("[data-valor]"))
        .map(|r| html::attr_ou_vazio(r, "data-valor").to_string())
        .unwrap_or_default();
    if texto.trim().is_empty() {
        texto = html::um_doc(&doc, sel!("small"))
            .map(|s| html::texto(s, " "))
            .unwrap_or_default();
    }
    // o data-valor pode trazer <br> (escapado na página, literal aqui)
    let texto = re!(r"(?i)<br\s*/?>").replace_all(&texto, "\n");
    let texto = re!(r"<[^>]+>").replace_all(&texto, " ");
    crate::texto::linhas(&texto)
        .into_iter()
        .filter(|l| !l.trim().is_empty())
        .map(limpo)
        .collect::<Vec<_>>()
        .join("\n")
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
        if ai == idx_corte {
            return (SIM, atual, "Na etapa de corte".into());
        }
        let mut obs = format!("Além de {}", v::ETAPA_CORTE);
        // Sem tempo em nenhuma coluna entre o corte e a atual: movido por cima delas.
        if etapas.iter().any(|e| !e.marca.is_empty() && !e.atual)
            && !etapas[idx_corte..ai].iter().any(|e| !e.marca.is_empty())
        {
            obs.push_str(&format!(
                " — sem registro de passagem por {}, conferir",
                v::ETAPA_CORTE
            ));
        }
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
// NOVO PROCESSO EMITIDO A PARTIR DA ELABORAÇÃO
// ============================================================

/// (hash, número, rótulo) dos Proc. Administrativos emitidos a partir desta
/// página ("É base de" e eventos "emitiu o … a partir deste documento").
pub fn ler_filhos(fonte: &str) -> Vec<(String, Option<String>, String)> {
    let doc = html::doc(fonte);
    let na_area = |e: ElementRef| {
        ["table_base", "marcou_quem"]
            .iter()
            .any(|c| html::tem_classe(e, c))
    };
    let mut filhos: Vec<(String, Option<String>, String)> = Vec::new();
    for a in doc.select(sel!("a[href]")) {
        if !(na_area(a) || html::ancestral(a, na_area).is_some()) {
            continue;
        }
        let href = html::attr_ou_vazio(a, "href");
        if !re!(r"origem=(?:emissao_anexada|emissao_evento_base)").is_match(href) {
            continue;
        }
        let texto = limpo(&html::texto(a, " "));
        let Some(h) = v::hash_de(href) else { continue };
        if !norm(&texto).contains("proc administrativo") {
            continue;
        }
        let num = ler_numero(&texto).map(|n| n.0);
        let rotulo = texto
            .split_once(" - ")
            .map(|x| limpo(x.1))
            .unwrap_or_default();
        match filhos.iter_mut().find(|f| f.0 == h) {
            None => filhos.push((h, num, rotulo)),
            Some(antigo) => {
                if (antigo.1.is_none() && num.is_some())
                    || (antigo.2.is_empty() && !rotulo.is_empty())
                {
                    antigo.1 = num.or(antigo.1.take());
                    if !rotulo.is_empty() {
                        antigo.2 = rotulo;
                    }
                }
            }
        }
    }
    filhos
}

/// Linhas da listagem: (hash, número, etiqueta).
pub fn linhas_listagem(fonte: &str) -> Vec<(String, String, String)> {
    let doc = html::doc(fonte);
    let mut linhas = Vec::new();
    for a in doc.select(sel!("a.link_emissao_a")) {
        let h = v::hash_de(html::attr_ou_vazio(a, "href"));
        let tr = html::ancestral_tag(a, "tr");
        let num = ler_numero(&html::texto(a, " "));
        let (Some(h), Some(tr), Some(num)) = (h, tr, num) else {
            continue;
        };
        let mut etiqueta = String::new();
        for el in tr.select(sel!("[title], [data-original-title]")) {
            let t_ = html::attr(el, "title")
                .filter(|t| !t.is_empty())
                .or_else(|| html::attr(el, "data-original-title"))
                .unwrap_or("");
            if re!(v::RE_ETIQ_INDICE).is_match(&norm(t_)) {
                etiqueta = limpo(t_);
                break;
            }
        }
        linhas.push((h, num.0, etiqueta));
    }
    linhas
}

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
    pub abertura: bool,
    pub objeto: String,
}

/// A cadeia a → via → via… chega a um processo de abertura?
fn tem_abertura(a: &Novo) -> bool {
    let mut atual = Some(a);
    while let Some(x) = atual {
        if x.abertura {
            return true;
        }
        atual = x.via.as_deref();
    }
    false
}

/// O descendente que decide pela cadeia: o mais adiantado; empate fica com o mais próximo.
pub fn efetivo(a: &Novo) -> &Novo {
    let (mut melhor, mut atual) = (a, a.via.as_deref());
    while let Some(x) = atual {
        if v::ordem(x.veredito) < v::ordem(melhor.veredito) {
            melhor = x;
        }
        atual = x.via.as_deref();
    }
    melhor
}

/// '28.133/2026 → 31.002/2026 → 39.144/2026' de a até alvo.
pub fn cadeia_ate(a: &Novo, alvo: &Novo) -> String {
    let (mut nums, mut atual) = (vec![a.numero.clone()], a);
    while !std::ptr::eq(atual, alvo) {
        let Some(via) = atual.via.as_deref() else {
            break;
        };
        atual = via;
        nums.push(atual.numero.clone());
    }
    nums.join(" → ")
}

pub struct Ctx<'a> {
    pub amb: &'a Ambiente,
    pub http: Http,
    pub cache: Cache,
    pub driver: Option<NavegadorSobDemanda>,
    pub ano_min: i32,
    indice: Mutex<Option<HashMap<String, Vec<String>>>>,
}

fn listar_caixa(ctx: &Ctx, caixa: &str, intervalo: &str) -> Res<Vec<(String, String, String)>> {
    let (mut linhas, mut pagina, mut total): (Vec<_>, i64, Option<i64>) = (Vec::new(), 1, None);
    while pagina
        <= total
            .filter(|t| *t > 0)
            .unwrap_or(MAX_PAGINAS_INDICE)
            .min(MAX_PAGINAS_INDICE)
    {
        let params = [
            ("chamada", "caixa".to_string()),
            ("pg", "painel/listar".into()),
            ("caixa", caixa.into()),
            ("num_pagina", pagina.to_string()),
            ("ordenar", "1".into()),
            ("ordem", "desc".into()),
            ("modo_visualizacao", "1".into()),
            ("intervalo", intervalo.into()),
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
        linhas.extend(achadas);
        match v::total_paginas(&fonte) {
            Some(t) => total = Some(t),
            None if total.is_none() && n < 10 => break,
            None => {}
        }
        pagina += 1;
    }
    Ok(linhas)
}

fn montar_indice(ctx: &Ctx) -> Res<HashMap<String, Vec<String>>> {
    let mut guard = ctx.indice.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(i) = guard.as_ref() {
        return Ok(i.clone());
    }
    let intervalo = format!(
        "01/01/{} - {}",
        ctx.ano_min,
        Local::now().format("%d/%m/%Y")
    );
    msg!(
        ctx.amb,
        "\n  … montando o índice de origem (caixas do setor, emissões de {intervalo})"
    );
    let (mut candidatos, mut falhas): (Vec<(String, String)>, Vec<String>) =
        (Vec::new(), Vec::new());
    for (caixa, rotulo) in v::CAIXAS_INDICE {
        match listar_caixa(ctx, caixa, &intervalo) {
            Ok(l) => {
                for (h, _num, etiqueta) in l {
                    if !etiqueta.is_empty() {
                        match candidatos.iter_mut().find(|(x, _)| *x == h) {
                            Some(c) => c.1 = etiqueta,
                            None => candidatos.push((h, etiqueta)),
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
    let mut indice: HashMap<String, Vec<String>> = HashMap::new();
    let mut com_origem = 0;
    for (h, _) in &candidatos {
        let o = ctx
            .cache
            .get(&format!("origem:{h}"))
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default();
        let s = |i: usize| o.get(i).and_then(Value::as_str).unwrap_or("").to_string();
        let (o_hash, o_num) = (s(0), s(1));
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

type Avaliar<'a> = dyn FnMut(&str, &str, bool, &mut Vec<Novo>) -> Res<()> + 'a;

/// Processos novos emitidos a partir de (número, ano, hash).
fn buscar_novos(
    ctx: &Ctx,
    numero: &str,
    ano: &str,
    hash: &str,
    fonte: &str,
    nivel: u32,
) -> Res<Vec<Novo>> {
    let mut achados: Vec<Novo> = Vec::new();
    let vistos = std::cell::RefCell::new(HashSet::from([hash.to_string()]));
    let chave_cache = format!("novo:{numero}");

    let mut avaliar_candidato =
        |h: &str, pagina: &str, citado: bool, achados: &mut Vec<Novo>| -> Res<()> {
            if !vistos.borrow_mut().insert(h.to_string()) {
                return Ok(());
            }
            let nome_em = ler_identificacao(pagina).unwrap_or_default(); // sem #nome_em: lê o cabeçalho
            let Some(nome) = ler_numero(&nome_em) else {
                return Ok(());
            };
            if nome.0 == numero || !norm(&nome_em).starts_with("proc administrativo") {
                return Ok(()); // memorando, parecer, ofício… emitido do processo
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
            let etiqueta = ler_etiqueta(pagina);
            let (veredito, etapa, obs) = avaliar(&etapas, &etiqueta);
            let abertura = !v::sem_etapa_corte(&etapas)
                || norm(&etiqueta).contains("abertura de procedimento");
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
                abertura,
                objeto: if abertura {
                    ler_objeto(pagina)
                } else {
                    String::new()
                },
            };
            if v::sem_etapa_corte(&etapas) && nivel < PROFUNDIDADE_MAX {
                let netos = buscar_novos(ctx, &novo.numero, &novo.ano, h, pagina, nivel + 1)?;
                novo.via = netos.into_iter().next().map(Box::new);
            }
            achados.push(novo);
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
    let por_numero =
        |n: &str, citado: bool, achados: &mut Vec<Novo>, aval: &mut Avaliar| -> Res<()> {
            let Some(item) = Item::do_numero(n) else {
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
    // "É base de" / "emitiu o … a partir deste documento": a própria página lista os filhos.
    for (h, _, _) in ler_filhos(fonte) {
        por_hash(&h, false, &mut achados, &mut avaliar_candidato)?;
    }
    let confirmado = achados
        .iter()
        .any(|a| a.vinculo == "origem no 1Doc" && tem_abertura(a));
    let (hashes, numeros) = v::citacoes(fonte, numero, ano);
    for h in hashes {
        por_hash(&h, true, &mut achados, &mut avaliar_candidato)?;
    }
    for n in numeros {
        if !achados.iter().any(|a| a.numero == n) {
            por_numero(&n, true, &mut achados, &mut avaliar_candidato)?;
        }
    }
    if USAR_INDICE_ORIGEM && !confirmado {
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
// PROCESSAMENTO DE CADA LINHA
// ============================================================

#[derive(Clone, Debug)]
pub struct Filho {
    pub numero: String,
    pub url: String,
    pub hash: String,
    pub rotulo: String,
}

#[derive(Clone, Debug, Default)]
pub struct Resultado {
    pub item: Item,
    pub hash: String,
    pub url: String,
    pub veredito: &'static str,
    pub etapa: String,
    pub obs: String,
    pub fonte: String,
    pub elaboracao: bool,
    pub novos: Vec<String>,
    pub novo: Option<Novo>,
    pub cadeia: String,
    pub pai: Option<(String, String)>,
    pub filhos: Vec<Filho>,
    pub objeto_filho: Option<String>,
}

/// A linha vai para a planilha?
pub fn entra(r: &Resultado) -> bool {
    if r.url.is_empty() {
        return false;
    }
    r.veredito == SIM || (INCLUIR_NOVO_ANTES_CORTE && r.novo.is_some())
}

/// Transfere para a linha da elaboração o veredito do processo novo.
fn aplicar_novo_processo(ctx: &Ctx, res: &mut Resultado, fonte: &str) -> Res<()> {
    let novos = buscar_novos(ctx, &res.item.numero, &res.item.ano, &res.hash, fonte, 1)?;
    res.novos = novos.iter().map(|n| n.numero.clone()).collect();
    let Some(principal) = novos.first() else {
        res.obs
            .push_str(" — nenhum novo processo emitido encontrado");
        return Ok(());
    };
    let alvo = efetivo(principal);
    let cadeia = cadeia_ate(principal, alvo);
    // objeto do processo de abertura da cadeia (o decisivo, se for de
    // abertura; senão o primeiro de abertura descendo a partir do filho)
    let mut ab = if alvo.abertura { Some(alvo) } else { None };
    let mut no = Some(principal);
    while ab.is_none() {
        let Some(n) = no else { break };
        ab = if n.abertura { Some(n) } else { None };
        no = n.via.as_deref();
    }
    if let Some(a) = ab.filter(|a| !a.objeto.is_empty()) {
        res.objeto_filho = Some(a.objeto.clone());
    }
    let outros: Vec<&String> = res
        .novos
        .iter()
        .filter(|n| **n != principal.numero)
        .collect();
    let mut obs = format!(
        "Novo processo {cadeia} ({}): {}",
        principal.vinculo, alvo.obs
    );
    if !outros.is_empty() {
        obs.push_str(&format!(
            "; também emitido(s): {}",
            outros
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    res.veredito = alvo.veredito;
    res.etapa = alvo.etapa.clone();
    res.obs = obs;
    res.cadeia = cadeia;
    res.novo = Some(alvo.clone());
    Ok(())
}

/// Linha que não é de elaboração mas tem filhos em "É base de": abre os de
/// abertura (no máximo 3) e leva o objeto deles.
fn objeto_dos_filhos(ctx: &Ctx, res: &mut Resultado) -> Res<()> {
    let mut objetos = Vec::new();
    let de_abertura: Vec<&Filho> = res
        .filhos
        .iter()
        .filter(|f| norm(&f.rotulo).contains("abertura de procedimento"))
        .take(3)
        .collect();
    for f in de_abertura {
        let pagina = match ctx.http.baixar(&f.url, &[], false, None) {
            Ok(p) => p,
            Err(e) if e.eh_http() => continue,
            Err(e) => return Err(e),
        };
        let (o_hash, o_num) = v::ler_origem(&pagina);
        if o_hash.as_deref() != Some(res.hash.as_str())
            && o_num.as_deref() != Some(res.item.numero.as_str())
        {
            continue;
        }
        let obj = ler_objeto(&pagina);
        if !obj.is_empty() {
            objetos.push(if res.filhos.len() == 1 {
                obj
            } else {
                format!("{}: {obj}", f.numero)
            });
        }
    }
    res.objeto_filho = Some(objetos.join("\n"));
    Ok(())
}

fn processar(ctx: &Ctx, item: &Item) -> Res<Resultado> {
    let mut res = Resultado {
        item: item.clone(),
        veredito: INDET,
        ..Default::default()
    };
    let Some(achado) = v::abrir(
        &ctx.http,
        &ctx.cache,
        ctx.driver.as_ref(),
        item,
        item.link_planilha.as_deref(),
        &confere,
    )?
    else {
        res.obs = "Processo não localizado no 1Doc".into();
        return Ok(res);
    };
    let fonte = &achado.pagina;
    res.hash = achado.hash.clone();
    res.url = v::url_detalhe(ctx.amb, &achado.hash);
    res.fonte = achado.fonte.clone();
    // de onde o processo veio (cabeçalho de origem) e o que foi emitido a partir dele
    let (pai_hash, pai_num) = v::ler_origem(fonte);
    if let Some(n) = pai_num {
        res.pai = Some((
            n,
            pai_hash
                .map(|h| v::url_detalhe(ctx.amb, &h))
                .unwrap_or_default(),
        ));
    }
    res.filhos = ler_filhos(fonte)
        .into_iter()
        .filter_map(|(h, n, rotulo)| {
            n.map(|numero| Filho {
                numero,
                url: v::url_detalhe(ctx.amb, &h),
                hash: h,
                rotulo,
            })
        })
        .collect();
    let etapas = ler_quadro(fonte);
    let etiqueta = ler_etiqueta(fonte);
    let (veredito, etapa, obs) = avaliar(&etapas, &etiqueta);
    res.veredito = veredito;
    res.etapa = etapa;
    res.obs = format!("{}{obs}", achado.aviso);
    // O processo da linha JÁ é a abertura: o objeto está na própria página.
    if !v::sem_etapa_corte(&etapas) || norm(&etiqueta).contains("abertura de procedimento") {
        res.objeto_filho = Some(ler_objeto(fonte));
    }
    if VERIFICAR_NOVO_PROCESSO && v::sem_etapa_corte(&etapas) {
        res.elaboracao = true;
        aplicar_novo_processo(ctx, &mut res, fonte)?;
    } else if !res.filhos.is_empty() && res.objeto_filho.as_deref().unwrap_or("").is_empty() {
        objeto_dos_filhos(ctx, &mut res)?;
    }
    Ok(res)
}

// ============================================================
// EXPORTAÇÃO
// ============================================================

/// Colunas acrescentadas no fim da aba, nesta ordem
pub const COLUNAS_NOVAS: [&str; 5] = [
    "Etapa atual (1Doc)",
    "Processo pai",
    "Processo filho",
    "Objeto (abertura)",
    "Observação",
];

/// Colunas que este script (ou versão anterior) gera: se a planilha de
/// entrada for uma saída anterior, saem e o bloco é refeito.
fn coluna_gerada(titulo: &str) -> bool {
    let n = norm(titulo);
    COLUNAS_NOVAS
        .iter()
        .chain(["Novo processo", "Objeto do processo filho"].iter())
        .any(|t| norm(t) == n)
}

/// [5, 6, 7, 10] → [(5, 3), (10, 1)]: (início, quantidade) de faixas contíguas.
pub fn blocos(linhas: &[u32]) -> Vec<(u32, u32)> {
    let mut v: Vec<u32> = linhas.to_vec();
    v.sort();
    let mut faixas: Vec<(u32, u32)> = Vec::new();
    for n in v {
        match faixas.last_mut() {
            Some(f) if n == f.0 + f.1 => f.1 += 1,
            _ => faixas.push((n, 1)),
        }
    }
    faixas
}

const AZUL: &str = "FF1F4E78";
const COR_LINK: &str = "FF0563C1";

fn por_link(ws: &mut Worksheet, c: u32, r: u32, url: &str, manter_fonte: bool) {
    let mut h = Hyperlink::default();
    h.set_url(url);
    ws.cell_mut((c, r)).set_hyperlink(h);
    let st = ws.style_mut((c, r));
    let f = st.font_mut();
    let (bold, size, nome) = (f.bold(), f.size(), f.name().to_string());
    let mut nova = umya_spreadsheet::Font::default();
    if manter_fonte {
        nova.set_bold(bold);
        nova.set_size(size);
        if !nome.is_empty() {
            nova.set_name(nome);
        }
    }
    nova.color_mut().set_argb_str(COR_LINK);
    nova.set_underline("single");
    st.set_font(nova);
}

/// Texto de uma célula como aparece (`_texto_celula`).
fn texto_celula(ws: &Worksheet, c: u32, r: u32) -> String {
    let Some(cel) = ws.cell((c, r)) else {
        return String::new();
    };
    let v = v::valor_celula(cel);
    if v.is_empty() {
        return v;
    }
    if cel.data_type() == "n" && !cel.is_formula() {
        let fmt = ws
            .style((c, r))
            .number_format()
            .map(|n| n.format_code().to_lowercase())
            .unwrap_or_default();
        if (fmt.contains('d') && fmt.contains('y')) || fmt.contains("yy") {
            return cel.formatted_value();
        }
        if let Ok(x) = v.parse::<f64>() {
            return if x.fract() == 0.0 && x.abs() < 1e15 {
                format!("{}", x as i64)
            } else {
                v::texto_numero(x)
            };
        }
    }
    v
}

fn ajustar_layout(ws: &mut Worksheet, lin_cab: u32, ultima_lin: u32, n_cols: u32) {
    let celula = |r: u32, c: u32| -> CelulaLayout {
        let st = ws.style((c, r));
        let (tamanho, negrito) = st
            .font()
            .map(|f| (if f.size() > 0.0 { f.size() } else { 11.0 }, f.bold()))
            .unwrap_or((11.0, false));
        CelulaLayout {
            texto: texto_celula(ws, c, r),
            tamanho,
            negrito,
        }
    };
    let (larguras, alturas) = v::calcular_layout(lin_cab, ultima_lin, n_cols, &celula);
    for c in 1..=n_cols {
        let col = ws.column_dimension_by_number_mut(c);
        col.set_width(larguras[c as usize]);
    }
    use umya_spreadsheet::VerticalAlignmentValues as V;
    for r in lin_cab..=ultima_lin {
        for c in 1..=n_cols {
            if texto_celula(ws, c, r).is_empty() {
                continue;
            }
            let al = ws.style_mut((c, r)).alignment_mut();
            al.set_wrap_text(true);
            if *al.vertical() == V::Bottom {
                al.set_vertical(if r == lin_cab { V::Center } else { V::Top });
            }
        }
    }
    for (r, h) in alturas {
        let d = ws.row_dimension_mut(r);
        d.set_height(h);
        d.set_custom_height(true);
    }
}

fn congelar(ws: &mut Worksheet, linha: u32) {
    let mut pane = umya_spreadsheet::Pane::default();
    pane.set_vertical_split((linha - 1) as f64);
    let mut coord = umya_spreadsheet::Coordinate::default();
    coord.set_coordinate(format!("A{linha}"));
    pane.set_top_left_cell(coord);
    pane.set_active_pane(umya_spreadsheet::PaneValues::BottomLeft);
    pane.set_state(umya_spreadsheet::PaneStateValues::Frozen);
    let vistas = ws.sheet_views_mut().sheet_view_list_mut();
    if vistas.is_empty() {
        ws.sheet_views_mut()
            .add_sheet_view_list_mut(umya_spreadsheet::SheetView::default());
    }
    if let Some(vv) = ws.sheet_views_mut().sheet_view_list_mut().first_mut() {
        vv.set_pane(pane);
    }
}

/// Mantém na aba só as linhas que chegaram à etapa de corte, acrescenta as
/// colunas novas e liga o autofiltro. Devolve quantas linhas ficaram; com
/// 0, não salva arquivo nenhum.
pub fn exportar(pl: &mut v::Planilha, resultados: &[Resultado], destino: &Path) -> Res<usize> {
    if !resultados.iter().any(entra) {
        return Ok(0);
    }
    let aba = pl.aba.clone();
    let (mut lin_cab, mut col_num) = (pl.lin_cab, pl.col);
    // Aba auxiliar de versões antigas do script, se a planilha de entrada for uma saída anterior
    if aba != "Mudaram" && pl.wb.sheet_by_name("Mudaram").is_ok() {
        let _ = pl.wb.remove_sheet_by_name("Mudaram");
    }
    let ws = pl.wb.sheet_by_name_mut(&aba).expect("aba");
    let (mut ult_lin, mut n_orig) = v::limpar_excesso(ws);
    // Tabelas do Excel têm filtro próprio e conflitam com o autofiltro da aba.
    ws.tables_mut().clear();
    let mut deslocar = 0;
    // Sem cabeçalho: cria um, porque o autofiltro precisa de linha de títulos
    if lin_cab < 1 {
        ws.insert_new_row(1, 1);
        for c in 1..=n_orig {
            let t = if c == col_num {
                "Nº Processo".to_string()
            } else {
                format!("Coluna {}", crate::planilha::letra(c))
            };
            ws.cell_mut((c, 1)).set_value(t);
            let st = ws.style_mut((c, 1));
            st.font_mut().set_bold(true);
            st.font_mut().color_mut().set_argb_str("FFFFFFFF");
            st.set_background_color(AZUL);
        }
        lin_cab = 1;
        deslocar = 1;
        ult_lin += 1;
    }
    let mut ficam: Vec<(u32, &Resultado)> = resultados
        .iter()
        .filter(|r| entra(r))
        .map(|r| (r.item.linha + deslocar, r))
        .collect();
    ficam.sort_by_key(|x| x.0);
    let manter: HashSet<u32> = ficam.iter().map(|x| x.0).collect();
    let remover: Vec<u32> = (lin_cab + 1..=ult_lin)
        .filter(|n| !manter.contains(n))
        .collect();
    let velhas: Vec<u32> = (1..=n_orig)
        .filter(|c| {
            *c != col_num
                && coluna_gerada(
                    &ws.cell((*c, lin_cab))
                        .map(v::valor_celula)
                        .unwrap_or_default(),
                )
        })
        .collect();
    // Desfaz as mesclagens que tocam a área de dados ou as colunas que saem
    // (o valor fica na célula superior esquerda).
    ws.merge_cells_mut().retain(|f| {
        let (c0, r0) = (
            f.coordinate_start_col().map(|c| c.num()).unwrap_or(0),
            f.coordinate_start_row().map(|r| r.num()).unwrap_or(0),
        );
        let (c1, r1) = (
            f.coordinate_end_col().map(|c| c.num()).unwrap_or(c0),
            f.coordinate_end_row().map(|r| r.num()).unwrap_or(r0),
        );
        let _ = r0;
        !(r1 > lin_cab || velhas.iter().any(|c| c0 <= *c && *c <= c1))
    });
    for c in velhas.iter().rev() {
        ws.remove_column_by_index(*c, 1); // da direita para a esquerda
    }
    col_num -= velhas.iter().filter(|c| **c < col_num).count() as u32;
    n_orig -= velhas.len() as u32;
    for (inicio, qtd) in blocos(&remover).into_iter().rev() {
        ws.remove_row(inicio, qtd); // de baixo para cima (os links acompanham as células)
    }
    ws.row_dimensions_to_hashmap_mut()
        .retain(|n, _| *n <= lin_cab);

    // Bloco das colunas novas no fim, com o estilo do cabeçalho do número
    let estilo_ref = ws.style((col_num, lin_cab)).clone();
    let col: Vec<u32> = (n_orig + 1..=n_orig + COLUNAS_NOVAS.len() as u32).collect();
    for (c, titulo) in col.iter().zip(COLUNAS_NOVAS) {
        ws.cell_mut((*c, lin_cab)).set_value(titulo);
        let mut st = estilo_ref.clone();
        st.remove_numbering_format();
        ws.set_style((*c, lin_cab), st);
    }
    let escrever = |ws: &mut Worksheet, linha: u32, i: usize, texto: &str, url: &str| {
        if !texto.is_empty() {
            ws.cell_mut((col[i], linha)).set_value(texto);
        }
        if !url.is_empty() {
            por_link(ws, col[i], linha, url, false);
        }
    };
    for (i, (_, r)) in ficam.iter().enumerate() {
        let linha = lin_cab + 1 + i as u32;
        escrever(ws, linha, 0, &r.etapa, "");
        let (pn, pu) = r.pai.clone().unwrap_or_default();
        escrever(ws, linha, 1, &pn, &pu);
        if let Some(n) = &r.novo {
            // filho conferido pela origem; a cadeia mostra os níveis até o processo que decidiu
            escrever(
                ws,
                linha,
                2,
                if r.cadeia.is_empty() {
                    &n.numero
                } else {
                    &r.cadeia
                },
                &n.url,
            );
        } else if !r.filhos.is_empty() {
            let nums: Vec<&str> = r.filhos.iter().map(|f| f.numero.as_str()).collect();
            escrever(ws, linha, 2, &nums.join("; "), &r.filhos[0].url);
        }
        escrever(ws, linha, 3, r.objeto_filho.as_deref().unwrap_or(""), "");
        escrever(ws, linha, 4, &r.obs, "");
        por_link(ws, col_num, linha, &r.url, true);
    }
    let n_cols = *col.last().unwrap();
    let ultima_lin = lin_cab + ficam.len() as u32;
    ajustar_layout(ws, lin_cab, ultima_lin, n_cols);
    ws.set_auto_filter(format!(
        "A{lin_cab}:{}{ultima_lin}",
        crate::planilha::letra(n_cols)
    ));
    congelar(ws, lin_cab + 1);
    if let Some(p) = destino.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(p)?;
    }
    umya_spreadsheet::writer::xlsx::write(&pl.wb, destino)
        .map_err(|e| Erro::outro(format!("não consegui gravar {}: {e}", destino.display())))?;
    Ok(ficam.len())
}

// ============================================================
// PRINCIPAL
// ============================================================

pub struct Opcoes {
    pub pasta_saida: Option<PathBuf>,
    pub abrir: Option<bool>,
    pub pasta_html: Option<PathBuf>,
    pub pausa: f64,
    pub sem_navegador: bool,
}

impl Default for Opcoes {
    fn default() -> Self {
        Opcoes {
            pasta_saida: None,
            abrir: None,
            pasta_html: None,
            pausa: v::PAUSA_REQ,
            sem_navegador: false,
        }
    }
}

/// Verifica a planilha `caminho` (aba `aba`, None = a ativa). Devolve o
/// caminho da planilha gerada, ou None se nenhuma foi gerada.
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
    if let Some(p) = &op.pasta_html {
        std::fs::create_dir_all(p)?;
    }
    if caminho.as_os_str().is_empty() || !caminho.is_file() {
        return Err(Erro::encerrar(format!(
            "✗ Arquivo não encontrado: '{}'",
            caminho.display()
        )));
    }
    let mut pl = v::ler_planilha(caminho, aba)?;
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
        "Coluna do número: {}  |  processos: {}",
        crate::planilha::letra(pl.col),
        pl.itens.len()
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
    msg!(amb, "{igual}");
    if pl.itens.is_empty() {
        return Err(Erro::encerrar("✗ Nada a verificar."));
    }
    // mesma linha repetida na planilha: consulta uma vez só
    let mut unicos: Vec<Item> = Vec::new();
    for it in &pl.itens {
        if !unicos.iter().any(|u| u.numero == it.numero) {
            unicos.push(it.clone());
        }
    }
    let t0 = Instant::now();
    let mut http = Http::nova(amb, cred, op.pausa);
    http.pasta_html = op.pasta_html.clone();
    let ctx = Ctx {
        amb,
        http,
        cache: Cache::carregar(amb),
        driver: if op.sem_navegador {
            None
        } else {
            Some(NavegadorSobDemanda::novo(cred, None))
        },
        ano_min: pl
            .itens
            .iter()
            .filter_map(|i| i.ano.parse().ok())
            .min()
            .unwrap_or_else(|| chrono::Datelike::year(&Local::now())),
        indice: Mutex::new(None),
    };
    let mut por_numero: Vec<Resultado> = Vec::new();
    let r = (|| -> Res<()> {
        ctx.http.baixar(&v::url_painel(amb), &[], false, None)?; // fixa o contexto do setor e testa os cookies
        msg!(amb, "✓ Sessão HTTP válida");
        msg!(amb);
        let n = unicos.len();
        paralelo::mapear(
            &unicos,
            v::WORKERS,
            |it| processar(&ctx, it),
            |i, r| {
                let mut icone = if r.url.is_empty() {
                    "✗".to_string()
                } else {
                    match r.veredito {
                        SIM => "✓",
                        NAO => "·",
                        _ => "?",
                    }
                    .to_string()
                };
                if r.novo.is_some() {
                    icone.push('↳');
                }
                let etapa = if r.etapa.is_empty() { "-" } else { &r.etapa };
                msg!(
                    amb,
                    "  [{:>3}/{n}] {icone:<2} {:<12} {:<13} {:<30} {}",
                    i + 1,
                    r.item.numero,
                    r.veredito,
                    corta(etapa, 30),
                    corta(&r.obs, 70)
                );
                por_numero.push(r);
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
    if por_numero.is_empty() {
        return Err(Erro::encerrar(
            "✗ Nenhum processo verificado; planilha não gerada.",
        ));
    }
    let resultados: Vec<Resultado> = pl
        .itens
        .iter()
        .filter_map(|it| {
            por_numero
                .iter()
                .find(|r| r.item.numero == it.numero)
                .map(|r| {
                    let mut x = r.clone();
                    x.item.linha = it.linha;
                    x
                })
        })
        .collect();
    let nome = caminho.file_stem().unwrap_or_default().to_string_lossy();
    let destino = pasta_saida.join(format!(
        "{nome}_verificada_{}.xlsx",
        Local::now().format("%Y-%m-%d_%H%M")
    ));
    let linhas = exportar(&mut pl, &resultados, &destino)?; // 0 = nada salvo

    let numeros = |cond: &dyn Fn(&Resultado) -> bool| {
        por_numero
            .iter()
            .filter(|r| cond(r))
            .map(|r| r.item.numero.clone())
            .collect::<Vec<_>>()
    };
    let na_planilha = numeros(&entra);
    let antes = numeros(&|r| !r.url.is_empty() && !entra(r) && r.veredito == NAO);
    let indet = numeros(&|r| !r.url.is_empty() && !entra(r) && r.veredito == INDET);
    let elab: Vec<&Resultado> = por_numero.iter().filter(|r| r.elaboracao).collect();
    let elab_com: Vec<&&Resultado> = elab.iter().filter(|r| r.novo.is_some()).collect();
    let perdidos = numeros(&|r| r.url.is_empty());
    let sem_resp: Vec<&str> = unicos
        .iter()
        .filter(|u| !por_numero.iter().any(|r| r.item.numero == u.numero))
        .map(|u| u.numero.as_str())
        .collect();
    let mut fontes: Vec<(String, usize)> = Vec::new();
    for r in &por_numero {
        if !r.fonte.is_empty() {
            match fontes.iter_mut().find(|(k, _)| *k == r.fonte) {
                Some(f) => f.1 += 1,
                None => fontes.push((r.fonte.clone(), 1)),
            }
        }
    }
    msg!(amb, "\n{igual}");
    if !elab.is_empty() {
        msg!(
            amb,
            "Em elaboração: {}  |  com novo processo emitido: {}",
            elab.len(),
            elab_com.len()
        );
        for r in &elab_com {
            let n = r.novo.as_ref().unwrap();
            let etapa = if n.etapa.is_empty() { "-" } else { &n.etapa };
            msg!(
                amb,
                "  {:<12} → {:<12} {:<13} {}  [{}]",
                r.item.numero,
                n.numero,
                n.veredito,
                corta(etapa, 30),
                n.vinculo
            );
        }
    }
    if linhas > 0 {
        let rotulo = format!(
            "Chegaram à {} ou além{}",
            v::ETAPA_CORTE,
            if INCLUIR_NOVO_ANTES_CORTE {
                " (ou com processo novo emitido)"
            } else {
                ""
            }
        );
        msg!(
            amb,
            "{rotulo}: {}  → na planilha ({linhas} linha(s))",
            na_planilha.len()
        );
        msg!(amb, "Fora da planilha:");
    } else {
        msg!(amb, "Nenhum processo chegou à {} ou além.", v::ETAPA_CORTE);
        msg!(amb, "Situação dos demais:");
    }
    msg!(amb, "  ainda antes:      {}", antes.len());
    if !indet.is_empty() {
        msg!(
            amb,
            "  indeterminados:   {}  — {}",
            indet.len(),
            indet.join(", ")
        );
    }
    if !perdidos.is_empty() {
        msg!(
            amb,
            "  não localizados:  {}  — {}",
            perdidos.len(),
            perdidos.join(", ")
        );
    }
    if !sem_resp.is_empty() {
        msg!(
            amb,
            "  não verificados:  {}  — {}",
            sem_resp.len(),
            sem_resp.join(", ")
        );
    }
    if !fontes.is_empty() {
        msg!(
            amb,
            "Localizados por: {}",
            fontes
                .iter()
                .map(|(k, v)| format!("{k} {v}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    msg!(amb, "Tempo: {:.0}s", t0.elapsed().as_secs_f64());
    if linhas > 0 {
        msg!(amb, "✓ Planilha: {}", destino.display());
        if abrir {
            amb.abrir_arquivo(&destino);
        }
    } else {
        msg!(amb, "○ Nenhuma planilha exportada.");
    }
    msg!(amb, "{igual}");
    Ok(if linhas > 0 { Some(destino) } else { None })
}
