//! Utilidades de texto comuns aos scripts.

use std::sync::LazyLock;

use regex::Regex;
use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

/// Regex compilada uma vez: `re!(r"...")`.
#[macro_export]
macro_rules! re {
    ($padrao:expr) => {{
        static R: std::sync::LazyLock<regex::Regex> =
            std::sync::LazyLock::new(|| regex::Regex::new($padrao).expect($padrao));
        &*R
    }};
}

/// Sem acentos (NFD sem as marcas combinantes, categoria Mn).
pub fn sem_acento(txt: &str) -> String {
    txt.nfd().filter(|c| !is_combining_mark(*c)).collect()
}

/// `casefold()` aproximado.
pub fn casefold(txt: &str) -> String {
    txt.to_lowercase().replace('ß', "ss")
}

/// `" ".join(txt.split())`: espaços colapsados, sem espaço nas pontas.
pub fn limpo(txt: &str) -> String {
    txt.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// minúsculas, sem acento, só letras/dígitos separados por 1 espaço.
pub fn norm(txt: &str) -> String {
    static NAO_ALFANUM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^a-z0-9]+").unwrap());
    limpo(&NAO_ALFANUM.replace_all(&sem_acento(txt).to_lowercase(), " "))
}

/// Comparação de textos: sem acento, sem caixa, espaços colapsados (`normtxt`).
pub fn normtxt(s: &str) -> String {
    static ESPACOS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());
    ESPACOS
        .replace_all(&casefold(&sem_acento(s)), " ")
        .trim()
        .to_string()
}

/// Número de processo: `'Proc. Administrativo 31.632/2026'` | `'31632/2026'`
/// → `("31.632/2026", "31632", "2026")`.
pub fn ler_numero(valor: &str) -> Option<(String, String, String)> {
    let m = re!(r"(\d{1,3}(?:\.\d{3})+|\d+)\s*/\s*(\d{4})").captures(valor)?;
    let bruto = m[1].replace('.', "");
    let digitos = bruto.trim_start_matches('0');
    let digitos = if digitos.is_empty() { "0" } else { digitos }.to_string();
    let ano = m[2].to_string();
    Some((format!("{}/{ano}", milhar(&digitos)), digitos, ano))
}

/// `f"{int(d):,}".replace(",", ".")` para uma sequência de dígitos.
pub fn milhar(digitos: &str) -> String {
    let d: Vec<char> = digitos.chars().collect();
    let mut s = String::new();
    for (i, c) in d.iter().enumerate() {
        if i > 0 && (d.len() - i).is_multiple_of(3) {
            s.push('.');
        }
        s.push(*c);
    }
    s
}

/// `texto[:n]` em caracteres.
pub fn corta(texto: &str, n: usize) -> String {
    texto.chars().take(n).collect()
}

/// Quantidade de caracteres (`len(texto)`).
pub fn compr(texto: &str) -> usize {
    texto.chars().count()
}

/// `texto.rsplit(" ", 1)[0]`.
pub fn ate_ultimo_espaco(texto: &str) -> &str {
    texto.rsplit_once(' ').map(|(a, _)| a).unwrap_or(texto)
}

/// `s[inicio:inicio + n]` com `inicio` em bytes (vindo de uma regex) e `n`
/// em caracteres, como no fatiamento de str do Python.
pub fn fatia(s: &str, inicio: usize, n: usize) -> &str {
    let resto = &s[inicio..];
    let fim = resto
        .char_indices()
        .nth(n)
        .map(|(i, _)| i)
        .unwrap_or(resto.len());
    &resto[..fim]
}

/// `str.splitlines()`.
pub fn linhas(s: &str) -> Vec<&str> {
    let mut v = Vec::new();
    let mut ini = 0;
    let mut it = s.char_indices().peekable();
    while let Some((i, c)) = it.next() {
        if matches!(
            c,
            '\n' | '\r'
                | '\u{b}'
                | '\u{c}'
                | '\u{1c}'
                | '\u{1d}'
                | '\u{1e}'
                | '\u{85}'
                | '\u{2028}'
                | '\u{2029}'
        ) {
            v.push(&s[ini..i]);
            let mut fim = i + c.len_utf8();
            if c == '\r' && it.peek().is_some_and(|(_, d)| *d == '\n') {
                it.next();
                fim += 1;
            }
            ini = fim;
        }
    }
    if ini < s.len() {
        v.push(&s[ini..]);
    }
    v
}
