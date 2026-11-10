//! Datas digitadas pela pessoa (os mesmos formatos dos scripts).

use chrono::{Datelike, Duration, Local, NaiveDate};

use crate::re;

pub fn hoje() -> NaiveDate {
    Local::now().date_naive()
}

pub fn ontem(hoje: NaiveDate) -> NaiveDate {
    hoje - Duration::days(1)
}

/// `hoje - timedelta(days=n)`; None se sair do calendário.
pub fn dias_atras(hoje: NaiveDate, n: &str) -> Option<NaiveDate> {
    let n: i64 = n.parse().ok()?;
    hoje.checked_sub_signed(Duration::try_days(n)?)
        .filter(|d| d.year() >= 1)
}

/// `'DD/MM/AAAA'`, `'DD/MM/AA'` ou `'DD/MM'` → data. Aceita `/`, `-`, `.`
/// ou espaço como separador, e também os dígitos colados (19082026,
/// 190826). Sem ano: o corrente — ou o anterior, se a data cairia no futuro
/// (digitar 20/12 em janeiro). None se não for uma data válida.
/// (`processos_do_dia.interpretar_data`)
pub fn interpretar_data(txt: &str, hoje: NaiveDate) -> Option<NaiveDate> {
    let txt = txt.trim();
    let mut partes: Vec<String> = if re!(r"^(?:\d{6}|\d{8})$").is_match(txt) && txt.is_ascii() {
        vec![
            txt[..2].to_string(),
            txt[2..4].to_string(),
            txt[4..].to_string(),
        ]
    } else {
        re!(r"[/\-.\s]+")
            .split(txt)
            .filter(|p| !p.is_empty())
            .map(str::to_string)
            .collect()
    };
    let sem_ano = partes.len() == 2;
    if sem_ano {
        partes.push(hoje.year().to_string());
    }
    if partes.len() != 3
        || !partes
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
        || !matches!(partes[2].len(), 2 | 4)
    {
        return None;
    }
    let d: u32 = partes[0].parse().ok()?;
    let m: u32 = partes[1].parse().ok()?;
    let mut a: i32 = partes[2].parse().ok()?;
    if partes[2].len() == 2 {
        a += 2000;
    }
    if a < 1 {
        return None;
    }
    let alvo = NaiveDate::from_ymd_opt(a, m, d)?;
    if sem_ano && alvo > hoje {
        return NaiveDate::from_ymd_opt(a - 1, m, d).filter(|_| a > 1);
    }
    Some(alvo)
}

/// `'17/08/2026'` ou `'17/08/2026 13:09'` → data (None se não casar).
pub fn data_br(txt: &str) -> Option<NaiveDate> {
    let m = re!(r"^\s*(\d{2})/(\d{2})/(\d{4})").captures(txt)?;
    NaiveDate::from_ymd_opt(m[3].parse().ok()?, m[2].parse().ok()?, m[1].parse().ok()?)
}

/// `{d:%d/%m/%Y}`.
pub fn br(d: NaiveDate) -> String {
    d.format("%d/%m/%Y").to_string()
}

pub const FORMATOS_DATA: &str = "DD/MM/AAAA, DD/MM/AA, DD/MM, hoje, ontem ou -N (N dias atrás)";

/// Data digitada (FORMATOS_DATA) → data; vazio → None. Erro com a mensagem
/// se não for uma data. (`faltantes.ler_data`)
pub fn ler_data(txt: &str, hoje: NaiveDate) -> Result<Option<NaiveDate>, String> {
    let t = re!(r"\s+").replace_all(txt.trim(), " ").to_lowercase();
    if t.is_empty() {
        return Ok(None);
    }
    if t == "hoje" {
        return Ok(Some(hoje));
    }
    if t == "ontem" {
        return Ok(Some(ontem(hoje)));
    }
    if let Some(m) = re!(r"^- ?(\d+)$").captures(&t) {
        return dias_atras(hoje, &m[1])
            .map(Some)
            .ok_or_else(|| format!("data fora do calendário: '{}'", txt.trim()));
    }
    interpretar_data(&t, hoje)
        .map(Some)
        .ok_or_else(|| format!("data não reconhecida: '{}'", txt.trim()))
}

/// Datas digitadas → (de, ate), cada uma data ou None (= padrão). Erro, com
/// a mensagem para o usuário, se o período não servir. (`faltantes.ler_periodo`)
pub fn ler_periodo(
    de_txt: &str,
    ate_txt: &str,
    hoje: NaiveDate,
) -> Result<(Option<NaiveDate>, Option<NaiveDate>), String> {
    let mut datas = Vec::new();
    for (nome, txt) in [("inicial", de_txt), ("final", ate_txt)] {
        match ler_data(txt, hoje) {
            Ok(d) => datas.push(d),
            Err(_) => return Err(format!("Data {nome} não reconhecida: \"{}\".", txt.trim())),
        }
    }
    let (de, ate) = (datas[0], datas[1]);
    if let Some(de) = de
        && de > hoje
    {
        return Err(format!("A data inicial ({}) ainda não chegou.", br(de)));
    }
    if let (Some(de), Some(ate)) = (de, ate)
        && de > ate
    {
        return Err(format!(
            "A data inicial ({}) é depois da final ({}).",
            br(de),
            br(ate)
        ));
    }
    Ok((de, ate))
}

/// `"DD/MM/AAAA a DD/MM/AAAA"`.
pub fn texto_periodo(de: NaiveDate, ate: NaiveDate, sep: &str) -> String {
    format!("{}{sep}{}", br(de), br(ate))
}

pub const DIAS_SEMANA: [&str; 7] = [
    "segunda-feira",
    "terça-feira",
    "quarta-feira",
    "quinta-feira",
    "sexta-feira",
    "sábado",
    "domingo",
];

pub fn dia_semana(d: NaiveDate) -> &'static str {
    DIAS_SEMANA[d.weekday().num_days_from_monday() as usize]
}
