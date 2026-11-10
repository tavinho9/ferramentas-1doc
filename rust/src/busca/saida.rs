//! Saída da busca: planilha (Resultado, Transições, Parâmetros), CSV e resumo.

use std::path::Path;

use rust_xlsxwriter::{Color, Format, FormatAlign, FormatUnderline, Workbook, Worksheet};

use super::registro::{Campo, Registro};
use crate::ambiente::Ambiente;
use crate::erro::{Erro, Res};
use crate::planilha::xl;
use crate::texto::corta;
use crate::{msg, paralelo, re};

pub const COLUNAS: [(&str, &str, f64); 26] = [
    ("Nº Processo", "numero", 13.0),
    ("Emissão", "emissao", 16.0),
    ("Prioridade", "prioridade", 10.0),
    ("Objeto", "assunto", 55.0),
    ("Assunto (1Doc)", "tipo", 30.0),
    ("Fase", "fase", 11.0),
    ("Etapa atual", "etapa", 28.0),
    ("Entrou na etapa em", "entrada", 17.0),
    ("Fonte da data", "entrada_fonte", 20.0),
    ("Dias na etapa", "dias_etapa", 9.0),
    ("Setor da etapa", "setor_etapa", 18.0),
    ("Etapa do meu setor", "etapa_meu_setor", 10.0),
    ("Últ. despacho — para", "ult_para", 26.0),
    ("Últ. despacho — de", "ult_de", 30.0),
    ("Última movimentação", "ult_mov", 17.0),
    ("Dias sem mov.", "dias_sem_mov", 9.0),
    ("Histórico de etapas", "historico", 60.0),
    ("Nº mudanças", "n_mudancas", 9.0),
    ("Cita setor-alvo", "cita_alvo", 10.0),
    ("Observação", "obs", 45.0),
    ("Situação", "situacao", 12.0),
    ("Status no meu setor", "status_setor", 16.0),
    ("Processo de origem", "pai", 13.0),
    ("Processos derivados", "derivados", 16.0),
    ("Setores envolvidos (caixa)", "para", 34.0),
    ("Caixa", "caixa", 14.0),
];
const CAMPOS_SIM_NAO: [&str; 2] = ["cita_alvo", "etapa_meu_setor"];

fn valor(r: &Registro, campo: &str) -> Campo {
    if campo == "obs" {
        return Campo::Texto(r.obs.join("; "));
    }
    let v = r.campo(campo);
    if CAMPOS_SIM_NAO.contains(&campo) {
        return match v {
            Campo::Bool(b) => Campo::Texto(if b { "sim" } else { "não" }.into()),
            _ => Campo::Texto(String::new()),
        };
    }
    v
}

/// Caractere de controle (comum em texto colado do Word) faz o Excel recusar
/// a célula: troca por espaço.
fn limpo(s: &str) -> String {
    s.chars()
        .map(|c| {
            if matches!(c as u32, 0x00..=0x08 | 0x0B | 0x0C | 0x0E..=0x1F) {
                ' '
            } else {
                c
            }
        })
        .collect()
}

fn escrever(
    ws: &mut Worksheet,
    l: u32,
    c: u16,
    v: &Campo,
    base: &Format,
    data: &Format,
) -> Res<()> {
    match v {
        Campo::Nada => ws.write_blank(l, c, base).map(|_| ()),
        Campo::Texto(t) if t.is_empty() => ws.write_blank(l, c, base).map(|_| ()),
        Campo::Texto(t) => ws
            .write_string_with_format(l, c, corta(&limpo(t), 32767), base)
            .map(|_| ()),
        Campo::Int(i) => ws
            .write_number_with_format(l, c, *i as f64, base)
            .map(|_| ()),
        Campo::Bool(b) => ws.write_boolean_with_format(l, c, *b, base).map(|_| ()),
        Campo::Data(d) => ws.write_datetime_with_format(l, c, d, data).map(|_| ()),
    }
    .map_err(xl)
}

fn cabecalho(ws: &mut Worksheet, cols: &[(&str, &str, f64)]) -> Res<()> {
    let f = Format::new()
        .set_bold()
        .set_font_color(Color::White)
        .set_background_color(Color::RGB(0x1F3A5F))
        .set_align(FormatAlign::VerticalCenter)
        .set_text_wrap();
    for (j, (rot, _, larg)) in cols.iter().enumerate() {
        ws.write_string_with_format(0, j as u16, *rot, &f)
            .map_err(xl)?;
        ws.set_column_width(j as u16, *larg).map_err(xl)?;
    }
    ws.set_freeze_panes(1, 1).map_err(xl)?;
    ws.set_row_height(0, 30).map_err(xl)?;
    Ok(())
}

pub struct Falha {
    pub numero: String,
    pub motivo: String,
    pub obtido: String,
}

pub fn escrever_xlsx(
    caminho: &Path,
    regs: &[Registro],
    params: &[(String, String)],
    falhas: &[Falha],
    mostrar_cita: bool,
    marcar_periodo: bool,
) -> Res<()> {
    let mut wb = Workbook::new();
    let dh = Format::new().set_num_format("dd/mm/yyyy hh:mm");
    let normal = Format::new();
    let quebra = Format::new().set_text_wrap().set_align(FormatAlign::Top);
    let quebra_dh = quebra.clone().set_num_format("dd/mm/yyyy hh:mm");
    let link = Format::new()
        .set_font_color(Color::RGB(0x0563C1))
        .set_underline(FormatUnderline::Single);

    let cols: Vec<(&str, &str, f64)> = COLUNAS
        .iter()
        .copied()
        .filter(|c| mostrar_cita || c.1 != "cita_alvo")
        .collect();
    let ws = wb.add_worksheet();
    ws.set_name("Resultado").map_err(xl)?;
    cabecalho(ws, &cols)?;
    for (i, r) in regs.iter().enumerate() {
        let l = (i + 1) as u32;
        for (j, (_, campo, _)) in cols.iter().enumerate() {
            let v = valor(r, campo);
            let largo = ["assunto", "historico", "obs", "para"].contains(campo);
            if *campo == "numero" && !r.url.is_empty() && !r.numero.is_empty() {
                crate::planilha::link(ws, l, j as u16, &r.url, &limpo(&r.numero), &link)?;
                continue;
            }
            let (base, data) = if largo {
                (&quebra, &quebra_dh)
            } else {
                (&normal, &dh)
            };
            escrever(ws, l, j as u16, &v, base, data)?;
        }
    }
    ws.autofilter(0, 0, regs.len() as u32, (cols.len() - 1) as u16)
        .map_err(xl)?;

    let mut tcols: Vec<(&str, &str, f64)> = vec![
        ("Nº Processo", "numero", 13.0),
        ("Objeto", "assunto", 45.0),
        ("Data/hora", "data", 17.0),
        ("Etapa de origem", "de", 26.0),
        ("Etapa de destino", "para", 26.0),
        ("Estimada?", "estimada", 10.0),
        ("Fonte", "fonte", 18.0),
        ("Despacho nº", "despacho", 10.0),
        ("Autor", "autor", 24.0),
        ("Setor de origem", "setor_de", 18.0),
        ("Destinatário", "destinatario", 22.0),
        ("Etapa atual do processo", "etapa_atual", 26.0),
    ];
    if marcar_periodo {
        tcols.insert(6, ("No período filtrado?", "no_periodo", 11.0));
    }
    let wt = wb.add_worksheet();
    wt.set_name("Transições").map_err(xl)?;
    cabecalho(wt, &tcols)?;
    let mut lin = 1u32;
    for r in regs {
        for (ti, t) in r.transicoes.iter().enumerate() {
            if t.mesma_etapa {
                continue;
            }
            for (j, (_, campo, _)) in tcols.iter().enumerate() {
                let v = match *campo {
                    "numero" => Campo::Texto(r.numero.clone()),
                    "assunto" => Campo::Texto(r.assunto.clone()),
                    "etapa_atual" => Campo::Texto(r.etapa.clone()),
                    "data" => t.data.map(Campo::Data).unwrap_or(Campo::Nada),
                    "de" => Campo::Texto(if t.de.is_empty() {
                        "(início)".into()
                    } else {
                        t.de.clone()
                    }),
                    "estimada" => Campo::Texto(if t.estimada { "sim" } else { "não" }.into()),
                    "no_periodo" => Campo::Texto(
                        if r.trans_periodo.contains(&ti) {
                            "sim"
                        } else {
                            "não"
                        }
                        .into(),
                    ),
                    outro => Campo::Texto(t.campo(outro)),
                };
                escrever(wt, lin, j as u16, &v, &normal, &dh)?;
            }
            lin += 1;
        }
    }
    wt.autofilter(0, 0, lin - 1, (tcols.len() - 1) as u16)
        .map_err(xl)?;

    let wp = wb.add_worksheet();
    wp.set_name("Parâmetros").map_err(xl)?;
    wp.set_column_width(0, 30).map_err(xl)?;
    wp.set_column_width(1, 110).map_err(xl)?;
    let negrito = Format::new().set_bold();
    for (i, (k, v)) in params.iter().enumerate() {
        wp.write_string_with_format(i as u32, 0, k, &negrito)
            .map_err(xl)?;
        crate::planilha::texto(wp, i as u32, 1, &limpo(v), &quebra)?;
    }
    if !falhas.is_empty() {
        let base = (params.len() + 1) as u32;
        let vermelho = Format::new()
            .set_bold()
            .set_font_color(Color::RGB(0xC00000));
        wp.write_string_with_format(base, 0, "Falhas (lacunas declaradas)", &vermelho)
            .map_err(xl)?;
        for (i, f) in falhas.iter().enumerate() {
            let l = base + 1 + i as u32;
            crate::planilha::texto(wp, l, 0, &f.numero, &normal)?;
            let t = if f.obtido.is_empty() {
                f.motivo.clone()
            } else {
                format!("{} — obtido: {}", f.motivo, f.obtido)
            };
            crate::planilha::texto(wp, l, 1, &limpo(&t), &normal)?;
        }
    }
    if let Some(p) = caminho.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(p)?;
    }
    wb.save(caminho).map_err(xl)
}

pub fn escrever_csv(caminho: &Path, regs: &[Registro], mostrar_cita: bool) -> Res<()> {
    let cols: Vec<&(&str, &str, f64)> = COLUNAS
        .iter()
        .filter(|c| mostrar_cita || c.1 != "cita_alvo")
        .collect();
    let mut bytes = "\u{feff}".as_bytes().to_vec(); // utf-8-sig
    {
        let mut w = csv::WriterBuilder::new()
            .delimiter(b';')
            .terminator(csv::Terminator::CRLF)
            .from_writer(&mut bytes);
        let erro = |e: csv::Error| Erro::outro(e.to_string());
        w.write_record(cols.iter().map(|c| c.0)).map_err(erro)?;
        for r in regs {
            let linha: Vec<String> = cols
                .iter()
                .map(|(_, campo, _)| match valor(r, campo) {
                    Campo::Nada => String::new(),
                    Campo::Texto(t) => t,
                    Campo::Int(i) => i.to_string(),
                    Campo::Bool(b) => if b { "True" } else { "False" }.into(),
                    Campo::Data(d) => d.format("%d/%m/%Y %H:%M").to_string(),
                })
                .collect();
            w.write_record(&linha).map_err(erro)?;
        }
        w.flush()?;
    }
    std::fs::write(caminho, bytes)?;
    Ok(())
}

fn esq(s: &str, n: usize) -> String {
    format!("{:<n$}", s, n = n)
}

pub fn imprimir_resumo(amb: &Ambiente, regs: &[Registro], mostrar: i64) {
    if regs.is_empty() {
        msg!(amb, "\nNenhum processo atende aos filtros.");
        return;
    }
    let dist = paralelo::mais_comuns(regs.iter().map(|r| {
        if r.etapa.is_empty() {
            "(sem etapa)".into()
        } else {
            r.etapa.clone()
        }
    }));
    msg!(amb, "\nResultado: {} processo(s)", regs.len());
    msg!(
        amb,
        "  Por etapa: {}",
        dist.iter()
            .map(|(k, v)| format!("{k} {v}"))
            .collect::<Vec<_>>()
            .join(" · ")
    );
    msg!(
        amb,
        "\n{}{}{}{:>5}  {}Objeto",
        esq("Nº", 13),
        esq("Etapa atual", 32),
        esq("Desde", 13),
        "Dias",
        esq("Últ. despacho p/", 22)
    );
    let mostrar = mostrar.max(0) as usize;
    for r in regs.iter().take(mostrar) {
        let mut desde = r
            .entrada
            .map(|e| e.format("%d/%m %H:%M").to_string())
            .unwrap_or_else(|| "—".into());
        if let Some(e) = r.entrada
            && r.entrada_fonte.contains("estimada")
        {
            desde = format!("{} ~", e.format("%d/%m"));
        }
        let dias = r.dias_etapa.map(|d| d.to_string()).unwrap_or_default();
        let ult_p = re!(r"\s*\([^)]*\)")
            .replace_all(&r.ult_para, "")
            .to_string();
        msg!(
            amb,
            "{}{}{}{:>5}  {}{}",
            esq(&r.numero, 13),
            esq(&corta(&r.etapa, 31), 32),
            esq(&desde, 13),
            dias,
            esq(&corta(&ult_p, 21), 22),
            corta(&r.assunto, 60)
        );
    }
    if regs.len() > mostrar {
        msg!(amb, "… e mais {} (na planilha).", regs.len() - mostrar);
    }
}
