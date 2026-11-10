//! Busca de processos no 1Doc por etapa, fase, datas e momento de mudança
//! de etapa (porta de busca.py). SOMENTE LEITURA: só consulta, como a
//! interface (listagem das caixas, abertura do processo, localizar por
//! número). Única alteração de estado possível: se a sessão estiver no
//! "Inbox pessoal", troca para o "Inbox de <setor>" (o mesmo link do menu).
//!
//! 1. Sessão — reaproveita os cookies salvos; se expiraram, abre o navegador
//!    e espera o login (crate::sessao).
//! 2. Listagem — lê as caixas do setor via link.php?chamada=caixa.
//! 3. Detalhe — abre cada processo que passou nos filtros baratos e lê o
//!    quadro de etapas e a linha do tempo.
//! 4. Mudança de etapa — eventos "Mudança de etapa" da linha do tempo; sem
//!    eles, o rótulo dos despachos "via etapa", conferido contra o quadro.
//! 5. Filtros e saída — .xlsx com abas Resultado, Transições e Parâmetros
//!    (CSV opcional) e um resumo.

pub mod args;
pub mod parse;
pub mod registro;
pub mod saida;

use std::path::{Path, PathBuf};

use chrono::NaiveDateTime;
use serde_json::{Map, Value, json};

use crate::ambiente::Ambiente;
use crate::erro::{Erro, Res};
use crate::http::{Repeticao, SessaoHttp};
use crate::sessao::{self, Credenciais};
use crate::texto::corta;
use crate::{arquivos, config, html, msg, msg_sem_quebra};
pub use args::{Analise, Args, Opcoes};
use parse::{VERSAO, de_iso, iso};
pub use registro::Registro;
use registro::{Filtro, avaliar, consolidar, montar_filtros, ordenar};
pub use saida::Falha;

pub const CAIXAS: [(&str, &str); 4] = [
    ("entrada", "Em aberto"),
    ("saida", "Caixa de saída"),
    ("arquivo", "Arquivados"),
    ("favoritos", "Favoritos"),
];

fn rotulo_caixa(cx: &str) -> String {
    CAIXAS
        .iter()
        .find(|(k, _)| *k == cx)
        .map(|(_, v)| v.to_string())
        .unwrap_or_else(|| cx.to_string())
}

// =============================================================================
// Sessão e cliente (somente leitura)
// =============================================================================

pub struct Sessao1Doc {
    pub base: String,
    http: SessaoHttp,
    pausa: f64,
    pub id_setor: String,
    amb: Ambiente,
}

impl Sessao1Doc {
    pub fn nova(amb: &Ambiente, cred: &Credenciais, pausa: f64) -> Self {
        let rep = Repeticao {
            post_tambem: true,
            ..Repeticao::padrao(0.8)
        };
        Sessao1Doc {
            base: amb.base(),
            http: SessaoHttp::nova(amb, &cred.cookies, &cred.ua, 40.0, rep),
            pausa,
            id_setor: String::new(),
            amb: amb.clone(),
        }
    }

    fn texto(&self, r: crate::http::Resposta) -> Res<String> {
        let r = r.conferir()?;
        self.amb.esperar(self.pausa)?;
        Ok(r.texto_utf8())
    }

    pub fn get(&self, caminho: &str, params: &[(&str, String)]) -> Res<String> {
        self.amb.checar_cancelamento()?;
        let r = self
            .http
            .get(&format!("{}{caminho}", self.base), params, &[])?;
        self.texto(r)
    }

    pub fn post(&self, caminho: &str, dados: &[(&str, String)]) -> Res<String> {
        self.amb.checar_cancelamento()?;
        let r = self
            .http
            .post(&format!("{}{caminho}", self.base), dados, &[])?;
        self.texto(r)
    }
}

/// Nome do setor ativo se a sessão vale; None se caiu no login. Grava em
/// `id_setor` o id do setor (input#s_is).
pub fn verificar_sessao(sess: &mut Sessao1Doc, tentou_inbox: bool) -> Res<Option<String>> {
    let p = [("pg", "painel/listar".to_string())];
    let fonte = match sess.get("", &p) {
        Ok(f) => f,
        Err(e) if e.eh_http() => return Ok(None),
        Err(e) => return Err(e),
    };
    let doc = html::doc(&fonte);
    let email = html::um_doc(&doc, sel!("input#sis_usr_email"))
        .map(|e| html::attr_ou_vazio(e, "value").trim().to_string());
    if email.unwrap_or_default().is_empty() {
        return Ok(None);
    }
    let pessoal = html::um_doc(&doc, sel!("li.li_pessoal.active")).is_some();
    if pessoal && !tentou_inbox {
        msg!(
            sess.amb,
            "  Inbox pessoal estava ativo; trocando para o inbox do setor."
        );
        let p = [
            ("pg", "painel/listar".to_string()),
            ("meu", "0".into()),
            ("trocar", "1".into()),
        ];
        match sess.get("", &p) {
            Ok(_) => {}
            Err(e) if e.eh_http() => return Ok(None),
            Err(e) => return Err(e),
        }
        return verificar_sessao(sess, true);
    }
    if pessoal {
        msg!(
            sess.amb,
            "  Aviso: não foi possível ativar o inbox do setor; a varredura pode ser só do inbox pessoal."
        );
    }
    if let Some(fl) = html::um_doc(&doc, sel!("input#enableFacelift"))
        && html::tem_attr(fl, "checked")
    {
        msg!(
            sess.amb,
            "  Aviso: o 'novo visual da 1Doc' está ativo nesta conta. Os seletores foram calibrados no visual clássico; desative-o se a coleta vier vazia ou com lacunas."
        );
    }
    sess.id_setor = html::um_doc(&doc, sel!("input#s_is"))
        .map(|e| html::attr_ou_vazio(e, "value").trim().to_string())
        .unwrap_or_default();
    let setor = html::um_doc(&doc, sel!("input#s_s"))
        .map(|e| html::attr_ou_vazio(e, "value").to_string())
        .unwrap_or_default();
    Ok(Some(if setor.is_empty() {
        "(setor não identificado)".into()
    } else {
        setor
    }))
}

/// Sessão salva, se ainda vale; senão o navegador abre e o programa espera
/// o login. `cred` = credenciais já obtidas pela interface.
pub fn abrir_sessao(
    amb: &Ambiente,
    a: &Args,
    cred: Option<&Credenciais>,
) -> Res<(Sessao1Doc, String)> {
    let cred = match cred {
        Some(c) => c.clone(),
        None => {
            let minutos = a
                .timeout_login
                .filter(|t| *t != 0)
                .map(|t| ((t as f64 / 60.0).round()).max(1.0));
            sessao::obter_sessao(amb, a.relogar, a.navegador.as_deref(), minutos, None)?
        }
    };
    let mut sess = Sessao1Doc::nova(amb, &cred, a.pausa);
    match verificar_sessao(&mut sess, false)? {
        Some(setor) => Ok((sess, setor)),
        None => {
            sessao::esquecer(amb);
            Err(Erro::encerrar(
                "O 1Doc recusou a sessão nas requisições. Execute de novo: o navegador vai abrir para o login.",
            ))
        }
    }
}

pub struct Cliente1Doc {
    pub sess: Sessao1Doc,
}

impl Cliente1Doc {
    pub fn pagina_caixa(
        &self,
        caixa: &str,
        num: i64,
        de: Option<NaiveDateTime>,
        ate: Option<NaiveDateTime>,
    ) -> Res<String> {
        let mut p = vec![
            ("chamada", "caixa".to_string()),
            ("pg", "painel/listar".into()),
            ("caixa", caixa.into()),
            ("num_pagina", num.to_string()),
            ("ordenar", "1".into()),
            ("ordem", "desc".into()),
            ("modo_visualizacao", "1".into()),
            ("emissoes_arquivado_prazos", "0".into()),
        ];
        if let Some(de) = de {
            // só "até": sem intervalo no servidor; o corte é feito linha a linha
            let ate = ate.unwrap_or_else(parse::agora_br);
            p.push((
                "intervalo",
                format!("{} - {}", de.format("%d/%m/%Y"), ate.format("%d/%m/%Y")),
            ));
        }
        self.sess.get("link.php", &p)
    }

    pub fn processo_por_hash(&self, hash: &str, itd: &str) -> Res<String> {
        self.sess.get(
            "",
            &[
                ("pg", "doc/ver".into()),
                ("itd", itd.into()),
                ("hash", hash.into()),
            ],
        )
    }

    pub fn processo_por_numero(&self, numero: &str, itd: &str) -> Res<String> {
        let (num, ano) = numero.split_once('/').unwrap_or((numero, ""));
        self.sess.post(
            "formEmissao.php?formAcessoRapido",
            &[
                ("id_documento", itd.into()),
                ("id_assunto", String::new()),
                ("num_codigo", num.into()),
                ("ano", ano.into()),
            ],
        )
    }
}

// =============================================================================
// Coleta (com cache)
// =============================================================================

pub fn chave(numero: &str, itd: &str) -> String {
    format!("{itd}|{numero}")
}

fn cache_vazio() -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("versao".into(), json!(VERSAO));
    m.insert("lista".into(), json!({}));
    m.insert("det".into(), json!({}));
    m.insert("posicao_lista".into(), json!(""));
    m
}

pub fn carregar_cache(amb: &Ambiente, caminho: &Path) -> Map<String, Value> {
    let vazio = cache_vazio();
    if !caminho.exists() {
        return vazio;
    }
    let Some(Value::Object(c)) = std::fs::read(caminho)
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
    else {
        msg!(
            amb,
            "Aviso: cache {} ilegível; ignorado.",
            caminho.display()
        );
        return vazio;
    };
    if c.get("versao").and_then(Value::as_str) != Some(VERSAO) {
        let v = c
            .get("versao")
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
            .unwrap_or("?");
        msg!(
            amb,
            "Aviso: cache de versão {v} (atual {VERSAO}); será refeito."
        );
        return vazio;
    }
    let mut m = vazio;
    for (k, v) in c {
        m.insert(k, v);
    }
    m
}

fn salvar_cache(caminho: &Path, cache: &Map<String, Value>) -> Res<()> {
    arquivos::gravar_atomico(caminho, &serde_json::to_string(cache)?)?;
    Ok(())
}

fn obj_mut<'a>(cache: &'a mut Map<String, Value>, k: &str) -> &'a mut Map<String, Value> {
    if !cache.get(k).is_some_and(Value::is_object) {
        cache.insert(k.into(), json!({}));
    }
    cache.get_mut(k).and_then(Value::as_object_mut).unwrap()
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

/// (lista, contagem por caixa, avisos de completude).
type Listagem = (Map<String, Value>, Vec<(String, i64)>, Vec<String>);

fn listar(
    amb: &Ambiente,
    cli: &Cliente1Doc,
    caixas: &[String],
    itds: &[String],
    op: &Opcoes,
) -> Res<Listagem> {
    let (de, ate, max_paginas) = (op.emissao_de, op.emissao_ate, op.a.max_paginas);
    let (mut lista, mut por_caixa, mut avisos) = (Map::new(), Vec::new(), Vec::new());
    for cx in caixas {
        let rot = rotulo_caixa(cx);
        let (mut n, mut primeira, mut tamanho): (i64, Option<String>, Option<usize>) =
            (0, None, None);
        let (mut meta, mut lidos, mut ultima) = ((None, None, None), 0usize, 0i64);
        for num in 1..=max_paginas.max(0) {
            let fonte = cli.pagina_caixa(cx, num, de, ate)?;
            let doc = html::doc(&fonte);
            let itens = parse::parse_listagem(&doc, &rot, itds, &cli.sess.base);
            if num == 1
                && itens.is_empty()
                && crate::re!(r#"(?i)name=["']?senha|formLogin|login\.php"#)
                    .is_match(parse::fatia_bytes(&fonte, 20000))
            {
                return Err(Erro::SessaoExpirada(String::new()));
            }
            let brutos = doc.select(sel!("a.link_emissao_a")).count();
            if num == 1 {
                meta = parse::meta_listagem(&doc, &fonte, cx);
                if brutos == 0 && meta.2.unwrap_or(0) > 0 {
                    avisos.push(format!(
                        "{rot}: a caixa informa {} itens, mas nenhuma linha foi reconhecida (layout mudou ou o 'novo visual' está ativo?)",
                        meta.2.unwrap_or(0)
                    ));
                }
            }
            if brutos == 0 {
                break;
            }
            lidos += brutos;
            ultima = num;
            let primeiro_num = itens.first().map(|i| s(i, "numero"));
            if primeiro_num.is_some() && primeiro_num == primeira {
                break; // servidor repetiu a última página
            }
            if primeiro_num.is_some() {
                primeira = primeiro_num;
            }
            tamanho = tamanho.or(Some(brutos));
            for it in itens {
                let em = de_iso(&s(&it, "emissao"));
                if let Some(em) = em
                    && (de.is_some_and(|d| em < d) || ate.is_some_and(|a| em > a))
                {
                    continue;
                }
                n += 1;
                let k = chave(&s(&it, "numero"), &s(&it, "itd"));
                match lista.get_mut(&k) {
                    Some(v) => {
                        let atual = s(v, "caixa");
                        if !atual.contains(&rot) {
                            v["caixa"] = json!(format!("{atual} | {rot}"));
                        }
                    }
                    None => {
                        lista.insert(k, it);
                    }
                }
            }
            let tot_pg = meta.1;
            msg_sem_quebra!(
                amb,
                "\r  {rot}: página {num}{}, {n} processos",
                tot_pg.map(|t| format!("/{t}")).unwrap_or_default()
            );
            if tot_pg.is_some_and(|t| t > 0 && num >= t)
                || (tot_pg.is_none_or(|t| t == 0) && Some(brutos) < tamanho)
            {
                break;
            }
        }
        msg!(amb);
        por_caixa.push((rot.clone(), n));
        // Completude: varredura que parou antes do fim é declarada
        let (tot_pg, total) = (meta.1.filter(|t| *t > 0), meta.2);
        if let Some(tp) = tot_pg
            && ultima < tp
        {
            avisos.push(format!(
                "{rot}: lidas {ultima} de {tp} páginas{}",
                if ultima >= max_paginas {
                    " (limite --max-paginas)"
                } else {
                    ""
                }
            ));
        } else if let Some(t) = total
            && de.is_none()
            && lidos as i64 != t
        {
            avisos.push(format!(
                "{rot}: a caixa informa {t} itens e foram lidas {lidos} linhas (processos movimentados durante a leitura mudam de página)"
            ));
        }
    }
    Ok((lista, por_caixa, avisos))
}

fn detalhar_um(
    cli: &Cliente1Doc,
    numero: &str,
    itd: &str,
    item: Option<&Value>,
    agora: NaiveDateTime,
) -> Res<(Option<Value>, Option<Falha>)> {
    let h = item.map(|i| s(i, "hash")).unwrap_or_default();
    let pagina = if h.is_empty() {
        cli.processo_por_numero(numero, itd)?
    } else {
        cli.processo_por_hash(&h, itd)?
    };
    let mut det = parse::parse_detalhe(&pagina, agora)?;
    if s(&det, "numero") != numero && !h.is_empty() {
        // hash velho: tenta pelo número
        det = parse::parse_detalhe(&cli.processo_por_numero(numero, itd)?, agora)?;
    }
    // O número devolvido TEM de ser o pedido: lacuna declarada é melhor que valor errado.
    if s(&det, "numero") != numero {
        let obtido = s(&det, "numero");
        return Ok((
            None,
            Some(Falha {
                numero: numero.into(),
                motivo: "número divergente".into(),
                obtido,
            }),
        ));
    }
    if s(&det, "etapa").is_empty() {
        return Ok((
            None,
            Some(Falha {
                numero: numero.into(),
                motivo: "sem etapa nem rótulo utilizável".into(),
                obtido: String::new(),
            }),
        ));
    }
    let hash = Some(s(&det, "hash")).filter(|x| !x.is_empty()).unwrap_or(h);
    let o = det.as_object_mut().unwrap();
    o.insert("itd".into(), json!(itd));
    o.insert(
        "url".into(),
        json!(if hash.is_empty() {
            String::new()
        } else {
            format!("{}?pg=doc/ver&hash={hash}&itd={itd}", cli.sess.base)
        }),
    );
    o.insert(
        "_ult_mov_lista".into(),
        json!(item.map(|i| s(i, "ult_mov")).unwrap_or_default()),
    );
    Ok((Some(det), None))
}

type Pedido = (String, String, Option<Value>);

/// Preenche `dets` à medida que avança (se a sessão cair, o parcial não se perde).
fn detalhar(
    amb: &Ambiente,
    cli: &Cliente1Doc,
    mut pedidos: Vec<Pedido>,
    agora: NaiveDateTime,
    paralelo: i64,
    dets: &mut Map<String, Value>,
) -> Res<Vec<Falha>> {
    let (mut falhas, mut feitos) = (Vec::new(), 0usize);
    let total = pedidos.len();
    for rodada in [1, 2] {
        // falha é "refazer", não "não existe": uma segunda tentativa
        if pedidos.is_empty() {
            break;
        }
        let mut falhas_rodada: Vec<(Pedido, Falha)> = Vec::new();
        let tarefa = |p: &Pedido| -> Res<(Option<Value>, Option<Falha>)> {
            match detalhar_um(cli, &p.0, &p.1, p.2.as_ref(), agora) {
                Ok(r) => Ok(r),
                Err(e) if e.atravessa() || matches!(e, Erro::SessaoExpirada(_)) => Err(e),
                // rede, HTML inesperado
                Err(e) => Ok((
                    None,
                    Some(Falha {
                        numero: p.0.clone(),
                        motivo: "exceção".into(),
                        obtido: corta(&e.to_string(), 120),
                    }),
                )),
            }
        };
        crate::paralelo::conforme_terminam(
            &pedidos,
            paralelo.max(1) as usize,
            tarefa,
            |i, (det, falha)| {
                let p = &pedidos[i];
                if let Some(d) = det {
                    dets.insert(chave(&p.0, &p.1), d);
                    feitos += 1;
                } else if let Some(f) = falha {
                    falhas_rodada.push((p.clone(), f));
                }
                msg_sem_quebra!(
                    amb,
                    "\r  detalhando {feitos}/{total}{}",
                    if rodada == 2 { " (2ª tentativa)" } else { "" }
                );
                Ok(())
            },
        )?;
        pedidos = falhas_rodada.iter().map(|(p, _)| p.clone()).collect();
        falhas = falhas_rodada.into_iter().map(|(_, f)| f).collect();
    }
    if total > 0 {
        msg!(amb);
    }
    Ok(falhas)
}

// =============================================================================
// Orquestração
// =============================================================================

pub struct Resultado {
    pub regs: Vec<Registro>,
    pub falhas: Vec<Falha>,
    pub params: Vec<(String, String)>,
    pub filtros: Vec<Filtro>,
    pub arquivo: Option<PathBuf>,
}

fn caminho_cache(amb: &Ambiente, a: &Args) -> PathBuf {
    match a.cache.as_deref().filter(|c| !c.is_empty()) {
        Some(c) => PathBuf::from(c),
        None => amb.arquivo_dados("1doc_cache.json"),
    }
}

pub fn executar(
    amb: &Ambiente,
    op: &mut Opcoes,
    cli: Option<&Cliente1Doc>,
    setor: &str,
    agora: NaiveDateTime,
) -> Res<Resultado> {
    let a = op.a.clone();
    let mut itds_ordem = args::lista(std::slice::from_ref(&a.itd));
    if itds_ordem.is_empty() {
        itds_ordem = vec!["3".into()];
    }
    let itds = itds_ordem.clone();
    let arq_cache = caminho_cache(amb, &a);
    let mut cache = if a.sem_cache {
        cache_vazio()
    } else {
        carregar_cache(amb, &arq_cache)
    };
    let det_cache_vazio = cache
        .get("det")
        .and_then(Value::as_object)
        .is_none_or(|d| d.is_empty());
    if a.offline && det_cache_vazio {
        return Err(Erro::encerrar(
            "--offline: cache vazio ou de versão anterior; rode uma vez com acesso ao 1Doc.",
        ));
    }
    let caixas: Vec<String> = if a.caixas == "todas" {
        CAIXAS.iter().map(|c| c.0.to_string()).collect()
    } else {
        args::lista(std::slice::from_ref(&a.caixas))
    };
    for cx in &caixas {
        if !CAIXAS.iter().any(|c| c.0 == cx) {
            return Err(Erro::encerrar(format!(
                "Caixa desconhecida: {cx} (use {} ou todas)",
                CAIXAS.iter().map(|c| c.0).collect::<Vec<_>>().join(", ")
            )));
        }
    }

    // ---- universo ----
    let mut numeros: Vec<String> = args::lista(&a.numeros)
        .iter()
        .filter_map(|x| parse::norm_numero(x))
        .collect();
    for arq in &a.numeros_arquivo {
        for n in parse::numeros_de_arquivo(arq)? {
            if !numeros.contains(&n) {
                numeros.push(n);
            }
        }
    }
    let mut lista_fresca = false;
    let universo;
    let mut lista: Map<String, Value> = Map::new();
    if !numeros.is_empty() {
        // Da listagem antiga só aproveita o hash (atalho para abrir a página).
        let itd0 = &itds_ordem[0];
        let lista_cache = cache.get("lista").cloned().unwrap_or(json!({}));
        for n in &numeros {
            let k = chave(n, itd0);
            let h = lista_cache
                .get(&k)
                .map(|v| s(v, "hash"))
                .unwrap_or_default();
            lista.insert(
                k,
                if h.is_empty() {
                    Value::Null
                } else {
                    json!({"hash": h})
                },
            );
        }
        universo = format!("{} número(s) informado(s)", numeros.len());
    } else if a.offline {
        if let Some(Value::Object(l)) = cache.get("lista") {
            for (k, v) in l {
                if itds.is_empty() || itds.contains(&s(v, "itd")) {
                    lista.insert(k.clone(), v.clone());
                }
            }
        }
        let pos = cache
            .get("posicao_lista")
            .and_then(Value::as_str)
            .filter(|p| !p.is_empty())
            .unwrap_or("sem data")
            .to_string();
        universo = format!("listagem do cache ({pos})");
    } else {
        let cli = cli.ok_or_else(|| Erro::outro("sem sessão do 1Doc"))?;
        msg!(
            amb,
            "Lendo caixas: {}",
            caixas
                .iter()
                .map(|c| rotulo_caixa(c))
                .collect::<Vec<_>>()
                .join(", ")
        );
        let (l, por_caixa, avisos) = listar(amb, cli, &caixas, &itds, op)?;
        for av in &avisos {
            msg!(amb, "  Aviso: {av}");
        }
        lista = l;
        cache.insert("lista".into(), Value::Object(lista.clone()));
        cache.insert("posicao_lista".into(), json!(iso(Some(agora))));
        let mut u = format!(
            "caixas {}",
            por_caixa
                .iter()
                .map(|(k, v)| format!("{k} ({v})"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        if !avisos.is_empty() {
            u.push_str(&format!("\nAVISOS: {}", avisos.join("; ")));
        }
        universo = u;
        lista_fresca = true;
    }

    // id do setor logado: da sessão; no --offline, o da última coleta
    if let Some(c) = cli
        && !c.sess.id_setor.is_empty()
    {
        cache.insert("id_setor".into(), json!(c.sess.id_setor));
    }
    op.id_setor = cache
        .get("id_setor")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if a.etapa_do_meu_setor && op.id_setor.is_empty() {
        return Err(Erro::encerrar(
            "--etapa-do-meu-setor: id do setor não identificado na sessão (input#s_is).",
        ));
    }
    let filtros = montar_filtros(op);

    // ---- pré-filtro com o que a listagem já traz (barato) ----
    let mut candidatos = Vec::new();
    for (k, item) in &lista {
        if item.is_object() && !s(item, "numero").is_empty() {
            let mut r0 = consolidar(Some(item), None, agora, op);
            if !avaliar(&mut r0, &filtros, false, true) {
                continue;
            }
        }
        candidatos.push(k.clone());
    }

    // ---- detalhe (com reaproveitamento do cache) ----
    let precisa = filtros.iter().any(|f| f.fonte == "detalhe");
    if a.sem_detalhe && precisa {
        return Err(Erro::encerrar(
            "Os filtros pedidos exigem abrir os processos; retire --sem-detalhe.",
        ));
    }
    let (mut dets, mut falhas, mut do_cache) = (Map::new(), Vec::new(), 0usize);
    if !a.sem_detalhe {
        let mut pedidos: Vec<Pedido> = Vec::new();
        let det_cache = cache
            .get("det")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        for k in &candidatos {
            let item = lista.get(k).filter(|v| !v.is_null());
            let c = det_cache.get(k).filter(|c| arquivos::verdadeiro(c));
            // Reaproveita só se a listagem é de agora e a última movimentação não mudou.
            let ult = item.map(|i| s(i, "ult_mov")).unwrap_or_default();
            let reaproveita = c.is_some()
                && !a.refazer
                && (a.offline
                    || (lista_fresca
                        && item.is_some()
                        && !ult.is_empty()
                        && c.map(|c| s(c, "_ult_mov_lista")) == Some(ult)));
            if reaproveita {
                dets.insert(k.clone(), c.unwrap().clone());
                do_cache += 1;
            } else if a.offline {
                let numero = k.split_once('|').map(|x| x.1).unwrap_or(k).to_string();
                falhas.push(Falha {
                    numero,
                    motivo: "sem detalhe no cache (modo offline)".into(),
                    obtido: String::new(),
                });
            } else {
                let (itd, num) = k.split_once('|').unwrap_or(("", k));
                pedidos.push((num.to_string(), itd.to_string(), item.cloned()));
            }
        }
        if !pedidos.is_empty() {
            let cli = cli.ok_or_else(|| Erro::outro("sem sessão do 1Doc"))?;
            let est = pedidos.len() as f64 * 0.8 / a.paralelo.max(1) as f64;
            msg!(
                amb,
                "Abrindo {} processo(s) (~{:.0} s); {do_cache} reaproveitado(s) do cache.",
                pedidos.len(),
                est
            );
            let mut novos = Map::new();
            let r = detalhar(amb, cli, pedidos, agora, a.paralelo, &mut novos);
            // salva o parcial mesmo se a sessão cair no meio
            for (k, v) in &novos {
                dets.insert(k.clone(), v.clone());
            }
            let dc = obj_mut(&mut cache, "det");
            for (k, v) in novos {
                dc.insert(k, v);
            }
            if !a.sem_cache {
                salvar_cache(&arq_cache, &cache)?;
            }
            falhas.extend(r?);
        }
    }

    // ---- filtros finais ----
    let excecao = a.excecao_cita_setor.is_some();
    let mut regs = Vec::new();
    for k in &candidatos {
        if !a.sem_detalhe && !dets.contains_key(k) {
            continue; // falha: fica declarada, não vira linha com chute
        }
        let mut r = consolidar(lista.get(k), dets.get(k), agora, op);
        if !r.numero.is_empty() && avaliar(&mut r, &filtros, excecao, false) {
            regs.push(r);
        }
    }
    let regs = ordenar(regs, &a.ordenar);

    if !a.sem_cache {
        salvar_cache(&arq_cache, &cache)?;
    }

    let excecao_txt = if excecao {
        format!(
            "processos reprovados só nos filtros de posição entram se os últimos {} despacho(s) citarem {}{}",
            a.ultimos_despachos,
            a.excecao_cita_setor.clone().unwrap_or_default(),
            if a.pessoa.is_empty() {
                String::new()
            } else {
                format!(" ou {}", a.pessoa.join(", "))
            }
        )
    } else {
        "(nenhuma)".into()
    };
    let filtros_txt = filtros
        .iter()
        .map(|f| format!("• {}", f.descricao))
        .collect::<Vec<_>>()
        .join("\n");
    let params: Vec<(String, String)> = vec![
        ("Posição".into(), agora.format("%d/%m/%Y %H:%M").to_string()),
        ("Sessão / setor".into(), if setor.is_empty() { "(offline)".into() } else { setor.to_string() }),
        ("Escopo".into(), "Somente as caixas do setor do usuário logado — não é a Prefeitura inteira.".into()),
        ("Universo".into(), universo),
        ("Filtros".into(), if filtros_txt.is_empty() { "(nenhum)".into() } else { filtros_txt }),
        ("Exceção".into(), excecao_txt),
        (
            "Contagens".into(),
            format!(
                "listados {} · após pré-filtro {} · detalhados {} ({do_cache} do cache) · falhas {} · resultado {}",
                lista.len(),
                candidatos.len(),
                dets.len(),
                falhas.len(),
                regs.len()
            ),
        ),
        (
            "Etapa".into(),
            "Texto exato do quadro: etapa com 'Está aqui'; só 'Ficou aqui' → 'Saiu – Próxima'; sem quadro → rótulo do documento + '(sem quadro de etapas padrão)'.".into(),
        ),
        (
            "Entrada na etapa".into(),
            "Evento 'Mudança de etapa' da linha do tempo (data/hora com segundos); sem eventos, despacho 'via etapa' mais recente para a etapa atual. Conferido contra o 'Está aqui N …' do quadro; se divergir além da granularidade do rótulo, vale o quadro (data estimada, marcada '~'). Sem registro: emissão (se for a 1ª etapa) ou estimativa pelo quadro. Em 'Saiu – Próxima' (Etapa Final não exibe 'Está aqui'), a data é a da entrada na Próxima.".into(),
        ),
        (
            "Setores envolvidos".into(),
            "Coluna 'Para' da caixa, com os setores escondidos no '+N' e nos badges truncados. Não é o setor que está com o processo: para isso, veja 'Últ. despacho — para'.".into(),
        ),
        ("Dias".into(), "Dias corridos de calendário até a data de posição.".into()),
    ];
    Ok(Resultado {
        regs,
        falhas,
        params,
        filtros,
        arquivo: None,
    })
}

/// `busca.main(argv, cred)`: analisa os argumentos, abre a sessão (ou usa
/// `cred`), executa e grava a planilha. Com `-h`, mostra a ajuda e devolve None.
pub fn main(amb: &Ambiente, argv: &[String], cred: Option<&Credenciais>) -> Res<Option<Resultado>> {
    let agora = parse::agora_br();
    let a = match args::analisar(argv)? {
        Analise::Ajuda(t) => {
            msg!(amb, "{t}");
            return Ok(None);
        }
        Analise::Ok(a) => *a,
    };
    let mut op = args::normalizar(a, agora.date())?;
    let a = op.a.clone();
    let (mut cli, mut setor) = (None, String::new());
    if !a.offline {
        let (sess, s) = abrir_sessao(amb, &a, cred)?;
        msg!(amb, "Sessão ativa — setor: {s}");
        setor = s;
        cli = Some(Cliente1Doc { sess });
    }
    let mut res = match executar(amb, &mut op, cli.as_ref(), &setor, agora) {
        Err(Erro::SessaoExpirada(_)) => {
            sessao::esquecer(amb);
            return Err(Erro::encerrar(
                "A sessão do 1Doc caiu durante a coleta. Execute de novo (o navegador vai abrir para o login); o que já foi detalhado está no cache.",
            ));
        }
        r => r?,
    };
    saida::imprimir_resumo(amb, &res.regs, a.mostrar);
    if !res.falhas.is_empty() {
        let lista: Vec<String> = res
            .falhas
            .iter()
            .take(15)
            .map(|f| format!("{} ({})", f.numero, f.motivo))
            .collect();
        msg!(
            amb,
            "\nFalhas ({}) — ficam como lacuna, sem chute: {}",
            res.falhas.len(),
            lista.join(", ")
        );
    }
    let mostrar_cita =
        a.cita_setor.is_some() || a.excecao_cita_setor.is_some() || !a.pessoa.is_empty();
    let marcar = op.mudanca_de.is_some()
        || op.mudanca_ate.is_some()
        || !a.mudanca_para.is_empty()
        || !a.mudanca_origem.is_empty();
    if !a.sem_xlsx {
        let saida = match a.saida.as_deref().filter(|s| !s.is_empty()) {
            Some(s) => PathBuf::from(s),
            None => {
                let cfg = config::carregar(amb);
                config::pasta_saida(amb, &cfg).join(format!(
                    "Busca 1Doc {}.xlsx",
                    agora.format("%d-%m-%Y %Hh%M")
                ))
            }
        };
        saida::escrever_xlsx(
            &saida,
            &res.regs,
            &res.params,
            &res.falhas,
            mostrar_cita,
            marcar,
        )?;
        let abs = std::path::absolute(&saida).unwrap_or(saida);
        msg!(amb, "\nPlanilha: {}", abs.display());
        if a.abrir {
            amb.abrir_arquivo(&abs);
        }
        res.arquivo = Some(abs);
    }
    if let Some(c) = a.csv.as_deref().filter(|c| !c.is_empty()) {
        saida::escrever_csv(Path::new(c), &res.regs, mostrar_cita)?;
        msg!(amb, "CSV: {c}");
    }
    Ok(Some(res))
}
