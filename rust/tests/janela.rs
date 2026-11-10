//! A janela de verdade (o estado e o desenho egui), rodando uma ferramenta
//! contra o 1Doc de mentira (porta de tests/test_janela.py).

mod comum;

use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{Duration as Dias, NaiveDate};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ferramentas1doc::gui::Janela;
use ferramentas1doc::gui::calendario::{self as cal, Calendario, Chave, Resultado, Vista};
use ferramentas1doc::{busca_argv, config, datas, sessao};

fn d(a: i32, m: u32, dia: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(a, m, dia).unwrap()
}

/// Janela com o login falso: avisa a espera e devolve a credencial, como o
/// obter_sessao real faz.
fn janela(fake: &comum::Fake1Doc) -> (comum::Dados, Janela) {
    let dd = comum::dados(&fake.base);
    let mut j = Janela::nova(&dd.amb, None);
    j.obter_sessao = Arc::new(|amb, ao_esperar| {
        ao_esperar(true);
        ao_esperar(false);
        let c = comum::cred();
        sessao::salvar(amb, &c).unwrap();
        Ok(c)
    });
    (dd, j)
}

fn esperar_fim(j: &mut Janela) {
    let fim = Instant::now() + Duration::from_secs(30);
    while j.executando.is_some() && Instant::now() < fim {
        j.consumir();
        std::thread::sleep(Duration::from_millis(20));
    }
    j.consumir();
}

#[test]
fn executar_processos_do_dia_pela_janela() {
    let fake = comum::Fake1Doc::novo();
    let (dd, mut j) = janela(&fake);
    j.processos.dia.texto = "hoje".into();
    // a pausa entre requisições do módulo é a padrão; o 1Doc falso é local
    j.executar(0);
    assert!(j.executando.is_some()); // bloqueado enquanto roda
    esperar_fim(&mut j);
    let texto = j.mensagens.texto.clone();
    assert_eq!(j.estado.0, "Concluído.", "{texto}");
    assert!(texto.contains("Processos exportados: 1"), "{texto}");
    assert_eq!(
        j.ultimo_arquivo.as_deref(),
        Some(
            dd.pasta_saida()
                .join(format!(
                    "processos novos {}.xlsx",
                    comum::hoje().format("%d.%m")
                ))
                .as_path()
        )
    );
    assert!(j.sessao.0.contains("Conectado"), "{}", j.sessao.0);
    let logs: Vec<_> = std::fs::read_dir(dd.pasta_dados().join("logs"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(logs.len(), 1);
    assert!(
        std::fs::read_to_string(&logs[0])
            .unwrap()
            .contains("Processos exportados: 1")
    );
}

#[test]
fn data_invalida_avisa_sem_executar() {
    let fake = comum::Fake1Doc::novo();
    let (_dd, mut j) = janela(&fake);
    j.processos.dia.texto = "31/02/2026".into();
    j.executar(0);
    assert!(j.executando.is_none());
    assert!(
        j.aviso
            .as_deref()
            .is_some_and(|a| a.contains("Data não reconhecida")),
        "{:?}",
        j.aviso
    );
}

#[test]
fn busca_com_formulario_invalido() {
    let fake = comum::Fake1Doc::novo();
    let (_dd, mut j) = janela(&fake);
    j.busca.caixas = [false; 4];
    j.executar(1);
    assert!(j.executando.is_none());
    assert!(
        j.aviso.as_deref().is_some_and(|a| a.contains("caixa")),
        "{:?}",
        j.aviso
    );
}

#[test]
fn busca_campos_de_etapa_com_lista() {
    let fake = comum::Fake1Doc::novo();
    let (dd, mut j) = janela(&fake);
    let det = serde_json::json!({"versao": "1.1", "det": {"3|1/2026": {"etapas_quadro": ["Elaboração do ETP", "Etapa Final"]}}});
    std::fs::write(dd.pasta_dados().join("1doc_cache.json"), det.to_string()).unwrap();
    let etapas = busca_argv::etapas_conhecidas(&j.busca.arquivo_cache);
    assert!(
        etapas.contains(&"Pesquisa de Mercado".to_string())
            && etapas.contains(&"Elaboração do ETP".to_string())
    );
    j.busca.escolher_etapa("etapa", "", "Pesquisa de Mercado");
    assert_eq!(j.busca.texto_mut("etapa"), "Pesquisa de Mercado");
    j.busca
        .escolher_etapa("etapa", "Pesquisa de Mercado; ", "Elaboração do ETP"); // etapa vinda do cache
    assert_eq!(
        j.busca.texto_mut("etapa"),
        "Pesquisa de Mercado; Elaboração do ETP"
    );
    j.busca
        .escolher_etapa("antes_de", "", "Pesquisa de Mercado");
    let form = j.busca.formulario();
    assert_eq!(
        form.texto("etapa"),
        "Pesquisa de Mercado; Elaboração do ETP"
    );
    assert_eq!(form.texto("antes_de"), "Pesquisa de Mercado");
}

#[test]
fn faltantes_periodo_invalido_avisa_sem_executar() {
    let fake = comum::Fake1Doc::novo();
    let (_dd, mut j) = janela(&fake);
    j.faltantes.de.texto = "31/02/2026".into();
    assert!(j.faltantes.leitura().0.contains("não reconhecida")); // aviso ao digitar
    j.executar(2);
    assert!(j.executando.is_none());
    assert!(
        j.aviso
            .as_deref()
            .is_some_and(|a| a.contains("Data inicial não reconhecida")),
        "{:?}",
        j.aviso
    );
}

#[test]
fn faltantes_periodo_vai_para_a_execucao_e_fica_salvo() {
    let fake = comum::Fake1Doc::novo();
    let (dd, mut j) = janela(&fake);
    let planilha = comum::planilha(&dd.pasta_dados().join("acomp.xlsx"), &["31.632/2026"]);
    j.faltantes.campos.planilha = planilha.to_string_lossy().into_owned();
    j.faltantes.de.texto = "-30".into();
    j.faltantes.ate.texto = "ontem".into();
    let hoje = comum::hoje();
    assert_eq!(
        j.faltantes.leitura().0,
        format!(
            "→ de {} até {}",
            datas::br(hoje - Dias::days(30)),
            datas::br(hoje - Dias::days(1))
        )
    );
    let t = j.tarefa_da_aba(2).unwrap();
    assert!(t.precisa_sessao);
    let cfg = config::carregar(&dd.amb);
    assert_eq!(
        (cfg.faltantes_de.as_str(), cfg.faltantes_ate.as_str()),
        ("-30", "ontem")
    ); // como digitado
    assert_eq!(cfg.planilha_faltantes, planilha.to_string_lossy());
    j.faltantes.de.texto.clear(); // vazio = padrão
    j.faltantes.ate.texto.clear();
    assert!(
        j.faltantes
            .leitura()
            .0
            .starts_with("→ de 1º/jan do ano mais antigo da planilha até hoje")
    );
}

#[test]
fn configuracoes_salvam_e_validam() {
    let fake = comum::Fake1Doc::novo();
    let (dd, mut j) = janela(&fake);
    j.abrir_configuracoes();
    j.abrir_configuracoes(); // uma só
    let c = j.configuracoes.as_mut().unwrap();
    c.minutos = "dez".into();
    assert_eq!(
        j.salvar_configuracoes().as_deref(),
        Some("Minutos: use um número inteiro.")
    );
    let c = j.configuracoes.as_mut().unwrap();
    c.minutos = "15".into();
    c.pasta = "/nao/existe".into();
    assert!(
        j.salvar_configuracoes()
            .unwrap()
            .contains("A pasta não existe")
    );
    let c = j.configuracoes.as_mut().unwrap();
    c.pasta = dd.pasta_saida().to_string_lossy().into_owned();
    c.navegador = "edge".into();
    assert_eq!(j.salvar_configuracoes(), None);
    assert!(j.configuracoes.is_none());
    let cfg = config::carregar(&dd.amb);
    assert_eq!((cfg.minutos_login, cfg.navegador.as_str()), (15, "edge"));
}

// ---------------------------------------------------------- desenho (egui_kittest)

fn harness(j: Janela) -> Harness<'static, Janela> {
    // no 1º quadro só instala as fontes do programa (valem a partir do seguinte)
    let mut primeiro = true;
    let mut h = Harness::builder()
        .with_size(egui::vec2(1100.0, 920.0))
        .build_ui_state(
            move |ui, j: &mut Janela| {
                if std::mem::take(&mut primeiro) {
                    ferramentas1doc::gui::configurar_fontes(ui.ctx());
                    return;
                }
                j.ui(ui)
            },
            j,
        );
    h.run();
    h
}

#[test]
fn janela_mostra_as_abas_e_a_busca_a_instrucao_de_datas() {
    let fake = comum::Fake1Doc::novo();
    let (_dd, j) = janela(&fake);
    let mut h = harness(j);
    h.run();
    for t in ferramentas1doc::gui::janela::TITULOS {
        let _ = h.get_by_label_contains(t);
    }
    let _ = h.get_by_label("Executar");
    h.get_by_label_contains("Busca").click();
    h.run();
    assert_eq!(h.state().aba_atual, 1);
    let _ = h.get_by_label_contains(busca_argv::DICA_DATAS.lines().next().unwrap()); // junto dos campos
    let _ = h.get_by_label("Ver todas as opções…");
}

#[test]
fn executar_pelo_botao_e_aviso_na_tela() {
    let fake = comum::Fake1Doc::novo();
    let (_dd, mut j) = janela(&fake);
    j.processos.dia.texto = "31/02/2026".into();
    let mut h = harness(j);
    h.run();
    h.get_by_label("Executar").click();
    h.run();
    assert!(h.state().aviso.is_some());
    let _ = h.get_by_label_contains("Data não reconhecida");
    h.get_by_label("OK").click();
    h.run();
    assert!(h.state().aviso.is_none());
}

// ---------------------------------------------------------- calendário

#[test]
fn calendario_semanas_e_leitura() {
    let sem = cal::semanas(2026, 11); // novembro de 2026 começa num domingo
    assert_eq!(sem.len(), 6);
    assert!(
        sem.iter()
            .all(|s| s.len() == 7 && chrono::Datelike::weekday(&s[0]) == chrono::Weekday::Sun)
    );
    assert_eq!(sem[0][0], d(2026, 11, 1));
    assert!(sem[4].contains(&d(2026, 11, 30)));
    let sem = cal::semanas(2026, 8); // agosto: começa no sábado, ocupa as 6 semanas
    assert_eq!((sem[0][0], sem[5][6]), (d(2026, 7, 26), d(2026, 9, 5)));
    let hoje = d(2026, 11, 6);
    for (txt, esperado) in [
        ("05/11/2026", Some(d(2026, 11, 5))),
        ("5/11/26", Some(d(2026, 11, 5))),
        ("2026-11-05", Some(d(2026, 11, 5))),
        ("hoje", Some(hoje)),
        ("ontem", Some(d(2026, 11, 5))),
        ("-7", Some(d(2026, 10, 30))),
        ("", None),
        ("31/02/2026", None),
        ("xyz", None),
    ] {
        assert_eq!(cal::ler_data_qualquer(txt, hoje), esperado, "{txt}");
    }
    assert_eq!(cal::anos_da_decada(2025), (2019..2031).collect::<Vec<_>>());
    assert_eq!(cal::anos_da_decada(2030)[1], 2030);
    assert_eq!(cal::mes_curto(3), "mar");
}

#[test]
fn calendario_escolhe_o_dia() {
    let hoje = d(2026, 11, 6);
    let mut c = Calendario::novo("05/03/2025", None, hoje);
    assert_eq!(
        (c.ano, c.mes, c.titulo().as_str()),
        (2025, 3, "março de 2025 ▾")
    );
    c.avancar(1);
    c.avancar(1);
    assert_eq!(c.titulo(), "maio de 2025 ▾");
    assert_eq!(
        c.escolher(d(2025, 5, 20)),
        Some(Resultado::Escolheu(d(2025, 5, 20)))
    );
    let mut campo = cal::CampoData::novo("20/05/2025", false);
    campo.abrir();
    assert_eq!(
        campo.calendario.as_ref().unwrap().escolhida,
        Some(d(2025, 5, 20))
    );
    campo.aplicar(Resultado::Fechou); // Esc / clique fora: fecha sem mudar
    assert_eq!(campo.texto, "20/05/2025");
    assert!(campo.calendario.is_none());
    campo.abrir();
    campo.aplicar(Resultado::Limpou);
    assert_eq!(campo.texto, "");
    campo.abrir();
    campo.aplicar(Resultado::Escolheu(d(2025, 5, 21)));
    assert_eq!(campo.texto, "21/05/2025");
}

#[test]
fn calendario_titulo_leva_aos_meses_e_aos_anos() {
    let mut c = Calendario::novo("05/03/2025", None, d(2026, 11, 6));
    c.subir(); // o título abre os meses do ano
    assert_eq!((c.vista, c.titulo().as_str()), (Vista::Meses, "2025 ▾"));
    let casas = c.casas();
    assert_eq!(casas.len(), 12);
    assert_eq!(
        (casas[0].0, casas[0].1.as_str()),
        (Chave::Mes(2025, 1), "jan")
    );
    c.subir(); // e o dele, os anos da década
    assert_eq!((c.vista, c.titulo().as_str()), (Vista::Anos, "2020–2029"));
    assert_eq!(c.casas()[0].0, Chave::Ano(2019));
    c.subir(); // acima dos anos não há nada
    assert_eq!(c.vista, Vista::Anos);
    c.avancar(-1);
    assert_eq!(c.titulo(), "2010–2019");
    c.entrar(Chave::Ano(2018)); // o ano abre os meses dele
    assert_eq!((c.vista, c.ano), (Vista::Meses, 2018));
    c.avancar(1);
    c.avancar(-1);
    assert_eq!(c.titulo(), "2018 ▾");
    c.entrar(Chave::Mes(2018, 8)); // e o mês, os dias dele
    assert_eq!(
        (c.vista, c.titulo().as_str()),
        (Vista::Dias, "agosto de 2018 ▾")
    );
    assert_eq!(c.foco, d(2018, 8, 5)); // só o dia escreve no campo
}

#[test]
fn calendario_teclado_nos_meses_e_anos() {
    let mut c = Calendario::novo("31/01/2024", None, d(2026, 11, 6));
    c.mover_meses(12); // Shift+PageDown: um ano à frente
    assert_eq!((c.foco, c.ano, c.mes), (d(2025, 1, 31), 2025, 1));
    c.mover_meses(c.pagina()); // PageDown: o mês (31/jan → 28/fev)
    assert_eq!(c.foco, d(2025, 2, 28));
    c.mover_meses(-12);
    assert_eq!(c.foco, d(2024, 2, 28));
    c.subir();
    c.seta(1); // nos meses, as setas andam de mês
    c.seta(7); // e de linha (4 meses)
    assert_eq!((c.foco, c.vista), (d(2024, 7, 28), Vista::Meses));
    c.mover_meses(c.pagina()); // PageDown: o ano
    assert_eq!((c.foco, c.titulo().as_str()), (d(2025, 7, 28), "2025 ▾"));
    c.subir();
    c.seta(-1); // nos anos, de ano
    assert_eq!((c.foco, c.vista), (d(2024, 7, 28), Vista::Anos));
    assert_eq!(c.confirmar(), None); // Enter abre o ano...
    assert_eq!((c.vista, c.ano), (Vista::Meses, 2024));
    assert_eq!(c.confirmar(), None); // ...e o mês
    assert_eq!((c.vista, c.ano, c.mes), (Vista::Dias, 2024, 7));
    assert_eq!(c.confirmar(), Some(Resultado::Escolheu(d(2024, 7, 28)))); // e escolhe o dia
}

#[test]
fn calendario_meses_e_anos_respeitam_o_maximo() {
    let hoje = d(2026, 6, 15);
    let mut c = Calendario::novo("", Some(hoje), hoje);
    c.subir();
    assert!(c.ao_lado(1).is_none() && c.ao_lado(-1).is_some());
    c.entrar(Chave::Mes(2026, 7)); // o mês que vem não se abre
    assert_eq!(c.vista, Vista::Meses);
    assert!(c.fora(Chave::Mes(2026, 12)));
    c.subir();
    assert!(c.ao_lado(1).is_none()); // a década que vem, toda no futuro
    c.entrar(Chave::Ano(2027)); // nem o ano que vem
    assert_eq!(c.vista, Vista::Anos);
    c.mover_meses(12); // nem pelo teclado
    assert!(c.foco <= hoje);
    c.entrar(Chave::Ano(2026));
    c.entrar(Chave::Mes(2026, 6));
    assert_eq!((c.vista, c.ano, c.mes), (Vista::Dias, 2026, 6));
    assert!(c.foco <= hoje);
}

#[test]
fn calendario_nao_deixa_escolher_o_futuro_onde_ele_nao_vale() {
    let hoje = d(2026, 6, 15);
    let c = Calendario::novo("", Some(hoje), hoje);
    assert_eq!((c.ano, c.mes), (2026, 6));
    assert_eq!(c.escolher(hoje + Dias::days(1)), None); // amanhã não se escolhe
    assert!(c.ao_lado(1).is_none());
    assert_eq!(c.escolher(hoje), Some(Resultado::Escolheu(hoje)));
    let ate = Calendario::novo("", None, hoje); // Faltantes, "até": o futuro vale (vira hoje)
    assert!(ate.ao_lado(1).is_some());
}
