//! As quatro ferramentas de ponta a ponta contra o 1Doc de mentira
//! (porta de tests/test_ferramentas.py).

mod comum;

use chrono::Duration;
use ferramentas1doc::processos_do_dia as pdd;
use ferramentas1doc::{Erro, sessao};

fn opcoes() -> pdd::Opcoes {
    pdd::Opcoes {
        pausa: 0.0,
        ..Default::default()
    }
}

#[test]
fn processos_do_dia() {
    let fake = comum::Fake1Doc::novo();
    let d = comum::dados(&fake.base);
    let hoje = comum::hoje();
    let caminho = pdd::executar(&d.amb, hoje, &comum::cred(), opcoes())
        .unwrap()
        .unwrap();
    assert_eq!(
        caminho,
        d.pasta_saida()
            .join(format!("processos novos {}.xlsx", hoje.format("%d.%m")))
    );
    let linhas = comum::valores(&caminho, None);
    assert_eq!(
        linhas[0],
        ["Nº Processo", "Prioridade", "Objeto", "Etapa", "Secretaria"]
    );
    // hoje: só entra quem está na etapa atual há menos de 24h (P1, "Esta aqui 2 horas")
    assert_eq!(
        linhas[1..]
            .iter()
            .map(|l| l[0].as_str())
            .collect::<Vec<_>>(),
        ["31.632/2026"]
    );
    assert_eq!(
        linhas[1][2],
        "Aquisição de medicamentos para a rede municipal de saúde"
    ); // campo da abertura
    assert_eq!(linhas[1][3], "Pesquisa de Mercado — Abertura");
    assert_eq!(linhas[1][4], "Secretaria de Saúde");
    assert_eq!(
        comum::link(&caminho, None, 1, 2).as_deref(),
        Some(format!("{}/?pg=doc/ver&hash=AAAA1111BBBB2222CCCC&itd=3", fake.base).as_str())
    );
    let js: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(d.pasta_saida().join("processos_do_dia.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(js["total"], 2);
    assert_eq!(js["dia"], hoje.format("%d/%m/%Y").to_string());
    let p2 = js["itens"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["numero"] == "31.700/2026")
        .unwrap();
    assert_eq!(p2["incluir"], false);
    assert_eq!(p2["motivo"], "etapa atual há 1 dia ou mais");
    assert!(fake.chaves().contains(&("GET".into(), "caixa".into())));
}

#[test]
fn processos_do_dia_rejeita_futuro() {
    let fake = comum::Fake1Doc::novo();
    let d = comum::dados(&fake.base);
    let e = pdd::executar(
        &d.amb,
        comum::hoje() + Duration::days(1),
        &comum::cred(),
        opcoes(),
    )
    .unwrap_err();
    assert!(
        matches!(&e, Erro::Encerrar(m) if m.contains("ainda não chegou")),
        "{e:?}"
    );
}

#[test]
fn processos_do_dia_sessao_caida_apaga_a_salva() {
    let fake = comum::Fake1Doc::novo();
    fake.deslogar();
    let d = comum::dados(&fake.base);
    sessao::salvar(&d.amb, &comum::cred()).unwrap();
    assert_eq!(
        pdd::executar(&d.amb, comum::hoje(), &comum::cred(), opcoes()).unwrap(),
        None
    );
    assert!(sessao::carregar_salva(&d.amb).is_none());
}

#[test]
fn cancelar_interrompe() {
    let fake = comum::Fake1Doc::novo();
    let d = comum::dados(&fake.base);
    d.amb.cancelar.pedir();
    assert_eq!(
        pdd::executar(&d.amb, comum::hoje(), &comum::cred(), opcoes()).unwrap_err(),
        Erro::Cancelado
    );
}
