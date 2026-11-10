//! Formulário da aba "Busca" → argumentos de linha de comando da busca
//! (porta de busca_argv.py). A interface só junta os valores dos campos e
//! chama `montar_argv`; quem interpreta é o mesmo analisador da busca.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Mutex;

use chrono::NaiveDateTime;
use serde_json::Value;

use crate::busca::{self, args};
use crate::re;
use crate::texto::{casefold, sem_acento};

pub const CAIXAS: [&str; 4] = ["entrada", "saida", "arquivo", "favoritos"];
pub const PRIORIDADES: [&str; 4] = ["Urgente", "Alta", "Média", "Baixa"];

/// Instrução de datas mostrada na aba, com um exemplo de cada formato.
pub const EXEMPLOS_DATA: [&str; 5] = ["05/11/2026", "2026-11-05", "hoje", "ontem", "-7"];
pub const DICA_DATAS: &str = "Datas: DD/MM/AAAA (05/11/2026), AAAA-MM-DD, hoje, ontem\nou -N = N dias atrás (-7). Inclui o próprio dia informado.";

/// Fluxo da Abertura de Procedimento Licitatório.
pub const ETAPAS_BASE: [&str; 7] = [
    "Validação de Requisição",
    "Correção de Documentação",
    "Pesquisa de Mercado",
    "Consulta Orçamentária",
    "Análise do Controle Interno",
    "Aprovação do COMPAC",
    "Etapa Final",
];
/// campos de etapa da aba (lista suspensa); os de TEXTOS_MULTIPLOS aceitam várias
pub const CAMPOS_ETAPA: [&str; 7] = [
    "etapa",
    "excluir_etapa",
    "antes_de",
    "a_partir_de",
    "passou_por",
    "mudanca_para",
    "mudanca_origem",
];

/// campo do formulário → opção da busca (texto; aceita vários separados por ";")
pub const TEXTOS_MULTIPLOS: [(&str, &str); 6] = [
    ("etapa", "--etapa"),
    ("excluir_etapa", "--excluir-etapa"),
    ("objeto", "--objeto"),
    ("mudanca_para", "--mudanca-para"),
    ("mudanca_origem", "--mudanca-origem"),
    ("passou_por", "--passou-por"),
];
/// campo → opção (valor único)
pub const TEXTOS: [(&str, &str); 9] = [
    ("antes_de", "--antes-de"),
    ("a_partir_de", "--a-partir-de"),
    ("emissao_de", "--emissao-de"),
    ("emissao_ate", "--emissao-ate"),
    ("mudanca_de", "--mudanca-de"),
    ("mudanca_ate", "--mudanca-ate"),
    ("entrada_de", "--entrada-de"),
    ("entrada_ate", "--entrada-ate"),
    ("numeros_arquivo", "--numeros-arquivo"),
];
pub const INTEIROS: [(&str, &str); 3] = [
    ("dias_etapa_min", "--dias-etapa-min"),
    ("dias_etapa_max", "--dias-etapa-max"),
    ("sem_mov_min", "--sem-mov-min"),
];
pub const BOOLEANOS: [(&str, &str); 4] = [
    ("etapa_exata", "--etapa-exata"),
    ("offline", "--offline"),
    ("refazer", "--refazer"),
    ("relogar", "--relogar"),
];

#[derive(Debug, Clone, PartialEq)]
pub struct FormularioInvalido(pub String);

impl std::fmt::Display for FormularioInvalido {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Os valores dos campos da aba.
#[derive(Debug, Clone, Default)]
pub struct Formulario {
    pub textos: HashMap<String, String>,
    pub caixas: Vec<String>,
    pub prioridade: Vec<String>,
    pub marcados: HashSet<String>,
}

impl Formulario {
    pub fn texto(&self, k: &str) -> &str {
        self.textos.get(k).map(String::as_str).unwrap_or("")
    }

    pub fn com(mut self, k: &str, v: &str) -> Self {
        self.textos.insert(k.into(), v.into());
        self
    }

    pub fn marcar(mut self, k: &str) -> Self {
        self.marcados.insert(k.into());
        self
    }
}

/// Para comparar nomes de etapa: sem acento, sem caixa, espaços colapsados.
pub fn chave_etapa(nome: &str) -> String {
    re!(r"\s+")
        .replace_all(&casefold(&sem_acento(nome)), " ")
        .trim()
        .to_string()
}

/// Nomes únicos de etapa: primeiro o fluxo da Abertura, na ordem; depois as
/// outras, na ordem dos quadros em que aparecem. Vale a grafia lida no 1Doc.
pub fn ordenar_etapas(fluxos: &[Vec<String>]) -> Vec<String> {
    let mut grafia: HashMap<String, String> = HashMap::new();
    for fluxo in fluxos {
        for nome in fluxo {
            grafia
                .entry(chave_etapa(nome))
                .or_insert_with(|| nome.clone());
        }
    }
    let (mut saida, mut vistos) = (Vec::new(), HashSet::new());
    let todos = ETAPAS_BASE
        .iter()
        .map(|s| s.to_string())
        .chain(fluxos.iter().flatten().cloned());
    for nome in todos {
        let k = chave_etapa(&nome);
        if !k.is_empty() && vistos.insert(k.clone()) {
            saida.push(grafia.get(&k).cloned().unwrap_or(nome));
        }
    }
    saida
}

type Marca = (std::path::PathBuf, Option<std::time::SystemTime>, u64);
static MEMO: Mutex<Option<(Option<Marca>, Vec<String>)>> = Mutex::new(None);

/// Lista de etapas para os campos da aba: o fluxo da Abertura mais as
/// etapas dos quadros que a busca já leu (1doc_cache.json). O cache só é
/// relido quando muda.
pub fn etapas_conhecidas(arquivo_cache: &Path) -> Vec<String> {
    let marca = std::fs::metadata(arquivo_cache)
        .ok()
        .map(|m| (arquivo_cache.to_path_buf(), m.modified().ok(), m.len()));
    if let Some((m, e)) = MEMO.lock().unwrap().as_ref()
        && marca.is_some()
        && *m == marca
    {
        return e.clone();
    }
    let (mut fluxos, mut vistos): (Vec<Vec<String>>, HashSet<Vec<String>>) =
        (Vec::new(), HashSet::new());
    if marca.is_some()
        && let Some(dados) = crate::arquivos::ler_json(arquivo_cache)
        && let Some(Value::Object(det)) = dados.get("det")
    {
        for rec in det.values() {
            let fluxo: Vec<String> = rec
                .get("etapas_quadro")
                .and_then(Value::as_array)
                .map(|q| {
                    q.iter()
                        .filter_map(Value::as_str)
                        .map(str::trim)
                        .filter(|n| !n.is_empty())
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            if !fluxo.is_empty() && vistos.insert(fluxo.clone()) {
                fluxos.push(fluxo);
            }
        }
    }
    let etapas = ordenar_etapas(&fluxos);
    *MEMO.lock().unwrap() = Some((marca, etapas.clone()));
    etapas
}

fn varios(valor: &str) -> Vec<String> {
    re!(r"[;\n]")
        .split(valor)
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect()
}

/// Texto de um campo de etapa depois de escolher `escolhida` na lista.
/// Campo de várias etapas cujo texto termina em ';': ela se junta às que já
/// estão lá (sem repetir). Nos outros casos, substitui o texto.
pub fn juntar_etapa(antes: &str, escolhida: &str, varias: bool) -> String {
    if varias && antes.trim_end().ends_with(';') {
        let mut itens = varios(antes);
        if !itens
            .iter()
            .any(|i| chave_etapa(i) == chave_etapa(escolhida))
        {
            itens.push(escolhida.to_string());
        }
        return itens.join("; ");
    }
    escolhida.to_string()
}

/// Opções extras digitadas como na linha de comando (`shlex.split(posix=False)`:
/// caminhos do Windows não perdem as barras); as aspas em volta de cada
/// termo são retiradas.
pub fn dividir_extras(texto: &str) -> Result<Vec<String>, FormularioInvalido> {
    let mut termos = Vec::new();
    let (mut atual, mut aspas, mut tem) = (String::new(), None::<char>, false);
    for c in texto.chars() {
        match aspas {
            Some(q) => {
                atual.push(c);
                if c == q {
                    aspas = None;
                }
            }
            None if c.is_whitespace() => {
                if tem {
                    termos.push(std::mem::take(&mut atual));
                    tem = false;
                }
            }
            None => {
                if c == '"' || c == '\'' {
                    aspas = Some(c);
                }
                atual.push(c);
                tem = true;
            }
        }
    }
    if aspas.is_some() {
        return Err(FormularioInvalido(
            "Outras opções: aspas sem fechar.".into(),
        ));
    }
    if tem {
        termos.push(atual);
    }
    Ok(termos
        .into_iter()
        .map(|t| {
            let c: Vec<char> = t.chars().collect();
            if c.len() >= 2 && c[0] == c[c.len() - 1] && (c[0] == '"' || c[0] == '\'') {
                c[1..c.len() - 1].iter().collect()
            } else {
                t
            }
        })
        .collect())
}

/// Monta a lista de argumentos da busca a partir do formulário.
pub fn montar_argv(
    form: &Formulario,
    pasta_saida: Option<&Path>,
    agora: NaiveDateTime,
    abrir: bool,
) -> Result<Vec<String>, FormularioInvalido> {
    let mut argv: Vec<String> = Vec::new();
    let caixas: Vec<&str> = CAIXAS
        .iter()
        .copied()
        .filter(|c| form.caixas.iter().any(|x| x == c))
        .collect();
    let numeros = form.texto("numeros").trim();
    if caixas.is_empty()
        && numeros.is_empty()
        && form.texto("numeros_arquivo").is_empty()
        && !form.marcados.contains("offline")
    {
        return Err(FormularioInvalido(
            "Marque ao menos uma caixa (ou informe números de processo).".into(),
        ));
    }
    if !caixas.is_empty() {
        argv.push("--caixas".into());
        argv.push(if caixas.len() == CAIXAS.len() {
            "todas".into()
        } else {
            caixas.join(",")
        });
    }
    if !numeros.is_empty() {
        argv.push("--numeros".into());
        argv.push(
            re!(r"[\s,;]+")
                .split(numeros)
                .filter(|n| !n.is_empty())
                .collect::<Vec<_>>()
                .join(","),
        );
    }
    for (campo, opcao) in TEXTOS {
        let v = form.texto(campo).trim();
        if !v.is_empty() {
            argv.push(opcao.into());
            argv.push(v.into());
        }
    }
    for (campo, opcao) in TEXTOS_MULTIPLOS {
        for v in varios(form.texto(campo)) {
            argv.push(opcao.into());
            argv.push(v);
        }
    }
    for (campo, opcao) in INTEIROS {
        let v = form.texto(campo).trim();
        if !v.is_empty() {
            if !re!(r"^\d+$").is_match(v) {
                return Err(FormularioInvalido(format!(
                    "'{v}' não é um número inteiro ({opcao})."
                )));
            }
            argv.push(opcao.into());
            argv.push(v.into());
        }
    }
    let fase = form.texto("fase").trim();
    if !fase.is_empty() {
        argv.push("--fase".into());
        argv.push(fase.into());
    }
    let prios: Vec<&str> = PRIORIDADES
        .iter()
        .copied()
        .filter(|p| form.prioridade.iter().any(|x| x == p))
        .collect();
    if !prios.is_empty() && prios.len() < PRIORIDADES.len() {
        argv.push("--prioridade".into());
        argv.push(prios.join(","));
    }
    for (campo, opcao) in BOOLEANOS {
        if form.marcados.contains(campo) {
            argv.push(opcao.into());
        }
    }
    if form.marcados.contains("csv") {
        let nome = format!("Busca 1Doc {}.csv", agora.format("%d-%m-%Y %Hh%M"));
        argv.push("--csv".into());
        argv.push(match pasta_saida {
            Some(p) => p.join(nome).to_string_lossy().into_owned(),
            None => nome,
        });
    }
    if abrir {
        argv.push("--abrir".into());
    }
    argv.extend(dividir_extras(form.texto("extras"))?);
    Ok(argv)
}

/// Passa os argumentos pelo analisador da busca sem executar nada. None se
/// estão certos; senão, a mensagem de erro.
pub fn validar(argv: &[String]) -> Option<String> {
    match args::analisar(argv) {
        Err(e) => Some(e.to_string()),
        Ok(args::Analise::Ajuda(_)) => Some("argumentos inválidos".into()),
        Ok(args::Analise::Ok(a)) => args::normalizar(*a, busca::parse::agora_br().date())
            .err()
            .map(|e| e.to_string()),
    }
}
