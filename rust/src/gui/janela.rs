//! Janela principal: barra da sessão, abas das ferramentas e mensagens.
//! A ferramenta roda numa thread; a janela só consome a fila de mensagens
//! (o que a ferramenta escreve) e a fila de eventos (login, sessão, fim).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;

use chrono::Local;
use egui::{Color32, RichText};

use super::abas::{AbaBusca, AbaFaltantes, AbaProcessosDoDia, AbaVerificar, Tarefa};
use crate::ambiente::Ambiente;
use crate::erro::{Erro, Res};
use crate::saida::{Saida, TelaEmMemoria, Terminal};
use crate::sessao::{self, Credenciais};
use crate::{NOME_APP, VERSAO, caminhos, config, msg};

pub const COR_OK: Color32 = Color32::from_rgb(0x1b, 0x7f, 0x3b);
pub const COR_AVISO: Color32 = Color32::from_rgb(0x9a, 0x67, 0x00);
pub const COR_ERRO: Color32 = Color32::from_rgb(0xb0, 0x00, 0x20);
pub const COR_NEUTRA: Color32 = Color32::from_rgb(0x55, 0x55, 0x55);

pub const TITULOS: [&str; 4] = [
    AbaProcessosDoDia::TITULO,
    AbaBusca::TITULO,
    AbaFaltantes::TITULO,
    AbaVerificar::TITULO,
];

/// Avisos das threads para a janela.
pub enum Evento {
    AguardandoLogin(bool),
    Sessao(Option<Credenciais>),
    Fim {
        estado: String,
        cor: Color32,
        resultado: Option<PathBuf>,
    },
}

/// Como obter a sessão (os testes trocam pelo falso).
pub type ObterSessao = Arc<dyn Fn(&Ambiente, &dyn Fn(bool)) -> Res<Credenciais> + Send + Sync>;

fn obter_sessao_real() -> ObterSessao {
    Arc::new(|amb, ao_esperar| sessao::obter_sessao(amb, false, None, None, Some(ao_esperar)))
}

enum Confirmacao {
    Sair,
    Fechar,
}

pub struct DialogoConfig {
    pub navegador: String,
    pub minutos: String,
    pub pasta: String,
    pub abrir: bool,
    pub erro: Option<String>,
}

pub struct Janela {
    pub amb: Ambiente,
    rx_saida: Receiver<String>,
    terminal: Terminal,
    pub mensagens: TelaEmMemoria,
    tx_eventos: Sender<Evento>,
    rx_eventos: Receiver<Evento>,
    pub executando: Option<JoinHandle<()>>,
    pub ultimo_arquivo: Option<PathBuf>,
    pub estado: (String, Color32),
    pub sessao: (String, Color32),
    pub aguardando_login: bool,
    pub aba_atual: usize,
    pub processos: AbaProcessosDoDia,
    pub busca: AbaBusca,
    pub faltantes: AbaFaltantes,
    pub verificar: AbaVerificar,
    /// aviso modal (o messagebox.showwarning da versão Python)
    pub aviso: Option<String>,
    confirmar: Option<Confirmacao>,
    pub configuracoes: Option<DialogoConfig>,
    pub obter_sessao: ObterSessao,
    fechar_confirmado: bool,
}

impl Janela {
    /// Janela sobre um ambiente (o de verdade ou o dos testes). `avisar`
    /// acorda a janela quando chega mensagem de outra thread.
    pub fn nova(amb_base: &Ambiente, avisar: Option<Box<dyn Fn() + Send + Sync>>) -> Self {
        let (tx, rx_saida) = channel();
        let amb = amb_base.com_saida(Saida::fila(tx, avisar));
        let (tx_eventos, rx_eventos) = channel();
        let mut j = Janela {
            busca: AbaBusca::nova(&amb),
            faltantes: AbaFaltantes::nova(&amb),
            verificar: AbaVerificar::nova(&amb),
            processos: AbaProcessosDoDia::default(),
            amb,
            rx_saida,
            terminal: Terminal::default(),
            mensagens: TelaEmMemoria::default(),
            tx_eventos,
            rx_eventos,
            executando: None,
            ultimo_arquivo: None,
            estado: ("Pronto.".into(), COR_NEUTRA),
            sessao: (String::new(), COR_NEUTRA),
            aguardando_login: false,
            aba_atual: 0,
            aviso: None,
            confirmar: None,
            configuracoes: None,
            obter_sessao: obter_sessao_real(),
            fechar_confirmado: false,
        };
        j.mostrar_sessao_salva();
        msg!(
            j.amb,
            "{NOME_APP} {VERSAO} (Rust). Pasta de dados: {}",
            j.amb.pasta_dados().display()
        );
        msg!(
            j.amb,
            "O login no 1Doc é sempre feito por você, na janela do navegador: o programa só espera.\n"
        );
        j
    }

    // ---------------------------------------------------------- mensagens

    /// Esvazia as filas: mensagens das ferramentas e eventos das threads.
    pub fn consumir(&mut self) {
        let mut n = 0;
        while n < 500 {
            let Ok(p) = self.rx_saida.try_recv() else {
                break;
            };
            self.terminal.alimentar(&p, &mut self.mensagens);
            n += 1;
        }
        if n > 0 {
            let linhas = self.mensagens.texto.matches('\n').count();
            if linhas > 20000 {
                // não deixa a caixa crescer sem limite
                let cortar = linhas - 15000;
                if let Some((i, _)) = self.mensagens.texto.match_indices('\n').nth(cortar - 1) {
                    self.mensagens.texto.drain(..=i);
                }
            }
        }
        while let Ok(ev) = self.rx_eventos.try_recv() {
            self.evento(ev);
        }
    }

    fn evento(&mut self, ev: Evento) {
        match ev {
            Evento::AguardandoLogin(sim) => {
                self.aguardando_login = sim;
                if sim {
                    self.estado = ("Esperando o login no navegador…".into(), COR_AVISO);
                }
            }
            Evento::Sessao(cred) => self.mostrar_sessao(cred.as_ref(), true),
            Evento::Fim {
                estado,
                cor,
                resultado,
            } => self.terminou(estado, cor, resultado),
        }
    }

    pub fn mostrar_sessao(&mut self, cred: Option<&Credenciais>, validada: bool) {
        self.sessao = match cred {
            None => ("○ Não conectado ao 1Doc".into(), COR_NEUTRA),
            Some(c) if validada => (
                format!("● Conectado: {} — setor {}", c.email, c.setor),
                COR_OK,
            ),
            Some(c) => (
                format!(
                    "◐ Sessão salva de {} (conferida ao executar)",
                    if c.email.is_empty() {
                        "usuário"
                    } else {
                        &c.email
                    }
                ),
                COR_AVISO,
            ),
        };
    }

    fn mostrar_sessao_salva(&mut self) {
        let salva = sessao::carregar_salva(&self.amb);
        self.mostrar_sessao(salva.as_ref(), false);
    }

    // ---------------------------------------------------------- execução

    pub fn tarefa_da_aba(&self, i: usize) -> Result<Tarefa, String> {
        match i {
            0 => self.processos.tarefa(),
            1 => self.busca.tarefa(&self.amb),
            2 => self.faltantes.tarefa(&self.amb),
            _ => self.verificar.tarefa(&self.amb),
        }
    }

    /// Botão Executar da aba `i`.
    pub fn executar(&mut self, i: usize) {
        if self.executando.is_some() {
            return;
        }
        match self.tarefa_da_aba(i) {
            Err(e) => self.aviso = Some(e),
            Ok(t) => self.iniciar(t),
        }
    }

    /// Botão Entrar no 1Doc.
    pub fn entrar(&mut self) {
        if self.executando.is_none() {
            self.iniciar(Tarefa {
                titulo: "Entrar no 1Doc".into(),
                funcao: None,
                precisa_sessao: true,
            });
        }
    }

    fn iniciar(&mut self, t: Tarefa) {
        self.amb.cancelar.limpar();
        self.estado = (format!("{}: executando…", t.titulo), COR_NEUTRA);
        let primeira = t.titulo.split(' ').next().unwrap_or("").to_lowercase();
        let nome_log = format!("{} {primeira}.log", Local::now().format("%Y-%m-%d_%H%M%S"));
        if self
            .amb
            .saida
            .gravar_em(Some(&self.amb.pasta_logs().join(nome_log)))
            .is_err()
        {
            let _ = self.amb.saida.gravar_em(None);
        }
        let (amb, tx, obter) = (
            self.amb.clone(),
            self.tx_eventos.clone(),
            self.obter_sessao.clone(),
        );
        self.executando = Some(std::thread::spawn(move || trabalho(amb, tx, obter, t)));
    }

    fn terminou(&mut self, estado: String, cor: Color32, resultado: Option<PathBuf>) {
        if let Some(h) = self.executando.take() {
            let _ = h.join();
        }
        let _ = self.amb.saida.gravar_em(None);
        self.aguardando_login = false;
        self.estado = (estado, cor);
        if let Some(r) = resultado.filter(|r| r.is_file()) {
            self.ultimo_arquivo = Some(r);
        }
    }

    pub fn cancelar(&mut self) {
        if self.executando.is_some() {
            self.amb.cancelar.pedir();
            self.estado = (
                "Cancelando… (termina a requisição em curso)".into(),
                COR_AVISO,
            );
        }
    }

    fn sair(&mut self) {
        for aviso in sessao::sair(&self.amb) {
            msg!(self.amb, "⚠ {aviso}");
        }
        msg!(
            self.amb,
            "Sessão apagada. Na próxima execução o navegador abre para o login."
        );
        self.mostrar_sessao(None, true);
    }

    pub fn limpar_mensagens(&mut self) {
        self.mensagens.texto.clear();
    }

    pub fn abrir_configuracoes(&mut self) {
        if self.configuracoes.is_none() {
            let c = config::carregar(&self.amb);
            self.configuracoes = Some(DialogoConfig {
                navegador: c.navegador,
                minutos: c.minutos_login.to_string(),
                pasta: c.pasta_saida,
                abrir: c.abrir_planilha,
                erro: None,
            });
        }
    }

    /// Salvar das Configurações: None se gravou; senão, o aviso.
    pub fn salvar_configuracoes(&mut self) -> Option<String> {
        let d = self.configuracoes.as_ref()?;
        let Ok(minutos) = d.minutos.trim().parse::<i64>() else {
            return Some("Minutos: use um número inteiro.".into());
        };
        let pasta = d.pasta.trim().to_string();
        if !pasta.is_empty() && !std::path::Path::new(&pasta).is_dir() {
            return Some(format!("A pasta não existe:\n{pasta}"));
        }
        let mut cfg = config::carregar(&self.amb);
        cfg.navegador = d.navegador.clone();
        cfg.minutos_login = minutos.max(1) as u64;
        cfg.pasta_saida = pasta;
        cfg.abrir_planilha = d.abrir;
        if let Err(e) = config::salvar(&self.amb, &cfg) {
            return Some(format!("Não consegui salvar: {e}"));
        }
        self.configuracoes = None;
        None
    }

    // ---------------------------------------------------------- desenho

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.consumir();
        let ocupado = self.executando.is_some();
        if ocupado {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
        self.tratar_fechar(ui.ctx());

        egui::Panel::top("topo").show(ui, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(&self.sessao.0).color(self.sessao.1).size(14.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Configurações…").clicked() {
                        self.abrir_configuracoes();
                    }
                    if ui.add_enabled(!ocupado, egui::Button::new("Sair do 1Doc")).clicked() {
                        self.confirmar = Some(Confirmacao::Sair);
                    }
                    if ui.add_enabled(!ocupado, egui::Button::new("Entrar no 1Doc")).clicked() {
                        self.entrar();
                    }
                });
            });
            if self.aguardando_login {
                egui::Frame::new().fill(Color32::from_rgb(0xff, 0xf4, 0xce)).inner_margin(8.0).show(ui, |ui| {
                    ui.label(
                        RichText::new("➜ Faça o login no 1Doc na janela do navegador que abriu. O programa continua sozinho assim que você entrar.")
                            .strong()
                            .color(Color32::from_rgb(0x5c, 0x44, 0x00)),
                    );
                });
            }
            ui.add_space(4.0);
        });

        egui::Panel::bottom("base").show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ocupado {
                    ui.spinner();
                }
                ui.label(RichText::new(&self.estado.0).color(self.estado.1));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(ocupado, egui::Button::new("Cancelar"))
                        .clicked()
                    {
                        self.cancelar();
                    }
                    if ui.button("Limpar mensagens").clicked() {
                        self.limpar_mensagens();
                    }
                    if ui.button("Abrir pasta de saída").clicked() {
                        let cfg = config::carregar(&self.amb);
                        caminhos::abrir_arquivo(
                            &self.amb.saida,
                            &config::pasta_saida(&self.amb, &cfg),
                        );
                    }
                    let tem = self.ultimo_arquivo.is_some();
                    if ui
                        .add_enabled(tem, egui::Button::new("Abrir última planilha"))
                        .clicked()
                        && let Some(a) = &self.ultimo_arquivo
                    {
                        caminhos::abrir_arquivo(&self.amb.saida, a);
                    }
                });
            });
            ui.add_space(4.0);
        });

        egui::Panel::bottom("mensagens")
            .resizable(true)
            .default_size(240.0)
            .min_size(110.0)
            .show(ui, |ui| {
                ui.add_space(4.0);
                ui.label(RichText::new("Mensagens").strong());
                let linhas: Vec<&str> = self.mensagens.texto.split('\n').collect();
                let altura = ui.text_style_height(&egui::TextStyle::Monospace);
                egui::ScrollArea::both()
                    .stick_to_bottom(true)
                    .auto_shrink([false, false])
                    .show_rows(ui, altura, linhas.len(), |ui, faixa| {
                        for l in &linhas[faixa] {
                            ui.add(
                                egui::Label::new(RichText::new(*l).monospace().size(12.0)).extend(),
                            );
                        }
                    });
            });

        egui::CentralPanel::default_margins().show(ui, |ui| {
            ui.horizontal(|ui| {
                for (i, t) in TITULOS.iter().enumerate() {
                    if ui
                        .selectable_label(
                            self.aba_atual == i,
                            RichText::new(format!("  {t}  ")).size(14.0),
                        )
                        .clicked()
                    {
                        self.aba_atual = i;
                    }
                }
            });
            ui.separator();
            // o rodapé com o Executar fica fora da rolagem: nunca some
            let rodape = 44.0;
            let altura = (ui.available_height() - rodape).max(60.0);
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .max_height(altura)
                .show(ui, |ui| {
                    let amb = self.amb.clone();
                    match self.aba_atual {
                        0 => self.processos.ui(ui),
                        1 => self.busca.ui(ui),
                        2 => self.faltantes.ui(ui, &amb),
                        _ => self.verificar.ui(ui, &amb),
                    }
                });
            ui.separator();
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), 34.0),
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    let b = egui::Button::new(RichText::new("Executar").strong().size(14.0))
                        .min_size(egui::vec2(110.0, 30.0));
                    if ui.add_enabled(!ocupado, b).clicked() {
                        self.executar(self.aba_atual);
                    }
                },
            );
        });

        self.dialogos(ui.ctx());
    }

    fn tratar_fechar(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested())
            && self.executando.is_some()
            && !self.fechar_confirmado
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.confirmar = Some(Confirmacao::Fechar);
        }
    }

    fn dialogos(&mut self, ctx: &egui::Context) {
        if let Some(texto) = self.aviso.clone() {
            let r = egui::Modal::new(egui::Id::unique("aviso")).show(ctx, |ui| {
                ui.set_max_width(460.0);
                ui.heading(NOME_APP);
                ui.add_space(6.0);
                ui.label(format!("⚠ {texto}"));
                ui.add_space(10.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.button("OK").clicked()
                })
                .inner
            });
            if r.inner || r.should_close() {
                self.aviso = None;
            }
        }
        if let Some(c) = &self.confirmar {
            let pergunta = match c {
                Confirmacao::Sair => {
                    "Sair do 1Doc neste computador?\n\nA sessão salva e o perfil do navegador usado no login serão apagados; na próxima execução o navegador abre para você entrar de novo."
                }
                Confirmacao::Fechar => "Há uma execução em andamento. Cancelar e fechar?",
            };
            let mut resposta = None;
            let r = egui::Modal::new(egui::Id::unique("confirmar")).show(ctx, |ui| {
                ui.set_max_width(460.0);
                ui.heading(NOME_APP);
                ui.add_space(6.0);
                ui.label(pergunta);
                ui.add_space(10.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Não").clicked() {
                        resposta = Some(false);
                    }
                    if ui.button("Sim").clicked() {
                        resposta = Some(true);
                    }
                });
            });
            if r.should_close() && resposta.is_none() {
                resposta = Some(false);
            }
            if let Some(sim) = resposta {
                let c = self.confirmar.take().unwrap();
                if sim {
                    match c {
                        Confirmacao::Sair => self.sair(),
                        Confirmacao::Fechar => {
                            self.amb.cancelar.pedir();
                            self.fechar_confirmado = true;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    }
                }
            }
        }
        if self.configuracoes.is_some() {
            let mut aberto = true;
            let mut salvar = false;
            let mut cancelar = false;
            let pasta_dados = self.amb.pasta_dados();
            let area = self.amb.area_de_trabalho();
            egui::Window::new("Configurações")
                .collapsible(false)
                .resizable(false)
                .open(&mut aberto)
                .show(ctx, |ui| {
                    let d = self.configuracoes.as_mut().unwrap();
                    egui::Grid::new("config")
                        .num_columns(3)
                        .spacing([8.0, 8.0])
                        .show(ui, |ui| {
                            let l = ui.label("Navegador do login:");
                            let atual = config::NAVEGADORES
                                .iter()
                                .find(|(k, _)| *k == d.navegador)
                                .map(|x| x.1)
                                .unwrap_or("");
                            egui::ComboBox::from_id_salt("navegador")
                                .selected_text(atual)
                                .width(300.0)
                                .show_ui(ui, |ui| {
                                    for (k, v) in config::NAVEGADORES {
                                        ui.selectable_value(&mut d.navegador, k.to_string(), v);
                                    }
                                })
                                .response
                                .labelled_by(l.id);
                            ui.end_row();
                            let l = ui.label("Esperar o login por até:");
                            ui.horizontal(|ui| {
                                ui.add(
                                    egui::TextEdit::singleline(&mut d.minutos).desired_width(40.0),
                                )
                                .labelled_by(l.id);
                                ui.label("minutos");
                            });
                            ui.end_row();
                            let l = ui.label("Pasta das planilhas:");
                            ui.add(egui::TextEdit::singleline(&mut d.pasta).desired_width(330.0))
                                .labelled_by(l.id);
                            if ui.button("Escolher…").clicked() {
                                let inicial = if d.pasta.is_empty() {
                                    area.clone()
                                } else {
                                    PathBuf::from(&d.pasta)
                                };
                                if let Some(p) = rfd::FileDialog::new()
                                    .set_title("Pasta das planilhas")
                                    .set_directory(inicial)
                                    .pick_folder()
                                {
                                    d.pasta = p.to_string_lossy().into_owned();
                                }
                            }
                            ui.end_row();
                            ui.label("");
                            ui.label(
                                RichText::new(format!(
                                    "vazio = Área de Trabalho ({})",
                                    area.display()
                                ))
                                .color(Color32::from_rgb(0x77, 0x77, 0x77)),
                            );
                            ui.end_row();
                            ui.label("");
                            ui.checkbox(&mut d.abrir, "Abrir a planilha no Excel ao terminar");
                            ui.end_row();
                        });
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!(
                                "Sessão, caches e logs ficam em:\n{}",
                                pasta_dados.display()
                            ))
                            .color(COR_NEUTRA),
                        );
                        if ui.button("Abrir pasta de dados").clicked() {
                            caminhos::abrir_arquivo(&Saida::nada(), &pasta_dados);
                        }
                    });
                    if let Some(e) = &d.erro {
                        ui.label(RichText::new(format!("⚠ {e}")).color(COR_ERRO));
                    }
                    ui.add_space(8.0);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Cancelar").clicked() {
                            cancelar = true;
                        }
                        if ui.button("Salvar").clicked() {
                            salvar = true;
                        }
                    });
                });
            if salvar
                && let Some(e) = self.salvar_configuracoes()
                && let Some(d) = self.configuracoes.as_mut()
            {
                d.erro = Some(e);
            }
            if !aberto || cancelar {
                self.configuracoes = None;
            }
        }
        if self.busca.ajuda_aberta {
            let mut aberto = true;
            let texto = AbaBusca::texto_ajuda();
            egui::Window::new("Opções da busca")
                .open(&mut aberto)
                .default_size([900.0, 620.0])
                .show(ctx, |ui| {
                    egui::ScrollArea::both().show(ui, |ui| {
                        ui.add(
                            egui::Label::new(RichText::new(texto).monospace().size(12.0)).extend(),
                        );
                    });
                });
            self.busca.ajuda_aberta = aberto;
        }
    }
}

/// Roda na thread. Só fala com a janela pela fila de eventos.
fn trabalho(amb: Ambiente, tx: Sender<Evento>, obter: ObterSessao, t: Tarefa) {
    let linha = "─".repeat(70);
    msg!(
        amb,
        "\n{linha}\n▶ {} — {}\n{linha}",
        t.titulo,
        Local::now().format("%d/%m/%Y %H:%M")
    );
    let tx2 = tx.clone();
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Res<Option<PathBuf>> {
        let mut cred = None;
        if t.precisa_sessao {
            let ao_esperar = |sim: bool| {
                let _ = tx2.send(Evento::AguardandoLogin(sim));
            };
            let c = obter(&amb, &ao_esperar)?;
            let _ = tx2.send(Evento::Sessao(Some(c.clone())));
            cred = Some(c);
        }
        match t.funcao {
            Some(f) => f(&amb, cred.as_ref()),
            None => Ok(None),
        }
    }));
    let (estado, cor, resultado) = match r {
        Ok(Ok(res)) => ("Concluído.".to_string(), COR_OK, res),
        Ok(Err(Erro::Cancelado)) => {
            msg!(amb, "\n⏹ Cancelado.");
            ("Cancelado.".into(), COR_AVISO, None)
        }
        Ok(Err(Erro::LoginCancelado(e))) => {
            msg!(amb, "\n✗ Login não concluído: {e}");
            ("Login não concluído.".into(), COR_ERRO, None)
        }
        Ok(Err(Erro::Encerrar(m))) => {
            msg!(amb, "\n{m}");
            (
                "Terminou com aviso — veja as mensagens.".into(),
                COR_ERRO,
                None,
            )
        }
        Ok(Err(e)) => {
            amb.saida.gravar_no_log(&format!("{e:?}\n"));
            msg!(amb, "\n✗ Erro inesperado: {}: {e}", e.nome_tipo());
            msg!(
                amb,
                "  Detalhes técnicos no arquivo de log (Configurações → Abrir pasta de dados → logs)."
            );
            ("Erro — veja as mensagens.".into(), COR_ERRO, None)
        }
        Err(p) => {
            let m = p
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_default();
            amb.saida.gravar_no_log(&format!("pânico: {m}\n"));
            msg!(amb, "\n✗ Erro inesperado: {m}");
            msg!(
                amb,
                "  Detalhes técnicos no arquivo de log (Configurações → Abrir pasta de dados → logs)."
            );
            ("Erro — veja as mensagens.".into(), COR_ERRO, None)
        }
    };
    if sessao::carregar_salva(&amb).is_none() {
        let _ = tx.send(Evento::Sessao(None));
    }
    let _ = tx.send(Evento::Fim {
        estado,
        cor,
        resultado,
    });
}
