//! Utilidades das planilhas .xlsx geradas (rust_xlsxwriter).

use rust_xlsxwriter::{Format, Url, Worksheet};

use crate::erro::{Erro, Res};

pub fn xl(e: rust_xlsxwriter::XlsxError) -> Erro {
    Erro::Outro(format!("planilha: {e}"))
}

/// Tira os caracteres de controle que o Excel não aceita numa célula (o
/// `ILLEGAL_CHARACTERS_RE` do openpyxl).
pub fn sem_controle(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(*c as u32, 0x00..=0x08 | 0x0B | 0x0C | 0x0E..=0x1F))
        .collect()
}

/// Texto numa célula; vazio vira célula em branco (como o openpyxl, que
/// grava "" sem valor).
pub fn texto(ws: &mut Worksheet, linha: u32, col: u16, valor: &str, f: &Format) -> Res<()> {
    if valor.is_empty() {
        ws.write_blank(linha, col, f).map_err(xl)?;
    } else {
        ws.write_string_with_format(linha, col, valor, f)
            .map_err(xl)?;
    }
    Ok(())
}

/// Texto com hiperlink.
pub fn link(
    ws: &mut Worksheet,
    linha: u32,
    col: u16,
    url: &str,
    valor: &str,
    f: &Format,
) -> Res<()> {
    if valor.is_empty() {
        return texto(ws, linha, col, valor, f);
    }
    ws.write_url_with_format(linha, col, Url::new(url).set_text(valor), f)
        .map_err(xl)?;
    Ok(())
}

/// Letra da coluna (1 = A).
pub fn letra(mut col: u32) -> String {
    let mut s = Vec::new();
    while col > 0 {
        let r = (col - 1) % 26;
        s.push((b'A' + r as u8) as char);
        col = (col - 1) / 26;
    }
    s.iter().rev().collect()
}

/// Índice da coluna (A = 1).
pub fn indice(letras: &str) -> u32 {
    letras
        .chars()
        .filter(|c| c.is_ascii_alphabetic())
        .fold(0, |n, c| n * 26 + (c.to_ascii_uppercase() as u32 - 64))
}
