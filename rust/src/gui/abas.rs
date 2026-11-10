//! As quatro abas do programa, uma por ferramenta. Cada aba só monta os
//! parâmetros e devolve uma tarefa (título, função e se precisa de sessão);
//! quem executa (thread, login, mensagens) é a janela principal.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{Local, NaiveDate};
use egui::{Color32, RichText};

use super::calendario::CampoData;
use crate::ambiente::Ambiente;
use crate::erro::Res;
use crate::sessao::Credenciais;
use crate::{
    busca, busca_argv, config, datas, faltantes, msg, processos_do_dia, verificadores,
    verificar_planilha,
};

pub const COR_ERRO: Color32 = Color32::from_rgb(0xb0, 0x00, 0x20);
pub const COR_DICA: Color32 = Color32::from_rgb(0x77, 0x77, 0x77);
pub const COR_LEITURA: Color32 = Color32::from_rgb(0x55, 0x55, 0x55);

pub type Funcao = Box<dyn FnOnce(&Ambiente, Option<&Credenciais>) -> Res<Option<PathBuf>> + Send>;

pub struct Tarefa {
    pub titulo: String,
    pub funcao: Option<Funcao>,
    pub precisa_sessao: bool,
}

fn descricao(ui: &mut egui::Ui, texto: &str) {
    ui.add(
        egui::Label::new(RichText::new(texto).color(Color32::from_rgb(0x44, 0x44, 0x44))).wrap(),
    );
    ui.add_space(6.0);
}

fn caixas(ui: &mut egui::Ui, rotulos: &[(&str, &str)], marcadas: &mut [bool]) {
    ui.horizontal(|ui| {
        for (i, (_, rot)) in rotulos.iter().enumerate() {
            ui.checkbox(&mut marcadas[i], *rot);
            ui.add_space(8.0);
        }
    });
}

fn dica(ui: &mut egui::Ui, t: &str) {
    ui.label(RichText::new(t).color(COR_DICA));
}

// ============================================================
// 1. Processos do dia
// ============================================================

pub struct AbaProcessosDoDia {
    pub dia: CampoData,
    pub caixas: [bool; 4],
}

impl Default for AbaProcessosDoDia {
    fn default() -> Self {
        AbaProcessosDoDia {
            dia: CampoData::novo(&datas::br(datas::hoje()), true),
            caixas: [true; 4],
        }
    }
}

impl AbaProcessosDoDia {
    pub const TITULO: &'static str = "Processos do dia";

    pub fn interpretar(&self) -> Option<NaiveDate> {
        processos_do_dia::ler_dia(&self.dia.texto, datas::hoje())
    }

    /// O que aparece ao lado do campo: (texto, é erro?).
    pub fn leitura(&self) -> (String, bool) {
        match self.interpretar() {
            None => ("data não reconhecida".into(), true),
            Some(d) if d > datas::hoje() => (format!("{} ainda não chegou", datas::br(d)), true),
            Some(d) => (
                format!("→ {}, {}", datas::dia_semana(d), datas::br(d)),
                false,
            ),
        }
    }

    pub fn tarefa(&self) -> Result<Tarefa, String> {
        let alvo = self
            .interpretar()
            .ok_or("Data não reconhecida. Use DD/MM/AAAA, DD/MM/AA, DD/MM ou 'ontem'.")?;
        if alvo > datas::hoje() {
            return Err(format!("{} ainda não chegou.", datas::br(alvo)));
        }
        let caixas: Vec<(String, String)> = processos_do_dia::CAIXAS
            .iter()
            .zip(self.caixas)
            .filter(|(_, m)| *m)
            .map(|((c, r), _)| (c.to_string(), r.to_string()))
            .collect();
        if caixas.is_empty() {
            return Err("Marque ao menos uma caixa.".into());
        }
        Ok(Tarefa {
            titulo: format!("Processos do dia {}", datas::br(alvo)),
            funcao: Some(Box::new(move |amb, cred| {
                let op = processos_do_dia::Opcoes {
                    caixas: Some(caixas),
                    ..Default::default()
                };
                processos_do_dia::executar(amb, alvo, cred.expect("sessão"), op)
            })),
            precisa_sessao: true,
        })
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        descricao(
            ui,
            "Processos emitidos num dia, nas caixas do setor, com a etapa atual do quadro. Gera a planilha \"processos novos DD.MM.xlsx\" e o processos_do_dia.json. Para hoje, entram só os processos cuja etapa atual tem menos de 24h; para um dia passado, entram com a etapa atual, qualquer que seja o tempo nela.",
        );
        ui.horizontal(|ui| {
            let l = ui.label("Dia de emissão:");
            self.dia.ui(ui, 100.0, "dia_processos").labelled_by(l.id);
            if ui.button("Hoje").clicked() {
                self.dia.texto = datas::br(datas::hoje());
            }
            if ui.button("Ontem").clicked() {
                self.dia.texto = datas::br(datas::ontem(datas::hoje()));
            }
            let (t, erro) = self.leitura();
            ui.label(RichText::new(t).color(if erro { COR_ERRO } else { COR_LEITURA }));
        });
        dica(ui, "DD/MM/AAAA, DD/MM/AA, DD/MM ou \"ontem\"; vazio = hoje");
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label("Caixas:");
            caixas(ui, &processos_do_dia::CAIXAS, &mut self.caixas);
        });
    }
}

// ============================================================
// 2. Busca
// ============================================================

pub struct AbaBusca {
    pub textos: HashMap<String, String>,
    pub datas: HashMap<String, CampoData>,
    pub caixas: [bool; 4],
    pub prioridades: [bool; 4],
    pub marcados: HashMap<String, bool>,
    /// lista de etapas aberta: (campo, texto antes de abrir)
    pub lista_aberta: Option<(String, String)>,
    pub ajuda_aberta: bool,
    pub arquivo_cache: PathBuf,
}

const CAMPOS_DATA: [&str; 6] = [
    "emissao_de",
    "emissao_ate",
    "mudanca_de",
    "mudanca_ate",
    "entrada_de",
    "entrada_ate",
];

impl AbaBusca {
    pub const TITULO: &'static str = "Busca";

    pub fn nova(amb: &Ambiente) -> Self {
        AbaBusca {
            textos: HashMap::new(),
            datas: CAMPOS_DATA
                .iter()
                .map(|c| (c.to_string(), CampoData::novo("", false)))
                .collect(),
            caixas: [true, false, false, false],
            prioridades: [false; 4],
            marcados: HashMap::new(),
            lista_aberta: None,
            ajuda_aberta: false,
            arquivo_cache: amb.arquivo_dados("1doc_cache.json"),
        }
    }

    pub fn texto_mut(&mut self, k: &str) -> &mut String {
        if CAMPOS_DATA.contains(&k) {
            return &mut self.datas.get_mut(k).unwrap().texto;
        }
        self.textos.entry(k.to_string()).or_default()
    }

    pub fn formulario(&self) -> busca_argv::Formulario {
        let mut textos = self.textos.clone();
        for (k, d) in &self.datas {
            textos.insert(k.clone(), d.texto.clone());
        }
        busca_argv::Formulario {
            textos,
            caixas: busca_argv::CAIXAS
                .iter()
                .zip(self.caixas)
                .filter(|(_, m)| *m)
                .map(|(c, _)| c.to_string())
                .collect(),
            prioridade: busca_argv::PRIORIDADES
                .iter()
                .zip(self.prioridades)
                .filter(|(_, m)| *m)
                .map(|(p, _)| p.to_string())
                .collect(),
            marcados: self
                .marcados
                .iter()
                .filter(|(_, v)| **v)
                .map(|(k, _)| k.clone())
                .collect(),
        }
    }

    /// Escolheu `etapa` na lista do campo `campo` (junta com ';' quando cabe).
    pub fn escolher_etapa(&mut self, campo: &str, antes: &str, etapa: &str) {
        let varios = busca_argv::TEXTOS_MULTIPLOS
            .iter()
            .any(|(c, _)| *c == campo);
        *self.texto_mut(campo) = busca_argv::juntar_etapa(antes, etapa, varios);
    }

    pub fn tarefa(&self, amb: &Ambiente) -> Result<Tarefa, String> {
        let cfg = config::carregar(amb);
        let pasta = config::pasta_saida(amb, &cfg);
        let argv = busca_argv::montar_argv(
            &self.formulario(),
            Some(&pasta),
            Local::now().naive_local(),
            cfg.abrir_planilha,
        )
        .map_err(|e| e.0)?;
        if let Some(erro) = busca_argv::validar(&argv) {
            return Err(format!("Opções inválidas: {erro}"));
        }
        let offline = argv.iter().any(|a| a == "--offline");
        Ok(Tarefa {
            titulo: "Busca".into(),
            funcao: Some(Box::new(move |amb, cred| {
                let opcoes: Vec<String> = argv
                    .iter()
                    .map(|a| {
                        if a.contains(' ') {
                            format!("\"{a}\"")
                        } else {
                            a.clone()
                        }
                    })
                    .collect();
                msg!(amb, "Opções: {}", opcoes.join(" "));
                Ok(busca::main(amb, &argv, cred)?.and_then(|r| r.arquivo))
            })),
            precisa_sessao: !offline,
        })
    }

    fn campo(&mut self, ui: &mut egui::Ui, chave: &str, rotulo: &str, largura: f32) {
        let l = ui.label(rotulo);
        ui.add_sized(
            [largura, 20.0],
            egui::TextEdit::singleline(self.texto_mut(chave)).id_salt(chave),
        )
        .labelled_by(l.id);
    }

    /// Linha de período: rótulo | [de] até: [até].
    fn campos_data(&mut self, ui: &mut egui::Ui, de: &str, ate: &str, rotulo: &str) {
        let l = ui.label(rotulo);
        ui.horizontal(|ui| {
            self.datas
                .get_mut(de)
                .unwrap()
                .ui(ui, 80.0, de)
                .labelled_by(l.id);
            let l2 = ui.label("até:");
            self.datas
                .get_mut(ate)
                .unwrap()
                .ui(ui, 80.0, ate)
                .labelled_by(l2.id);
        });
    }

    /// Campo de etapa: aceita texto (parte do nome, várias com ';') e tem
    /// uma lista com as etapas conhecidas.
    fn campo_etapa(&mut self, ui: &mut egui::Ui, chave: &str, rotulo: &str) {
        let l = ui.label(rotulo);
        ui.horizontal(|ui| {
            let campo = ui
                .add(
                    egui::TextEdit::singleline(self.texto_mut(chave))
                        .desired_width(190.0)
                        .id_salt(chave),
                )
                .labelled_by(l.id);
            let botao = ui
                .add(egui::Button::new("▾").small())
                .on_hover_text("Escolher a etapa na lista");
            if botao.clicked() {
                let antes = self.texto_mut(chave).clone();
                self.lista_aberta = if self.lista_aberta.as_ref().is_some_and(|l| l.0 == chave) {
                    None
                } else {
                    Some((chave.to_string(), antes))
                };
            }
            if self.lista_aberta.as_ref().is_some_and(|l| l.0 == chave) {
                let etapas = busca_argv::etapas_conhecidas(&self.arquivo_cache);
                let area = egui::Area::new(egui::Id::unique(("lista_etapas", chave)))
                    .order(egui::Order::Foreground)
                    .fixed_pos(campo.rect.left_bottom() + egui::vec2(0.0, 2.0))
                    .show(ui.ctx(), |ui| {
                        egui::Frame::popup(ui.style())
                            .show(ui, |ui| {
                                let mut escolhida = None;
                                egui::ScrollArea::vertical()
                                    .max_height(380.0)
                                    .show(ui, |ui| {
                                        for e in &etapas {
                                            if ui.selectable_label(false, e).clicked() {
                                                escolhida = Some(e.clone());
                                            }
                                        }
                                    });
                                escolhida
                            })
                            .inner
                    });
                if let Some(e) = area.inner {
                    let antes = self.lista_aberta.take().map(|l| l.1).unwrap_or_default();
                    self.escolher_etapa(chave, &antes, &e);
                } else if ui.input(|i| i.key_pressed(egui::Key::Escape))
                    || ui.input(|i| {
                        i.pointer.any_pressed()
                            && i.pointer.interact_pos().is_some_and(|p| {
                                !area.response.rect.contains(p) && !botao.rect.contains(p)
                            })
                    })
                {
                    self.lista_aberta = None;
                }
            }
        });
    }

    fn marcar(&mut self, ui: &mut egui::Ui, chave: &str, rotulo: &str) {
        let v = self.marcados.entry(chave.to_string()).or_insert(false);
        ui.checkbox(v, rotulo);
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        descricao(
            ui,
            "Busca nas caixas do setor por etapa, fase, datas e mudanças de etapa. Gera uma planilha com as abas Resultado, Transições e Parâmetros. Textos: sem diferença de maiúsculas e acentos, \"contém\"; vários valores num campo, separados por ';', valem como OU. Etapas: escolha na lista (ou digite parte do nome); para juntar outra, digite ';' depois da primeira e escolha de novo. A lista traz o fluxo da Abertura e as etapas que as buscas já encontraram.",
        );
        egui::CollapsingHeader::new("O que ler")
            .default_open(true)
            .show(ui, |ui| {
                caixas(ui, &busca::CAIXAS, &mut self.caixas);
                egui::Grid::new("busca_universo")
                    .num_columns(3)
                    .spacing([8.0, 4.0])
                    .show(ui, |ui| {
                        self.campo(ui, "numeros", "Só estes números:", 240.0);
                        dica(ui, "ex.: 31.632/2026, 5712/2025 (no lugar das caixas)");
                        ui.end_row();
                        let l = ui.label("Ou números de um arquivo:");
                        ui.add_sized(
                            [340.0, 20.0],
                            egui::TextEdit::singleline(self.texto_mut("numeros_arquivo")),
                        )
                        .labelled_by(l.id);
                        if ui.button("Escolher…").clicked()
                            && let Some(c) = rfd::FileDialog::new()
                                .set_title("Arquivo com números de processo")
                                .add_filter("Planilha ou texto", &["xlsx", "xlsm", "txt", "csv"])
                                .pick_file()
                        {
                            *self.texto_mut("numeros_arquivo") = c.to_string_lossy().into_owned();
                        }
                        ui.end_row();
                    });
            });
        ui.columns(2, |cols| {
            egui::CollapsingHeader::new("Etapa e posição")
                .default_open(true)
                .show(&mut cols[0], |ui| {
                    egui::Grid::new("busca_etapa")
                        .num_columns(2)
                        .spacing([8.0, 4.0])
                        .show(ui, |ui| {
                            self.campo_etapa(ui, "etapa", "Etapa atual contém:");
                            ui.end_row();
                            ui.label("");
                            self.marcar(ui, "etapa_exata", "nome exato");
                            ui.end_row();
                            self.campo_etapa(ui, "excluir_etapa", "Exceto a etapa:");
                            ui.end_row();
                            let l = ui.label("Fase:");
                            let fase = self.texto_mut("fase");
                            egui::ComboBox::from_id_salt("fase")
                                .selected_text(if fase.is_empty() {
                                    "(qualquer)".to_string()
                                } else {
                                    fase.clone()
                                })
                                .width(120.0)
                                .show_ui(ui, |ui| {
                                    for f in ["", "Elaboração", "Abertura"] {
                                        ui.selectable_value(
                                            fase,
                                            f.to_string(),
                                            if f.is_empty() { "(qualquer)" } else { f },
                                        );
                                    }
                                })
                                .response
                                .labelled_by(l.id);
                            ui.end_row();
                            self.campo_etapa(ui, "antes_de", "Ainda não chegou a:");
                            ui.end_row();
                            self.campo_etapa(ui, "a_partir_de", "Já chegou a:");
                            ui.end_row();
                            self.campo_etapa(ui, "passou_por", "Já passou por:");
                            ui.end_row();
                            self.campo(ui, "dias_etapa_min", "Dias na etapa, mín.:", 50.0);
                            ui.end_row();
                            self.campo(ui, "dias_etapa_max", "Dias na etapa, máx.:", 50.0);
                            ui.end_row();
                        });
                });
            egui::CollapsingHeader::new("Datas e mudanças de etapa")
                .default_open(true)
                .show(&mut cols[1], |ui| {
                    egui::Grid::new("busca_datas")
                        .num_columns(2)
                        .spacing([6.0, 4.0])
                        .show(ui, |ui| {
                            self.campos_data(ui, "emissao_de", "emissao_ate", "Emitido de:");
                            ui.end_row();
                            self.campos_data(ui, "mudanca_de", "mudanca_ate", "Mudou de etapa de:");
                            ui.end_row();
                            self.campo_etapa(ui, "mudanca_para", "…para a etapa:");
                            ui.end_row();
                            self.campo_etapa(ui, "mudanca_origem", "…saindo da etapa:");
                            ui.end_row();
                            self.campos_data(
                                ui,
                                "entrada_de",
                                "entrada_ate",
                                "Entrou na etapa atual de:",
                            );
                            ui.end_row();
                            self.campo(ui, "sem_mov_min", "Parado há (dias), mín.:", 50.0);
                            ui.end_row();
                        });
                    dica(ui, busca_argv::DICA_DATAS);
                });
        });
        egui::CollapsingHeader::new("Outros")
            .default_open(true)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Prioridade:");
                    let rot: Vec<(&str, &str)> =
                        busca_argv::PRIORIDADES.iter().map(|p| (*p, *p)).collect();
                    caixas(ui, &rot, &mut self.prioridades);
                });
                ui.horizontal(|ui| self.campo(ui, "objeto", "Objeto contém:", 320.0));
                ui.horizontal(|ui| {
                    self.marcar(ui, "csv", "Gerar também CSV");
                    self.marcar(
                        ui,
                        "refazer",
                        "Reabrir todos os processos (ignorar o cache)",
                    );
                    self.marcar(
                        ui,
                        "offline",
                        "Sem acessar o 1Doc (só o cache da última busca)",
                    );
                });
                ui.horizontal(|ui| {
                    self.campo(ui, "extras", "Outras opções:", 480.0);
                    if ui.button("Ver todas as opções…").clicked() {
                        self.ajuda_aberta = true;
                    }
                });
            });
    }

    /// Texto da janela "Opções da busca".
    pub fn texto_ajuda() -> String {
        format!(
            "No campo \"Outras opções\", escreva as opções como abaixo, por exemplo:\n   --setor-etapa CPPFI --ordenar prioridade,dias_etapa:desc\nTextos com espaço vão entre aspas: --cita-setor \"SPOT-DCL-GP-CPPFI\"\n\n{}",
            busca::args::ajuda()
        )
    }
}

// ============================================================
// 3 e 4. Planilha de acompanhamento
// ============================================================

/// Campos comuns das duas abas de planilha.
pub struct CamposPlanilha {
    pub planilha: String,
    pub aba: String,
    pub abas: Vec<String>,
    lidas_de: Option<(String, Option<std::time::SystemTime>)>,
}

impl CamposPlanilha {
    pub fn nova(planilha: &str) -> Self {
        let mut c = CamposPlanilha {
            planilha: planilha.into(),
            aba: String::new(),
            abas: Vec::new(),
            lidas_de: None,
        };
        c.ler_abas();
        c
    }

    fn caminho(&self) -> String {
        self.planilha.trim().trim_matches('"').to_string()
    }

    /// Lê as abas da planilha (só quando o arquivo muda).
    pub fn ler_abas(&mut self) {
        let caminho = self.caminho();
        let marca = (
            caminho.clone(),
            std::fs::metadata(&caminho)
                .ok()
                .and_then(|m| m.modified().ok()),
        );
        if self.lidas_de.as_ref() == Some(&marca) {
            return;
        }
        self.lidas_de = Some(marca);
        let (mut nomes, mut ativa) = (Vec::new(), String::new());
        if !caminho.is_empty()
            && Path::new(&caminho).is_file()
            && let Ok(n) = verificadores::abas(Path::new(&caminho))
        {
            nomes = n;
            ativa = umya_spreadsheet::reader::xlsx::lazy_read(Path::new(&caminho))
                .ok()
                .map(|wb| wb.active_sheet().name().to_string())
                .filter(|a| !a.is_empty())
                .unwrap_or_else(|| nomes.first().cloned().unwrap_or_default());
        }
        if !nomes.contains(&self.aba) {
            self.aba = if nomes.is_empty() {
                String::new()
            } else {
                ativa
            };
        }
        self.abas = nomes;
    }

    /// Confere o arquivo e guarda o caminho na config (`chave`).
    pub fn validar(
        &self,
        amb: &Ambiente,
        chave: &str,
    ) -> Result<(PathBuf, Option<String>), String> {
        let caminho = self.caminho();
        if caminho.is_empty() {
            return Err("Escolha a planilha.".into());
        }
        if !Path::new(&caminho).is_file() {
            return Err(format!("Arquivo não encontrado:\n{caminho}"));
        }
        let mut cfg = config::carregar(amb);
        match chave {
            "planilha_faltantes" => cfg.planilha_faltantes = caminho.clone(),
            _ => cfg.planilha_verificar = caminho.clone(),
        }
        let _ = config::salvar(amb, &cfg);
        Ok((
            PathBuf::from(caminho),
            Some(self.aba.clone()).filter(|a| !a.is_empty()),
        ))
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, amb: &Ambiente) {
        egui::Grid::new(("planilha", self as *const _ as usize))
            .num_columns(3)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                let l = ui.label("Planilha:");
                ui.add_sized(
                    [560.0, 20.0],
                    egui::TextEdit::singleline(&mut self.planilha),
                )
                .labelled_by(l.id);
                if ui.button("Escolher…").clicked() {
                    let inicial = Path::new(&self.caminho())
                        .parent()
                        .map(Path::to_path_buf)
                        .filter(|p| p.is_dir())
                        .unwrap_or_else(|| amb.area_de_trabalho());
                    if let Some(c) = rfd::FileDialog::new()
                        .set_title("Planilha de processos")
                        .set_directory(inicial)
                        .add_filter("Excel", &["xlsx", "xlsm"])
                        .pick_file()
                    {
                        self.planilha = c.to_string_lossy().into_owned();
                    }
                }
                ui.end_row();
                self.ler_abas();
                let l = ui.label("Aba:");
                egui::ComboBox::from_id_salt(("aba", self as *const _ as usize))
                    .selected_text(self.aba.clone())
                    .width(220.0)
                    .show_ui(ui, |ui| {
                        for n in self.abas.clone() {
                            ui.selectable_value(&mut self.aba, n.clone(), n);
                        }
                    })
                    .response
                    .labelled_by(l.id);
                ui.end_row();
            });
    }
}

pub struct AbaFaltantes {
    pub campos: CamposPlanilha,
    pub de: CampoData,
    pub ate: CampoData,
}

impl AbaFaltantes {
    pub const TITULO: &'static str = "Faltantes na planilha";

    pub fn nova(amb: &Ambiente) -> Self {
        let cfg = config::carregar(amb);
        AbaFaltantes {
            campos: CamposPlanilha::nova(&cfg.planilha_faltantes),
            de: CampoData::novo(&cfg.faltantes_de, true),
            ate: CampoData::novo(&cfg.faltantes_ate, false),
        }
    }

    /// O período que será usado, ao lado dos campos: (texto, é erro?).
    pub fn leitura(&self) -> (String, bool) {
        let hoje = datas::hoje();
        match datas::ler_periodo(&self.de.texto, &self.ate.texto, hoje) {
            Err(e) => (e, true),
            Ok((de, ate)) => {
                let inicio = de
                    .map(datas::br)
                    .unwrap_or_else(|| "1º/jan do ano mais antigo da planilha".into());
                let fim = match ate {
                    Some(a) if a < hoje => datas::br(a),
                    _ => format!("hoje ({})", datas::br(hoje)),
                };
                (format!("→ de {inicio} até {fim}"), false)
            }
        }
    }

    pub fn tarefa(&self, amb: &Ambiente) -> Result<Tarefa, String> {
        let (de, ate) = datas::ler_periodo(&self.de.texto, &self.ate.texto, datas::hoje())?;
        let (caminho, aba) = self.campos.validar(amb, "planilha_faltantes")?;
        let mut cfg = config::carregar(amb);
        cfg.faltantes_de = self.de.texto.trim().to_string();
        cfg.faltantes_ate = self.ate.texto.trim().to_string();
        let _ = config::salvar(amb, &cfg);
        Ok(Tarefa {
            titulo: "Faltantes na planilha".into(),
            funcao: Some(Box::new(move |amb, cred| {
                let op = faltantes::Opcoes {
                    de,
                    ate,
                    ..Default::default()
                };
                faltantes::executar(amb, &caminho, aba.as_deref(), cred.expect("sessão"), op)
            })),
            precisa_sessao: true,
        })
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, amb: &Ambiente) {
        descricao(
            ui,
            "Procura, nas caixas do setor, os Proc. Administrativos de abertura, fase preparatória ou elaboração emitidos no período que estão ANTES da Pesquisa de Mercado e não estão na planilha de acompanhamento. Gera <planilha>_faltantes_….xlsx (a original não é alterada), com a aba \"Cancelados (conferir)\" quando houver. A 1ª execução lê os arquivados e demora; as seguintes usam o cache.",
        );
        self.campos.ui(ui, amb);
        ui.horizontal(|ui| {
            let l = ui.label("Emitidos de:");
            self.de.ui(ui, 90.0, "faltantes_de").labelled_by(l.id);
            let l2 = ui.label("até:");
            self.ate.ui(ui, 90.0, "faltantes_ate").labelled_by(l2.id);
            if ui.button("Padrão").clicked() {
                self.de.texto.clear();
                self.ate.texto.clear();
            }
            let (t, erro) = self.leitura();
            ui.label(RichText::new(t).color(if erro { COR_ERRO } else { COR_LEITURA }));
        });
        dica(ui, &format!("{}; vazio = padrão", datas::FORMATOS_DATA));
    }
}

pub struct AbaVerificar {
    pub campos: CamposPlanilha,
    pub html: bool,
}

impl AbaVerificar {
    pub const TITULO: &'static str = "Verificar planilha";

    pub fn nova(amb: &Ambiente) -> Self {
        AbaVerificar {
            campos: CamposPlanilha::nova(&config::carregar(amb).planilha_verificar),
            html: false,
        }
    }

    pub fn tarefa(&self, amb: &Ambiente) -> Result<Tarefa, String> {
        let (caminho, aba) = self.campos.validar(amb, "planilha_verificar")?;
        let pasta_html = self.html.then(|| {
            let cfg = config::carregar(amb);
            config::pasta_saida(amb, &cfg).join(format!(
                "html 1Doc {}",
                Local::now().format("%Y-%m-%d_%H%M")
            ))
        });
        Ok(Tarefa {
            titulo: "Verificar planilha".into(),
            funcao: Some(Box::new(move |amb, cred| {
                let op = verificar_planilha::Opcoes {
                    pasta_html,
                    ..Default::default()
                };
                verificar_planilha::executar(
                    amb,
                    &caminho,
                    aba.as_deref(),
                    cred.expect("sessão"),
                    op,
                )
            })),
            precisa_sessao: true,
        })
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, amb: &Ambiente) {
        descricao(
            ui,
            "Confere, para cada processo da planilha, se ele já chegou à Pesquisa de Mercado (ou a etapa posterior). Processos em elaboração são avaliados pelo processo novo emitido a partir deles. Gera <planilha>_verificada_….xlsx só com os que chegaram, mais as colunas Etapa atual, Processo pai, Processo filho, Objeto (abertura) e Observação.",
        );
        self.campos.ui(ui, amb);
        ui.checkbox(
            &mut self.html,
            "Salvar as páginas de processo baixadas (diagnóstico)",
        );
    }
}
