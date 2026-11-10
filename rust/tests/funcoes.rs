//! Funções puras: datas, critérios de etapa, formulário da busca, log
//! (porta de tests/test_funcoes.py).

use chrono::NaiveDate;
use ferramentas1doc::datas;
use ferramentas1doc::saida::{TelaEmMemoria, Terminal};
use ferramentas1doc::texto;

fn d(a: i32, m: u32, dia: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(a, m, dia).unwrap()
}

const HOJE: (i32, u32, u32) = (2026, 1, 15);

fn hoje() -> NaiveDate {
    d(HOJE.0, HOJE.1, HOJE.2)
}

#[test]
fn interpretar_data() {
    let casos = [
        ("15/01/2026", Some(d(2026, 1, 15))),
        ("15/01/26", Some(d(2026, 1, 15))),
        ("15-01-2026", Some(d(2026, 1, 15))),
        ("15012026", Some(d(2026, 1, 15))),
        ("10/01", Some(d(2026, 1, 10))),
        ("20/12", Some(d(2025, 12, 20))), // sem ano e no futuro: ano anterior
        ("31/02/2026", None),
        ("abc", None),
    ];
    for (txt, esperado) in casos {
        assert_eq!(datas::interpretar_data(txt, hoje()), esperado, "{txt}");
    }
}

#[test]
fn faltantes_ler_data() {
    let casos = [
        ("", None),
        ("  ", None),
        ("hoje", Some(hoje())),
        ("Ontem", Some(d(2026, 1, 14))),
        ("-30", Some(d(2025, 12, 16))),
        ("- 7", Some(d(2026, 1, 8))),
        ("01/01/2025", Some(d(2025, 1, 1))),
        ("1/1/25", Some(d(2025, 1, 1))),
        ("10/01", Some(d(2026, 1, 10))),
        ("20/12", Some(d(2025, 12, 20))),
    ];
    for (txt, esperado) in casos {
        assert_eq!(datas::ler_data(txt, hoje()), Ok(esperado), "{txt}");
    }
}

#[test]
fn faltantes_ler_data_invalida() {
    for txt in ["31/02/2026", "abc", "-99999999999", "2026"] {
        assert!(datas::ler_data(txt, hoje()).is_err(), "{txt}");
    }
}

#[test]
fn faltantes_ler_periodo() {
    assert_eq!(datas::ler_periodo("", "", hoje()), Ok((None, None)));
    assert_eq!(
        datas::ler_periodo("01/12/2025", "ontem", hoje()),
        Ok((Some(d(2025, 12, 1)), Some(d(2026, 1, 14))))
    );
    assert_eq!(
        datas::ler_periodo("", "31/12/2020", hoje()),
        Ok((None, Some(d(2020, 12, 31))))
    );
    assert!(
        datas::ler_periodo("", "32/01/2026", hoje())
            .unwrap_err()
            .contains("Data final não reconhecida")
    );
    assert!(
        datas::ler_periodo("16/01/2026", "", hoje())
            .unwrap_err()
            .contains("ainda não chegou")
    );
    assert!(
        datas::ler_periodo("10/01/2026", "09/01/2026", hoje())
            .unwrap_err()
            .contains("depois da final")
    );
}

#[test]
fn ler_numero() {
    assert_eq!(
        texto::ler_numero("Proc. Administrativo 31.632/2026"),
        Some(("31.632/2026".into(), "31632".into(), "2026".into()))
    );
    assert_eq!(
        texto::ler_numero("031632/2026"),
        Some(("31.632/2026".into(), "31632".into(), "2026".into()))
    );
    assert_eq!(texto::ler_numero("5712/2025").unwrap().0, "5.712/2025");
    assert_eq!(
        texto::ler_numero("1234567/2025").unwrap().0,
        "1.234.567/2025"
    );
    assert_eq!(texto::ler_numero("sem número"), None);
}

#[test]
fn norm_e_sem_acento() {
    assert_eq!(texto::norm("  Nº Processo "), "n processo");
    assert_eq!(texto::norm("Número-do Processo"), "numero do processo");
    assert_eq!(
        texto::normtxt("  Validação  de\nRequisição "),
        "validacao de requisicao"
    );
    assert_eq!(texto::limpo(" a \n\t b\u{a0}c "), "a b c");
}

#[test]
fn terminal_reescreve_linha_com_retorno_de_carro() {
    let mut t = Terminal::default();
    let mut tela = TelaEmMemoria::default();
    for pedaco in [
        "Lendo caixas\n",
        "  Em aberto: página 1/3",
        "\r  Em aberto: página 2/3",
        "\r",
        "  Em aberto: página 3/3, 25 processos",
        "\n",
        "fim\r\n",
    ] {
        t.alimentar(pedaco, &mut tela);
    }
    let linhas: Vec<&str> = tela.texto.split('\n').collect();
    assert_eq!(
        linhas,
        [
            "Lendo caixas",
            "  Em aberto: página 3/3, 25 processos",
            "fim",
            ""
        ]
    );
}

#[test]
fn etapa_dentro_24h() {
    use ferramentas1doc::processos_do_dia as pdd;
    for (txt, dentro) in [
        ("Esta aqui 1 hora", true),
        ("Aqui há alguns segundos", true),
        ("Está aqui 45 minutos", true),
        ("Está aqui 3 dias", false),
        ("Está aqui 1 semana", false),
        ("", false),
        ("???", false),
    ] {
        assert_eq!(pdd::etapa_dentro_24h(txt), dentro, "{txt}");
    }
}

#[test]
fn kanban_ignora_copia_do_modelo_de_despacho() {
    use ferramentas1doc::processos_do_dia as pdd;
    let html = concat!(
        r#"<div id="emissao_principal"><div class="kanban_estagios" data-current="true">"#,
        r#"<div class="kanban_titulo"><p>Validação de Requisição</p></div>"#,
        r#"<div class="kanban_itens"><p class="kanban_item">Esta aqui 3 dias</p></div></div></div>"#,
        r#"<div id="parent_wa"><div class="kanban_estagios" data-current="true">"#,
        r#"<div class="kanban_titulo"><p>Validação de Requisição</p></div>"#,
        r#"<div class="kanban_itens"><p class="kanban_item">Aqui há alguns segundos</p></div></div></div>"#
    );
    assert_eq!(
        pdd::ler_etapa_atual(html),
        Some(("Validação de Requisição".into(), "Esta aqui 3 dias".into()))
    );
}

#[test]
fn rotulo_etapa() {
    use ferramentas1doc::processos_do_dia as pdd;
    assert_eq!(
        pdd::rotulo_etapa("Etapa Final", "Abertura de Procedimento Licitatório", false),
        "Etapa Final — Abertura"
    );
    assert_eq!(
        pdd::rotulo_etapa("", "Elaboração de Documentação Preparatória", true),
        "Elaboração"
    );
    assert_eq!(
        pdd::rotulo_etapa("Etapa Final", "Outra fase", false),
        "Elaboração"
    );
}

#[test]
fn descricao_do_objeto() {
    use ferramentas1doc::processos_do_dia as pdd;
    let campo =
        |dentro: &str| format!(r#"<div id="campo_adicional_descricao_objeto">{dentro}</div>"#);
    assert_eq!(
        pdd::ler_descricao_objeto(&campo(
            r#"<div class="cp"><span data-valor="Compra de &lt;br&gt;papel">Objeto<span>*</span>:</span><small>x</small></div>"#
        )),
        "Compra de\npapel"
    );
    assert_eq!(
        pdd::ler_descricao_objeto(&campo(
            r#"<div class="cp"><span>Objeto*:</span><small>linha 1<br>linha   2</small></div>"#
        )),
        "linha 1\nlinha 2"
    );
    assert_eq!(
        pdd::ler_descricao_objeto(&campo("<p>Objeto: um</p><p>dois</p>")),
        "um\ndois"
    );
    assert_eq!(pdd::ler_descricao_objeto("<div id='outro'>x</div>"), "");
}
