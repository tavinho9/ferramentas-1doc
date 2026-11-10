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

#[test]
fn busca() {
    use ferramentas1doc::{busca, busca_argv};
    let fake = comum::Fake1Doc::novo();
    let d = comum::dados(&fake.base);
    let form = busca_argv::Formulario {
        caixas: vec!["entrada".into()],
        ..Default::default()
    }
    .com("etapa", "Pesquisa de Mercado")
    .marcar("csv");
    let mut argv = busca_argv::montar_argv(
        &form,
        Some(&d.pasta_saida()),
        chrono::Local::now().naive_local(),
        false,
    )
    .unwrap();
    argv.extend(["--pausa".into(), "0".into()]);
    let res = busca::main(&d.amb, &argv, Some(&comum::cred()))
        .unwrap()
        .unwrap();
    assert_eq!(
        res.regs
            .iter()
            .map(|r| r.numero.as_str())
            .collect::<Vec<_>>(),
        ["31.632/2026"]
    );
    let arquivo = res.arquivo.unwrap();
    assert_eq!(arquivo.parent().unwrap(), d.pasta_saida());
    let col_a: Vec<String> = comum::valores(&arquivo, Some("Resultado"))
        .into_iter()
        .map(|l| l[0].clone())
        .collect();
    assert_eq!(col_a, ["Nº Processo", "31.632/2026"]);
    assert!(
        std::fs::read_dir(d.pasta_saida()).unwrap().any(|e| e
            .unwrap()
            .path()
            .extension()
            .is_some_and(|x| x == "csv"))
    );
    assert!(d.pasta_dados().join("1doc_cache.json").is_file()); // cache na pasta de dados

    // a mesma consulta sem acessar o 1Doc, só com o cache
    fake.deslogar();
    let argv: Vec<String> = [
        "--offline",
        "--antes-de",
        "Pesquisa de Mercado",
        "--sem-xlsx",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let res2 = busca::main(&d.amb, &argv, None).unwrap().unwrap();
    assert_eq!(
        res2.regs
            .iter()
            .map(|r| r.numero.as_str())
            .collect::<Vec<_>>(),
        ["31.700/2026"]
    );
}

mod faltantes_testes {
    use super::comum;
    use chrono::Duration;
    use ferramentas1doc::{Erro, faltantes};
    use std::collections::HashSet;

    fn op() -> faltantes::Opcoes {
        faltantes::Opcoes {
            pausa: 0.0,
            sem_navegador: true,
            ..Default::default()
        }
    }

    fn br(d: chrono::NaiveDate) -> String {
        d.format("%d/%m/%Y").to_string()
    }

    fn intervalos(fake: &comum::Fake1Doc) -> HashSet<String> {
        fake.intervalos().into_iter().flatten().collect()
    }

    #[test]
    fn faltantes() {
        let fake = comum::Fake1Doc::novo();
        let d = comum::dados(&fake.base);
        let pl = comum::planilha(&d.pasta_dados().join("acomp.xlsx"), &["31.632/2026"]);
        let destino = faltantes::executar(&d.amb, &pl, None, &comum::cred(), op())
            .unwrap()
            .unwrap();
        assert_eq!(destino.parent().unwrap(), d.pasta_saida());
        let linhas = comum::valores(&destino, Some("Faltantes"));
        // P2 está antes da Pesquisa de Mercado e fora da planilha; P1 está na planilha
        assert_eq!(
            linhas[1..]
                .iter()
                .map(|l| l[0].as_str())
                .collect::<Vec<_>>(),
            ["31.700/2026"]
        );
        assert_eq!(linhas[1][2], "Reforma da Escola Municipal Exemplo");
        assert!(d.pasta_dados().join("hash_cache_1doc.json").is_file());
        // período padrão: de 1º/jan do ano mais antigo da planilha até hoje
        assert_eq!(
            intervalos(&fake),
            HashSet::from([format!("01/01/2026 - {}", br(comum::hoje()))])
        );
    }

    #[test]
    fn faltantes_periodo_escolhido() {
        let fake = comum::Fake1Doc::novo();
        let d = comum::dados(&fake.base);
        let pl = comum::planilha(&d.pasta_dados().join("acomp.xlsx"), &["31.632/2026"]);
        let hoje = comum::hoje();
        let o = faltantes::Opcoes {
            de: Some(hoje - Duration::days(10)),
            ate: Some(hoje + Duration::days(5)),
            ..op()
        }; // fim no futuro vira hoje
        let destino = faltantes::executar(&d.amb, &pl, None, &comum::cred(), o)
            .unwrap()
            .unwrap();
        let linhas = comum::valores(&destino, Some("Faltantes"));
        assert_eq!(
            linhas[1..]
                .iter()
                .map(|l| l[0].as_str())
                .collect::<Vec<_>>(),
            ["31.700/2026"]
        );
        assert_eq!(
            intervalos(&fake),
            HashSet::from([format!("{} - {}", br(hoje - Duration::days(10)), br(hoje))])
        );
    }

    #[test]
    fn faltantes_periodo_sem_processos() {
        // Os dois processos foram emitidos hoje. Com o período terminando
        // ontem nenhum entra, mesmo com o servidor ignorando o intervalo.
        let fake = comum::Fake1Doc::novo();
        let d = comum::dados(&fake.base);
        let pl = comum::planilha(&d.pasta_dados().join("acomp.xlsx"), &["31.632/2026"]);
        let ontem = comum::hoje() - Duration::days(1);
        let o = faltantes::Opcoes {
            de: Some(ontem - Duration::days(30)),
            ate: Some(ontem),
            ..op()
        };
        assert_eq!(
            faltantes::executar(&d.amb, &pl, None, &comum::cred(), o).unwrap(),
            None
        );
        assert_eq!(
            intervalos(&fake),
            HashSet::from([format!(
                "{} - {}",
                br(ontem - Duration::days(30)),
                br(ontem)
            )])
        );
        assert!(!fake.chaves().contains(&("GET".into(), "doc/ver".into()))); // nenhum processo aberto
        assert!(
            d.saida_texto()
                .contains("Nenhum Proc. Administrativo emitido de")
        );
        assert_eq!(std::fs::read_dir(d.pasta_saida()).unwrap().count(), 0);
    }

    #[test]
    fn faltantes_todos_ja_na_planilha() {
        let fake = comum::Fake1Doc::novo();
        let d = comum::dados(&fake.base);
        let pl = comum::planilha(
            &d.pasta_dados().join("acomp.xlsx"),
            &["31.632/2026", "31.700/2026"],
        );
        assert_eq!(
            faltantes::executar(&d.amb, &pl, None, &comum::cred(), op()).unwrap(),
            None
        );
        assert!(d.saida_texto().contains("já estão na planilha"));
    }

    #[test]
    fn faltantes_periodo_vazio() {
        // Só a data final, antes do início padrão (1º/jan de 2026, pela planilha).
        let fake = comum::Fake1Doc::novo();
        let d = comum::dados(&fake.base);
        let pl = comum::planilha(&d.pasta_dados().join("acomp.xlsx"), &["31.632/2026"]);
        let o = faltantes::Opcoes {
            ate: chrono::NaiveDate::from_ymd_opt(2025, 12, 31),
            ..op()
        };
        let e = faltantes::executar(&d.amb, &pl, None, &comum::cred(), o).unwrap_err();
        assert!(
            matches!(&e, Erro::Encerrar(m) if m.contains("Período vazio")),
            "{e:?}"
        );
    }

    #[test]
    fn planilha_inexistente() {
        let fake = comum::Fake1Doc::novo();
        let d = comum::dados(&fake.base);
        let e = faltantes::executar(
            &d.amb,
            &d.pasta_dados().join("nao_existe.xlsx"),
            None,
            &comum::cred(),
            op(),
        )
        .unwrap_err();
        assert!(
            matches!(&e, Erro::Encerrar(m) if m.contains("não encontrado")),
            "{e:?}"
        );
    }
}
