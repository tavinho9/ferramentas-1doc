//! `Ferramentas1Doc --autoteste`: confere o pacote e sai (0 = tudo certo).
//! Os mesmos itens do autoteste da versão Python.

use std::panic::AssertUnwindSafe;

use crate::ambiente::Ambiente;
use crate::saida::Saida;
use crate::{VERSAO, busca_argv, navegador, processos_do_dia, sessao};

pub fn rodar(relatorio: Option<&str>, com_navegador: bool, com_janela: bool) -> i32 {
    let mut linhas: Vec<String> = Vec::new();
    let mut falhas = 0;
    let mut item = |nome: &str, f: &dyn Fn() -> Result<String, String>| {
        let r = std::panic::catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|p| {
            Err(p
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_else(|| "pânico".into()))
        });
        match r {
            Ok(d) if d.is_empty() => linhas.push(format!("OK     {nome}")),
            Ok(d) => linhas.push(format!("OK     {nome} — {d}")),
            Err(e) => {
                falhas += 1;
                linhas.push(format!("FALHA  {nome} — {e}"));
            }
        }
    };
    let amb = Ambiente::padrao(Saida::nada());

    item("módulos", &|| {
        Ok(format!(
            "versão {VERSAO}, dados em {}",
            amb.pasta_dados().display()
        ))
    });
    item("leitura de HTML", &|| {
        let html = concat!(
            r#"<div id="emissao_principal"><div class="kanban_estagios" data-current="true">"#,
            r#"<div class="kanban_titulo"><p>Pesquisa de Mercado</p></div><div class="kanban_itens">"#,
            r#"<p class="kanban_item">Esta aqui 2 horas</p></div></div></div>"#
        );
        match processos_do_dia::ler_etapa_atual(html) {
            Some((e, t)) if e == "Pesquisa de Mercado" && t == "Esta aqui 2 horas" => {
                Ok("parser html5ever (scraper)".into())
            }
            outro => Err(format!("lido {outro:?}")),
        }
    });
    item("argumentos da busca", &|| {
        let form = busca_argv::Formulario {
            caixas: vec!["entrada".into()],
            ..Default::default()
        }
        .com("etapa", "Pesquisa de Mercado")
        .com("emissao_de", "-30");
        let argv = busca_argv::montar_argv(&form, None, chrono::Local::now().naive_local(), false)
            .map_err(|e| e.0)?;
        match busca_argv::validar(&argv) {
            None => Ok(argv.join(" ")),
            Some(e) => Err(e),
        }
    });
    item("planilha .xlsx", &|| {
        let pasta = tempfile::tempdir().map_err(|e| e.to_string())?;
        let caminho = pasta.path().join("teste.xlsx");
        let mut wb = rust_xlsxwriter::Workbook::new();
        wb.add_worksheet()
            .write_string(0, 0, "Nº Processo")
            .map_err(|e| e.to_string())?;
        wb.save(&caminho).map_err(|e| e.to_string())?;
        let lido = umya_spreadsheet::reader::xlsx::read(&caminho).map_err(|e| e.to_string())?;
        let v = lido.sheet(0).map(|w| w.value("A1")).unwrap_or_default();
        if v == "Nº Processo" {
            Ok(String::new())
        } else {
            Err(format!("lido {v:?}"))
        }
    });
    item("Chrome/Edge instalado", &|| {
        let achados: Vec<(String, Option<std::path::PathBuf>)> = ["chrome", "edge"]
            .iter()
            .map(|q| (navegador::nome(q), navegador::localizar_executavel(q)))
            .collect();
        let texto = achados
            .iter()
            .map(|(n, c)| {
                format!(
                    "{n}: {}",
                    c.as_ref()
                        .map(|c| c.display().to_string())
                        .unwrap_or_else(|| "não encontrado".into())
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        if achados.iter().any(|(_, c)| c.is_some()) {
            Ok(texto)
        } else {
            Err("nem Chrome nem Edge encontrados".into())
        }
    });
    if com_navegador {
        item("navegador (invisível) controlado sem driver", &|| {
            let qual = std::env::var("AUTOTESTE_NAVEGADOR").unwrap_or_else(|_| "auto".into());
            let (mut drv, nome) =
                sessao::abrir_navegador(&amb, &qual, true, false).map_err(|e| e.to_string())?;
            let r = (|| {
                drv.ir(
                    "data:text/html,<title>ok</title><input id='sis_usr_email' value='x'>",
                    None,
                )
                .map_err(|e| e.to_string())?;
                let aba = drv.pagina().map_err(|e| e.to_string())?;
                for _ in 0..50 {
                    if drv
                        .avaliar(&aba, "document.title")
                        .ok()
                        .and_then(|v| v.as_str().map(str::to_string))
                        .as_deref()
                        == Some("ok")
                    {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                let v = drv
                    .avaliar(&aba, "document.getElementById('sis_usr_email').value")
                    .map_err(|e| e.to_string())?;
                if v != "x" {
                    return Err(format!("lido {v}"));
                }
                Ok(format!("{nome} {}", drv.versao().unwrap_or_default()))
            })();
            drv.fechar();
            r
        });
    }
    item("janela (desenho sem tela)", &|| {
        let mut h = sem_tela::Harness::nova(&amb);
        h.rodar();
        Ok(crate::gui::janela::TITULOS.join(", "))
    });
    if com_janela {
        item("janela principal", &|| janela_de_verdade(&amb));
    }
    linhas.push(format!(
        "\n{}",
        if falhas == 0 {
            "TUDO CERTO".to_string()
        } else {
            format!("{falhas} FALHA(S)")
        }
    ));
    let texto = linhas.join("\n") + "\n";
    if let Some(r) = relatorio {
        let _ = std::fs::write(r, &texto);
    }
    print!("{texto}");
    if falhas > 0 { 1 } else { 0 }
}

/// Abre a janela de verdade por alguns quadros e a fecha.
fn janela_de_verdade(amb: &Ambiente) -> Result<String, String> {
    struct Fecha(
        crate::gui::Janela,
        u32,
        std::sync::Arc<std::sync::Mutex<String>>,
    );
    impl eframe::App for Fecha {
        fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
            self.0.ui(ui);
            self.1 += 1;
            if self.1 == 1 {
                let info = frame
                    .wgpu_render_state()
                    .map(|r| format!("wgpu {:?}", r.adapter.get_info().backend))
                    .unwrap_or_else(|| "OpenGL".into());
                *self.2.lock().unwrap() = info;
            }
            ui.ctx().request_repaint();
            if self.1 >= 20 {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }
    let info = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let (amb, i2) = (amb.clone(), info.clone());
    crate::gui::rodar_com(&move |cc| {
        crate::gui::configurar_fontes(&cc.egui_ctx);
        Box::new(Fecha(crate::gui::Janela::nova(&amb, None), 0, i2.clone()))
    })
    .map_err(|e| e.to_string())?;
    let d = info.lock().unwrap().clone();
    Ok(format!(
        "{}; desenho {d}",
        crate::gui::janela::TITULOS.join(", ")
    ))
}

/// Monta a janela de verdade num contexto egui sem tela (desenha alguns
/// quadros), para conferir que ela abre sem erro.
mod sem_tela {
    use crate::ambiente::Ambiente;
    use crate::gui::Janela;

    pub struct Harness {
        ctx: egui::Context,
        janela: Janela,
    }

    impl Harness {
        pub fn nova(amb: &Ambiente) -> Self {
            let ctx = egui::Context::default();
            crate::gui::configurar_fontes(&ctx);
            Harness {
                ctx,
                janela: Janela::nova(amb, None),
            }
        }

        pub fn rodar(&mut self) {
            for _ in 0..3 {
                let entrada = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1100.0, 920.0),
                    )),
                    ..Default::default()
                };
                let janela = &mut self.janela;
                let mut saida = self.ctx.run_ui(entrada, |ui| janela.ui(ui));
                saida.textures_delta.clear();
            }
        }
    }
}
