//! Configurações do usuário (config.json na pasta de dados), com as mesmas
//! chaves da versão Python.

use std::path::PathBuf;

use serde_json::{Map, Value, json};

use crate::ambiente::Ambiente;
use crate::arquivos;

pub const NAVEGADORES: [(&str, &str); 3] = [
    ("auto", "Automático (Chrome; se não houver, Edge)"),
    ("chrome", "Google Chrome"),
    ("edge", "Microsoft Edge"),
];

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// auto (Chrome, senão Edge), chrome ou edge
    pub navegador: String,
    /// vazio = Área de Trabalho
    pub pasta_saida: String,
    /// abre a planilha no Excel ao terminar
    pub abrir_planilha: bool,
    /// quanto tempo esperar o login no navegador
    pub minutos_login: u64,
    /// última planilha usada em cada aba
    pub planilha_faltantes: String,
    pub planilha_verificar: String,
    /// período de emissão da aba Faltantes, como digitado
    pub faltantes_de: String,
    pub faltantes_ate: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            navegador: "auto".into(),
            pasta_saida: String::new(),
            abrir_planilha: true,
            minutos_login: 10,
            planilha_faltantes: String::new(),
            planilha_verificar: String::new(),
            faltantes_de: String::new(),
            faltantes_ate: String::new(),
        }
    }
}

fn arquivo(amb: &Ambiente) -> PathBuf {
    amb.arquivo_dados("config.json")
}

/// `int(x)` do Python para os valores que o JSON pode trazer.
fn inteiro(v: &Value) -> Option<i64> {
    match v {
        Value::Bool(b) => Some(*b as i64),
        Value::Number(n) => n.as_i64().or_else(|| {
            n.as_f64()
                .filter(|f| f.is_finite())
                .map(|f| f.trunc() as i64)
        }),
        Value::String(s) => s.trim().parse::<i64>().ok(),
        _ => None,
    }
}

pub fn carregar(amb: &Ambiente) -> Config {
    let mut c = Config::default();
    let Some(Value::Object(lido)) = arquivos::ler_json(&arquivo(amb)) else {
        return c;
    };
    let txt = |chave: &str, atual: &mut String| {
        if let Some(v) = lido.get(chave) {
            *atual = v.as_str().map(str::to_string).unwrap_or_default();
        }
    };
    if let Some(Value::String(n)) = lido.get("navegador") {
        c.navegador = n.clone();
    } else if lido.contains_key("navegador") {
        c.navegador = String::new();
    }
    if !NAVEGADORES.iter().any(|(k, _)| *k == c.navegador) {
        c.navegador = "auto".into();
    }
    txt("pasta_saida", &mut c.pasta_saida);
    txt("planilha_faltantes", &mut c.planilha_faltantes);
    txt("planilha_verificar", &mut c.planilha_verificar);
    txt("faltantes_de", &mut c.faltantes_de);
    txt("faltantes_ate", &mut c.faltantes_ate);
    if let Some(v) = lido.get("abrir_planilha") {
        c.abrir_planilha = arquivos::verdadeiro(v);
    }
    if let Some(v) = lido.get("minutos_login") {
        c.minutos_login = inteiro(v).map(|n| n.max(1) as u64).unwrap_or(10);
    }
    c
}

pub fn para_json(c: &Config) -> Value {
    let mut m = Map::new();
    m.insert("navegador".into(), json!(c.navegador));
    m.insert("pasta_saida".into(), json!(c.pasta_saida));
    m.insert("abrir_planilha".into(), json!(c.abrir_planilha));
    m.insert("minutos_login".into(), json!(c.minutos_login));
    m.insert("planilha_faltantes".into(), json!(c.planilha_faltantes));
    m.insert("planilha_verificar".into(), json!(c.planilha_verificar));
    m.insert("faltantes_de".into(), json!(c.faltantes_de));
    m.insert("faltantes_ate".into(), json!(c.faltantes_ate));
    Value::Object(m)
}

pub fn salvar(amb: &Ambiente, c: &Config) -> std::io::Result<()> {
    arquivos::gravar_atomico(&arquivo(amb), &arquivos::json_indentado(&para_json(c)))
}

/// Pasta de saída válida: a configurada, se existir; senão a Área de Trabalho.
pub fn pasta_saida(amb: &Ambiente, c: &Config) -> PathBuf {
    let p = c.pasta_saida.trim();
    if !p.is_empty() && std::path::Path::new(p).is_dir() {
        return PathBuf::from(p);
    }
    amb.area_de_trabalho()
}
