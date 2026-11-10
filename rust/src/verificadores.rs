//! O que os dois verificadores de planilha (faltantes e verificar_planilha)
//! têm de idêntico nos scripts Python: leitura da planilha de
//! acompanhamento, camada HTTP, localização do processo (número → hash),
//! cache de hashes, origem e citações nas páginas e o ajuste de layout.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::{Map, Value, json};
use umya_spreadsheet::{Workbook as Spreadsheet, Worksheet};

use crate::ambiente::Ambiente;
use crate::erro::{Erro, Res};
use crate::http::{Repeticao, Resposta, SessaoHttp};
use crate::navegador::{self, Navegador};
use crate::sessao::{Credenciais, NavegadorSobDemanda};
use crate::texto::{ler_numero, limpo, norm};
use crate::{html, msg, re};

pub const TIMEOUT: f64 = 20.0;
pub const WORKERS: usize = 4;
pub const PAUSA_REQ: f64 = 0.3;
pub const ETAPA_CORTE: &str = "Pesquisa de Mercado";
pub const ID_DOCUMENTO: &str = "3";
pub const CABECALHOS_NUMERO: [&str; 7] = [
    "n processo",
    "no processo",
    "numero processo",
    "numero do processo",
    "processo",
    "numero",
    "n",
];
pub const ARQ_CACHE: &str = "hash_cache_1doc.json";
pub const CAIXAS_INDICE: [(&str, &str); 4] = [
    ("entrada", "Em aberto"),
    ("saida", "Caixa de saída"),
    ("arquivo", "Arquivados"),
    ("favoritos", "Favoritos"),
];
pub const RE_ETIQ_INDICE: &str = r"abertura de procedimento|fase preparatoria";

pub const SIM: &str = "Sim";
pub const NAO: &str = "Não";
pub const INDET: &str = "Indeterminado";

/// Ordem dos vereditos (`_ORDEM`).
pub fn ordem(v: &str) -> u8 {
    match v {
        SIM => 0,
        INDET => 1,
        _ => 2,
    }
}

pub fn url_painel(amb: &Ambiente) -> String {
    format!(
        "{}/?pg=painel/listar&meu=0&trocar=1&caixa=entrada",
        amb.url_1doc
    )
}

pub fn url_detalhe(amb: &Ambiente, hash: &str) -> String {
    format!(
        "{}/?pg=doc/ver&hash={hash}&itd={ID_DOCUMENTO}",
        amb.url_1doc
    )
}

pub fn hash_de(texto: &str) -> Option<String> {
    re!(r"[?&](?:amp;)?hash=([0-9A-Fa-f]{16,40})")
        .captures(texto)
        .map(|m| m[1].to_uppercase())
}

// ============================================================
// PLANILHA DE ACOMPANHAMENTO
// ============================================================

/// Um processo lido da planilha.
#[derive(Clone, Debug, Default)]
pub struct Item {
    pub linha: u32,
    pub numero: String,
    pub digitos: String,
    pub ano: String,
    pub link_planilha: Option<String>,
}

impl Item {
    pub fn do_numero(numero: &str) -> Option<Item> {
        let (n, d, a) = ler_numero(numero)?;
        Some(Item {
            numero: n,
            digitos: d,
            ano: a,
            ..Default::default()
        })
    }
}

fn valor(ws: &Worksheet, c: u32, r: u32) -> String {
    ws.cell((c, r))
        .map(|x| x.value().to_string())
        .unwrap_or_default()
}

/// Apaga as células só de formatação além da última linha e da última
/// coluna com conteúdo (planilhas formatadas "na coluna inteira" chegam com
/// células vazias até XFD/1048576). Devolve (última linha, última coluna).
pub fn limpar_excesso(ws: &mut Worksheet) -> (u32, u32) {
    let com_valor: Vec<(u32, u32)> = ws
        .cells()
        .iter()
        .filter(|c| !c.value().is_empty())
        .map(|c| (c.coordinate().row_num(), c.coordinate().col_num()))
        .collect();
    let ult_lin = com_valor.iter().map(|x| x.0).max().unwrap_or(1);
    let ult_col = com_valor.iter().map(|x| x.1).max().unwrap_or(1);
    let fora: Vec<(u32, u32)> = ws
        .cells()
        .iter()
        .map(|c| (c.coordinate().col_num(), c.coordinate().row_num()))
        .filter(|(c, r)| *r > ult_lin || *c > ult_col)
        .collect();
    for (c, r) in fora {
        ws.remove_cell((c, r));
    }
    // As colunas agrupadas já chegam uma por coluna: ficam só até a última com conteúdo.
    ws.column_dimensions_mut()
        .retain(|c| c.col_num() <= ult_col);
    ws.row_dimensions_to_hashmap_mut()
        .retain(|n, _| *n <= ult_lin);
    ws.remove_auto_filter();
    (ult_lin, ult_col)
}

/// (linha do cabeçalho, coluna) da coluna de número de processo.
pub fn achar_coluna_numero(ws: &Worksheet, max_lin: u32, max_col: u32) -> Res<(u32, u32)> {
    for r in 1..=max_lin.min(10) {
        for c in 1..=max_col {
            let v = valor(ws, c, r);
            if !v.is_empty() && CABECALHOS_NUMERO.contains(&norm(&v).as_str()) {
                return Ok((r, c));
            }
        }
    }
    // Sem cabeçalho reconhecível: coluna com mais números NN.NNN/AAAA
    let (mut melhor, mut melhor_c) = (0, None);
    for c in 1..=max_col {
        let n = (1..=max_lin)
            .filter(|r| ler_numero(&valor(ws, c, *r)).is_some())
            .count();
        if n > melhor {
            melhor = n;
            melhor_c = Some(c);
        }
    }
    let c = melhor_c.ok_or_else(|| {
        Erro::encerrar("✗ Nenhuma coluna com números de processo (NN.NNN/AAAA) na aba.")
    })?;
    let primeira = (1..=max_lin)
        .find(|r| ler_numero(&valor(ws, c, *r)).is_some())
        .unwrap_or(1);
    Ok((primeira - 1, c))
}

/// A planilha lida: pasta de trabalho, aba escolhida, cabeçalho, coluna do
/// número, processos e células sem número reconhecível.
pub struct Planilha {
    pub wb: Spreadsheet,
    pub aba: String,
    pub lin_cab: u32,
    pub col: u32,
    pub itens: Vec<Item>,
    pub ignoradas: Vec<(u32, String)>,
}

impl Planilha {
    pub fn ws(&self) -> &Worksheet {
        self.wb.sheet_by_name(&self.aba).expect("aba")
    }

    pub fn ws_mut(&mut self) -> &mut Worksheet {
        self.wb.sheet_by_name_mut(&self.aba).expect("aba")
    }
}

/// Nomes das abas (para a lista da interface).
pub fn abas(caminho: &Path) -> Res<Vec<String>> {
    let wb = abrir_pasta(caminho, true)?;
    Ok(wb
        .sheet_collection_no_check()
        .iter()
        .map(|w| w.name().to_string())
        .collect())
}

fn abrir_pasta(caminho: &Path, preguicoso: bool) -> Res<Spreadsheet> {
    let r = std::panic::catch_unwind(|| {
        if preguicoso {
            umya_spreadsheet::reader::xlsx::lazy_read(caminho)
        } else {
            umya_spreadsheet::reader::xlsx::read(caminho)
        }
    });
    match r {
        Ok(Ok(wb)) => Ok(wb),
        Ok(Err(e)) => Err(Erro::outro(format!(
            "não consegui ler {}: {e}",
            caminho.display()
        ))),
        Err(_) => Err(Erro::outro(format!(
            "não consegui ler {}: arquivo .xlsx inválido",
            caminho.display()
        ))),
    }
}

/// Lê a planilha (sem data_only: preserva fórmulas da original).
pub fn ler_planilha(caminho: &Path, aba: Option<&str>) -> Res<Planilha> {
    let mut wb = abrir_pasta(caminho, false)?;
    let nomes: Vec<String> = wb
        .sheet_collection()
        .iter()
        .map(|w| w.name().to_string())
        .collect();
    let aba = match aba.filter(|a| !a.is_empty()) {
        Some(a) if !nomes.iter().any(|n| n == a) => {
            return Err(Erro::encerrar(format!(
                "✗ Aba '{a}' não existe. Abas: {}",
                nomes.join(", ")
            )));
        }
        Some(a) => a.to_string(),
        None => wb.active_sheet().name().to_string(),
    };
    let ws = wb.sheet_by_name_mut(&aba).expect("aba");
    let (ult_lin, ult_col) = limpar_excesso(ws);
    let (lin_cab, col) = achar_coluna_numero(ws, ult_lin, ult_col)?;
    let (mut itens, mut ignoradas) = (Vec::new(), Vec::new());
    for r in lin_cab + 1..=ult_lin {
        let cel = ws.cell((col, r));
        let v = cel.map(|c| c.value().to_string()).unwrap_or_default();
        let Some((numero, digitos, ano)) = ler_numero(&v) else {
            if !v.is_empty() {
                ignoradas.push((r, v));
            }
            continue;
        };
        let link = cel.and_then(|c| c.hyperlink()).map(|h| h.url().to_string());
        itens.push(Item {
            linha: r,
            numero,
            digitos,
            ano,
            link_planilha: link,
        });
    }
    Ok(Planilha {
        wb,
        aba,
        lin_cab,
        col,
        itens,
        ignoradas,
    })
}

// ============================================================
// CAMADA HTTP
// ============================================================

const MARCAS_APP: [&str; 6] = [
    "link_emissao_a",
    "caixa_parametro",
    "navega_caixa",
    "kanban_estagios",
    "nome_em",
    "tit_assunto",
];

pub fn parece_login(fonte: &str) -> bool {
    let tem_senha = re!(r#"(?i)type\s*=\s*["']?password"#).is_match(fonte)
        || fonte.contains(r#"name="identifier""#);
    tem_senha && !MARCAS_APP.iter().any(|m| fonte.contains(m))
}

/// A sessão HTTP dos verificadores (cookies do navegador logado).
pub struct Http {
    pub amb: Ambiente,
    pub sessao: SessaoHttp,
    pub pausa: f64,
    /// --html PASTA (verificar_planilha): guarda cada página baixada para diagnóstico
    pub pasta_html: Option<PathBuf>,
}

impl Http {
    pub fn nova(amb: &Ambiente, cred: &Credenciais, pausa: f64) -> Self {
        // POST do Localizar não é repetido
        Http {
            amb: amb.clone(),
            sessao: SessaoHttp::nova(
                amb,
                &cred.cookies,
                &cred.ua,
                TIMEOUT,
                Repeticao::padrao(0.6),
            ),
            pausa,
            pasta_html: None,
        }
    }

    pub fn baixar(
        &self,
        url: &str,
        params: &[(&str, String)],
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
        let r = self.sessao.get(url, params, &cab)?.conferir()?;
        let fonte = r.texto();
        if parece_login(&fonte) {
            return Err(Erro::SessaoExpirada(format!(
                "{} devolveu a tela de login",
                url.split('?').next().unwrap_or(url)
            )));
        }
        if let Some(p) = &self.pasta_html
            && let Some(h) = hash_de(url)
        {
            let _ = std::fs::write(p.join(format!("{h}.html")), &fonte);
        }
        self.amb.esperar(self.pausa)?;
        Ok(fonte)
    }

    pub fn detalhe(&self, hash: &str) -> Res<String> {
        self.baixar(&url_detalhe(&self.amb, hash), &[], false, None)
    }
}

// ============================================================
// LOCALIZAR O PROCESSO (número → hash)
// ============================================================

/// Cache de hashes e de vínculos (hash_cache_1doc.json), compartilhado
/// pelos dois verificadores e com a versão Python.
pub struct Cache {
    caminho: PathBuf,
    pub dados: Mutex<Map<String, Value>>,
}

impl Cache {
    pub fn carregar(amb: &Ambiente) -> Self {
        let caminho = amb.arquivo_dados(ARQ_CACHE);
        let dados = match crate::arquivos::ler_json(&caminho) {
            Some(Value::Object(m)) => m,
            _ => Map::new(),
        };
        Cache {
            caminho,
            dados: Mutex::new(dados),
        }
    }

    pub fn get(&self, k: &str) -> Option<Value> {
        self.dados.lock().unwrap().get(k).cloned()
    }

    pub fn texto(&self, k: &str) -> Option<String> {
        self.get(k).and_then(|v| v.as_str().map(str::to_string))
    }

    pub fn contem(&self, k: &str) -> bool {
        self.dados.lock().unwrap().contains_key(k)
    }

    pub fn por(&self, k: &str, v: Value) {
        self.dados.lock().unwrap().insert(k.to_string(), v);
    }

    /// `json.dump(cache, indent=1, sort_keys=True, ensure_ascii=False)`.
    pub fn salvar(&self, amb: &Ambiente) {
        let ordenado: BTreeMap<String, Value> =
            self.dados.lock().unwrap().clone().into_iter().collect();
        let mut buf = Vec::new();
        let f = serde_json::ser::PrettyFormatter::with_indent(b" ");
        let mut ser = serde_json::Serializer::with_formatter(&mut buf, f);
        if serde::Serialize::serialize(&ordenado, &mut ser).is_ok()
            && let Err(e) =
                crate::arquivos::gravar_atomico(&self.caminho, &String::from_utf8_lossy(&buf))
        {
            msg!(amb, "  ⚠ cache não salvo ({e})");
        }
    }
}

/// Hashes que a resposta do Localizar pode trazer, em ordem de confiança:
/// redirecionamentos, URL final, campos JSON e, só se a resposta for curta,
/// o corpo.
fn candidatos_resposta(r: &Resposta) -> Vec<String> {
    let mut vistos: Vec<String> = Vec::new();
    let mut add = |h: Option<String>| {
        if let Some(h) = h
            && !vistos.contains(&h)
        {
            vistos.push(h);
        }
    };
    for l in &r.locations {
        add(hash_de(l));
    }
    add(hash_de(&r.url));
    let corpo = r.texto();
    match serde_json::from_str::<Value>(&corpo) {
        Ok(d) => add(hash_de(&d.to_string())),
        Err(_) if crate::texto::compr(&corpo) < 5000 => add(hash_de(&corpo)),
        Err(_) => {}
    }
    vistos
}

pub fn localizar_http(http: &Http, item: &Item) -> Res<Vec<String>> {
    http.amb.checar_cancelamento()?;
    let dados = [
        ("id_documento", ID_DOCUMENTO.to_string()),
        ("id_assunto", String::new()),
        ("num_codigo", item.digitos.clone()),
        ("ano", item.ano.clone()),
    ];
    let cab = [
        ("Referer", url_painel(&http.amb)),
        ("X-Requested-With", "XMLHttpRequest".to_string()),
    ];
    let r = http
        .sessao
        .post(
            &format!("{}/formEmissao.php?formAcessoRapido", http.amb.url_1doc),
            &dados,
            &cab,
        )?
        .conferir()?;
    http.amb.esperar(http.pausa)?;
    Ok(candidatos_resposta(&r))
}

/// Preenche o 'Localizar' do painel no navegador e lê o hash da URL.
pub fn localizar_navegador(amb: &Ambiente, nav: &mut dyn Navegador, item: &Item) -> Vec<String> {
    let achou =
        |u: &str, antes: &str| hash_de(u).is_some() || (u != antes && u.contains("pg=doc/ver"));
    match navegador::localizar_no_painel(
        nav,
        &url_painel(amb),
        ID_DOCUMENTO,
        &item.digitos,
        &item.ano,
        TIMEOUT,
        &achou,
    ) {
        Ok(Some(u)) => hash_de(&u).into_iter().collect(),
        _ => Vec::new(),
    }
}

/// 'Proc. Administrativo 31.243/2026' ou None.
pub fn ler_nome_em(fonte: &str) -> Option<String> {
    let m = re!(r#"(?i)<input\b[^>]*\bid\s*=\s*["']nome_em["'][^>]*>"#).find(fonte)?;
    re!(r#"(?i)\bvalue\s*=\s*["']([^"']*)["']"#)
        .captures(m.as_str())
        .map(|v| v[1].to_string())
}

/// Página encontrada pelo `abrir`.
pub struct Aberto {
    pub hash: String,
    pub pagina: String,
    pub fonte: String,
    pub aviso: String,
}

/// Localiza e baixa o detalhe do processo `item`: cache, link da
/// planilha, Localizar por HTTP e, por último, no navegador invisível.
/// `confere(pagina, item)`: Some(true) = é o processo; Some(false) = não é;
/// None = página sem identificação.
pub fn abrir(
    http: &Http,
    cache: &Cache,
    driver: Option<&NavegadorSobDemanda>,
    item: &Item,
    link: Option<&str>,
    confere: &dyn Fn(&str, &Item) -> Option<bool>,
) -> Res<Option<Aberto>> {
    let chave = &item.numero;
    let mut tentativas: Vec<(&str, Option<Vec<String>>)> = Vec::new();
    if let Some(h) = cache.texto(chave) {
        tentativas.push(("cache", Some(vec![h])));
    }
    if let Some(h) = link.and_then(hash_de) {
        tentativas.push(("link", Some(vec![h])));
    }
    tentativas.push(("localizar (HTTP)", None));
    tentativas.push(("localizar (navegador)", None));
    for (fonte, hashes) in tentativas {
        let hashes = match hashes {
            Some(h) => h,
            None if fonte.ends_with("(HTTP)") => match localizar_http(http, item) {
                Ok(h) => h,
                Err(e) if e.eh_http() => Vec::new(),
                Err(e) => return Err(e),
            },
            // navegador invisível aberto só agora, na 1ª vez que precisar
            None => driver
                .and_then(|d| d.com(&http.amb, |nav| localizar_navegador(&http.amb, nav, item)))
                .unwrap_or_default(),
        };
        for h in hashes {
            let pagina = match http.detalhe(&h) {
                Ok(p) => p,
                Err(e) if e.eh_http() => continue,
                Err(e) => return Err(e),
            };
            let ok = confere(&pagina, item);
            if ok == Some(false) {
                continue;
            }
            cache.por(chave, json!(h));
            let aviso = if ok.is_none() {
                "número não confirmado no detalhe; "
            } else {
                ""
            };
            return Ok(Some(Aberto {
                hash: h,
                pagina,
                fonte: fonte.to_string(),
                aviso: aviso.into(),
            }));
        }
    }
    Ok(None)
}

// ============================================================
// LEITURA DO DETALHE
// ============================================================

/// Uma etapa do quadro visível.
#[derive(Clone, Debug, Default)]
pub struct Etapa {
    pub nome: String,
    pub atual: bool,
    pub marca: String,
    pub final_: bool,
}

/// Fase de elaboração: sem quadro, ou quadro cujo fluxo não tem a etapa de corte.
pub fn sem_etapa_corte(etapas: &[Etapa]) -> bool {
    let corte = norm(ETAPA_CORTE);
    !etapas.iter().any(|e| norm(&e.nome).contains(&corte))
}

/// (hash, número) do processo de origem no cabeçalho.
pub fn ler_origem(fonte: &str) -> (Option<String>, Option<String>) {
    let Some(m) = re!(r#"(?i)class\s*=\s*["'][^"']*\bemission_infos_item_from\b"#).find(fonte)
    else {
        return (None, None);
    };
    let inicio = fonte[..m.start()].rfind('<').unwrap_or(0);
    let doc = html::doc(crate::texto::fatia(fonte, inicio, 3000));
    let a = html::um_doc(&doc, crate::sel!(".emission_infos_item_from"))
        .and_then(|h2| html::um(h2, crate::sel!("a[href]")));
    let Some(a) = a else { return (None, None) };
    let num = ler_numero(&html::texto(a, " ")).map(|n| n.0);
    (hash_de(html::attr_ou_vazio(a, "href")), num)
}

/// Candidatos citados nos despachos: (hashes de links, números).
pub fn citacoes(
    fonte: &str,
    proprio_numero: &str,
    proprio_ano: &str,
) -> (Vec<String>, Vec<String>) {
    let doc = html::doc(fonte);
    // Como o SoupStrainer(class_="texto_original") do bs4 4.13+: só casa o
    // atributo class que é exatamente "texto_original" (com outra classe
    // junto, o bloco fica de fora na versão Python; aqui também, pela paridade).
    let so_ela = |e: scraper::ElementRef| {
        html::attr(e, "class").is_some_and(|c| limpo(c) == "texto_original")
    };
    let blocos: Vec<scraper::ElementRef> = doc
        .select(crate::sel!(".texto_original"))
        .filter(|e| so_ela(*e) && html::ancestral(*e, so_ela).is_none())
        .collect();
    let (mut hashes, mut numeros): (Vec<String>, Vec<String>) = (Vec::new(), Vec::new());
    let ano_min: i64 = proprio_ano.parse().unwrap_or(0);
    for b in &blocos {
        for a in b.select(crate::sel!("a[href]")) {
            if let Some(h) = hash_de(html::attr_ou_vazio(a, "href"))
                && !hashes.contains(&h)
            {
                hashes.push(h);
            }
        }
    }
    let texto = blocos
        .iter()
        .map(|b| html::texto(*b, " "))
        .collect::<Vec<_>>()
        .join(" ");
    for m in re!(r"(\d{1,3}(?:\.\d{3})+|\d+)\s*/\s*(\d{4})").find_iter(&texto) {
        let Some(num) = ler_numero(m.as_str()) else {
            continue;
        };
        if num.0 == proprio_numero || num.2.parse::<i64>().unwrap_or(0) < ano_min {
            continue;
        }
        let antes_txt: String = {
            let anteriores: Vec<char> = texto[..m.start()].chars().collect();
            anteriores[anteriores.len().saturating_sub(45)..]
                .iter()
                .collect()
        };
        let antes = norm(&antes_txt);
        let fim: String = {
            let c: Vec<char> = antes.chars().collect();
            c[c.len().saturating_sub(30)..].iter().collect()
        };
        if !re!(r"(proc|abertura|emitid|gerad|autuad|instaurad|\bno?\b|numero)").is_match(&antes)
            || re!(r"(lei|decreto|portaria|requisi|\brc\b|contrato|\bata\b|edital|pregao|oficio|memorando|resolucao|instrucao|acordao|cnpj|cpf)").is_match(&fim)
        {
            continue;
        }
        if !numeros.contains(&num.0) {
            numeros.push(num.0);
        }
    }
    (hashes, numeros)
}

/// A listagem vem como HTML ou embrulhada em JSON; neste caso junta as
/// strings que contêm marcação.
pub fn html_listagem(texto: &str) -> String {
    let ini = texto.trim_start();
    if !(ini.starts_with('{') || ini.starts_with('[')) {
        return texto.to_string();
    }
    let Ok(dados) = serde_json::from_str::<Value>(texto) else {
        return texto.to_string();
    };
    fn coletar(x: &Value, p: &mut Vec<String>) {
        match x {
            Value::String(s) if s.contains('<') => p.push(s.clone()),
            Value::Object(o) => o.values().for_each(|v| coletar(v, p)),
            Value::Array(a) => a.iter().for_each(|v| coletar(v, p)),
            _ => {}
        }
    }
    let mut p = Vec::new();
    coletar(&dados, &mut p);
    p.join("\n")
}

pub fn total_paginas(fonte: &str) -> Option<i64> {
    re!(r"P(?:á|&aacute;|a)gina\s*(\d+)\s*/\s*(\d+)")
        .captures(fonte)
        .and_then(|m| m[2].parse().ok())
}

// ============================================================
// LAYOUT (largura das colunas e altura das linhas)
// ============================================================

// Medidas do Excel: largura em "caracteres do 0" da fonte padrão (Calibri
// 11); altura em pontos, ~15 pt por linha de texto em 11 pt.
pub const LARG_MIN: f64 = 8.0;
pub const LARG_MAX: f64 = 60.0;
pub const PT_POR_LINHA: f64 = 15.0;

/// Comprimento visual em "caracteres do 0" (Calibri).
pub fn compr(s: &str) -> f64 {
    s.chars()
        .map(|ch| {
            if "MWmw@%".contains(ch) {
                1.35
            } else if ch.is_uppercase() {
                1.08
            } else if ch == ' ' {
                0.45
            } else if ".,;:!|'ilj1()/-".contains(ch) {
                0.55
            } else if ch.is_numeric() {
                1.0
            } else {
                0.88
            }
        })
        .sum()
}

/// Quantas linhas o texto ocupa numa coluna de `largura`, quebrando por palavra.
pub fn linhas_quebradas(texto: &str, largura: f64, fator: f64) -> f64 {
    let util = (largura - 1.5).max(1.0) / fator;
    let mut total = 0.0;
    for paragrafo in texto.split('\n') {
        let (mut linhas, mut atual) = (1.0, 0.0);
        for palavra in paragrafo.split_whitespace() {
            let n = compr(palavra);
            if atual == 0.0 {
                atual = n;
            } else if atual + 0.45 + n <= util {
                atual += 0.45 + n;
            } else {
                linhas += 1.0;
                atual = n;
            }
            while atual > util {
                linhas += 1.0;
                atual -= util;
            }
        }
        total += linhas;
    }
    total
}

/// `round(x, 1)`.
pub fn arred1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

/// O que o layout precisa de cada célula: texto e fonte.
pub struct CelulaLayout {
    pub texto: String,
    pub tamanho: f64,
    pub negrito: bool,
}

impl CelulaLayout {
    pub fn fator(&self) -> f64 {
        (self.tamanho / 11.0) * if self.negrito { 1.1 } else { 1.0 }
    }
}

/// Largura de cada coluna pelo conteúdo (com teto) e altura de cada linha
/// pelo texto mais alto (o `ajustar_layout`). `celula(linha, coluna)`.
pub fn calcular_layout(
    lin_cab: u32,
    ultima_lin: u32,
    n_cols: u32,
    celula: &dyn Fn(u32, u32) -> CelulaLayout,
) -> (Vec<f64>, Vec<(u32, f64)>) {
    let mut larguras = vec![0.0; n_cols as usize + 1];
    for c in 1..=n_cols {
        let mut dados: f64 = 0.0;
        for r in lin_cab + 1..=ultima_lin {
            let cel = celula(r, c);
            if !cel.texto.is_empty() {
                let maior = cel.texto.split('\n').map(compr).fold(0.0, f64::max);
                dados = dados.max(maior * cel.fator());
            }
        }
        let cab = celula(lin_cab, c);
        let palavra_cab = cab.texto.split_whitespace().map(compr).fold(0.0, f64::max) * cab.fator();
        let cab_inteiro = compr(&cab.texto) * cab.fator();
        let alvo = dados.max(palavra_cab).max(cab_inteiro.min(dados.max(14.0))) + 2.5; // + seta do filtro
        larguras[c as usize] = arred1(alvo.clamp(LARG_MIN, LARG_MAX));
    }
    let mut alturas = Vec::new();
    for r in lin_cab..=ultima_lin {
        let mut maior: f64 = 1.0;
        for c in 1..=n_cols {
            let cel = celula(r, c);
            if cel.texto.is_empty() {
                continue;
            }
            let n = linhas_quebradas(&cel.texto, larguras[c as usize], cel.fator());
            maior = maior.max(n * (cel.tamanho / 11.0));
        }
        alturas.push((r, arred1(maior * PT_POR_LINHA + 3.0)));
    }
    (larguras, alturas)
}

/// Texto de uma célula como aparece (`_texto_celula`).
pub fn texto_numero(v: f64) -> String {
    // f"{v:,.2f}"
    let s = format!("{:.2}", v.abs());
    let (int, dec) = s.split_once('.').unwrap_or((&s, "00"));
    let mut com = String::new();
    for (i, ch) in int.chars().enumerate() {
        if i > 0 && (int.len() - i).is_multiple_of(3) {
            com.push(',');
        }
        com.push(ch);
    }
    format!("{}{com}.{dec}", if v < 0.0 { "-" } else { "" })
}

pub fn sem_espacos(s: &str) -> String {
    limpo(s)
}
