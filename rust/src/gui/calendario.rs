//! Seletor de data: um botão ao lado do campo abre o calendário do mês logo
//! abaixo dele. Clicar num dia escreve a data (DD/MM/AAAA); digitar continua
//! valendo. O título (mês e ano) abre a grade dos meses do ano e, desta, a
//! dos anos da década. ◀ ▶ e a roda do mouse passam o mês, o ano ou a
//! década; no teclado, as setas mudam o dia, PageUp/PageDown o mês,
//! Shift+PageUp/PageDown o ano, Home vai a hoje, Enter escolhe e Esc fecha.
//! A lógica fica aqui, separada do desenho, para ser testada sem janela.

use chrono::{Datelike, Duration, NaiveDate};
use egui::{Color32, RichText};

use crate::datas;
use crate::re;

pub const MESES: [&str; 12] = [
    "janeiro",
    "fevereiro",
    "março",
    "abril",
    "maio",
    "junho",
    "julho",
    "agosto",
    "setembro",
    "outubro",
    "novembro",
    "dezembro",
];
pub const DIAS_SEMANA: [&str; 7] = ["D", "S", "T", "Q", "Q", "S", "S"]; // a semana começa no domingo
pub const ANO_MIN: i32 = 1900;
pub const ANO_MAX: i32 = 2100;
pub const COLUNAS: i64 = 4; // grades de meses e de anos: 4 × 3 casas

const AZUL: Color32 = Color32::from_rgb(0x1f, 0x6f, 0xc5);
const AZUL_CLARO: Color32 = Color32::from_rgb(0xdc, 0xe9, 0xf9);
const APAGADO: Color32 = Color32::from_rgb(0x9a, 0x9a, 0x9a);
const BLOQUEADO: Color32 = Color32::from_rgb(0xcf, 0xcf, 0xcf);
const TEXTO: Color32 = Color32::from_rgb(0x22, 0x22, 0x22);

pub fn mes_curto(m: u32) -> String {
    MESES[(m - 1) as usize].chars().take(3).collect()
}

/// A data que está no campo, em qualquer formato das abas (DD/MM/AAAA,
/// DD/MM/AA, DD/MM, AAAA-MM-DD, hoje, ontem, -N), para o calendário abrir
/// no mês dela. None se não for uma data.
pub fn ler_data_qualquer(txt: &str, hoje: NaiveDate) -> Option<NaiveDate> {
    let t = re!(r"\s+").replace_all(txt.trim(), " ").to_lowercase();
    if t.is_empty() {
        return None;
    }
    if t == "hoje" {
        return Some(hoje);
    }
    if t == "ontem" {
        return Some(datas::ontem(hoje));
    }
    if let Some(m) = re!(r"^- ?(\d{1,5})$").captures(&t) {
        return datas::dias_atras(hoje, &m[1]);
    }
    if let Some(m) = re!(r"^(\d{4})-(\d{1,2})-(\d{1,2})$").captures(&t) {
        return NaiveDate::from_ymd_opt(m[1].parse().ok()?, m[2].parse().ok()?, m[3].parse().ok()?);
    }
    datas::interpretar_data(&t, hoje)
}

/// As seis semanas (domingo a sábado) que mostram o mês, com os dias dos
/// meses vizinhos completando as pontas. Sempre seis.
pub fn semanas(ano: i32, mes: u32) -> Vec<Vec<NaiveDate>> {
    let primeiro = NaiveDate::from_ymd_opt(ano, mes, 1).expect("mês");
    let inicio = primeiro - Duration::days(primeiro.weekday().num_days_from_sunday() as i64);
    (0..6)
        .map(|s| (0..7).map(|d| inicio + Duration::days(s * 7 + d)).collect())
        .collect()
}

/// Os anos da grade de anos: a década de `ano`, com o último ano da
/// anterior e o primeiro da seguinte nas pontas. 12 casas.
pub fn anos_da_decada(ano: i32) -> Vec<i32> {
    let inicio = ano.div_euclid(10) * 10;
    (inicio - 1..inicio + 11).collect()
}

pub fn mes_ao_lado(ano: i32, mes: u32, passo: i64) -> (i32, u32) {
    let n = ano as i64 * 12 + (mes as i64 - 1) + passo;
    (n.div_euclid(12) as i32, (n.rem_euclid(12) + 1) as u32)
}

/// O dia `dia` do mês, ou o último dia dele (31 em fevereiro → 28 ou 29).
pub fn no_mes(ano: i32, mes: u32, dia: u32) -> NaiveDate {
    (1..=dia.min(31))
        .rev()
        .find_map(|d| NaiveDate::from_ymd_opt(ano, mes, d))
        .expect("dia")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Vista {
    Dias,
    Meses,
    Anos,
}

/// O que identifica uma casa da grade à mostra.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chave {
    Dia(NaiveDate),
    Mes(i32, u32),
    Ano(i32),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Resultado {
    Escolheu(NaiveDate),
    Limpou,
    Fechou,
}

#[derive(Clone, Debug)]
pub struct Calendario {
    pub vista: Vista,
    pub ano: i32,
    pub mes: u32,
    pub foco: NaiveDate,
    pub escolhida: Option<NaiveDate>,
    pub maximo: Option<NaiveDate>,
    pub hoje: NaiveDate,
    pub sob: Option<Chave>,
}

impl Calendario {
    pub fn novo(texto: &str, maximo: Option<NaiveDate>, hoje: NaiveDate) -> Self {
        let escolhida = ler_data_qualquer(texto, hoje);
        let foco = match escolhida {
            Some(e) if (ANO_MIN..=ANO_MAX).contains(&e.year()) => e,
            _ => maximo.map(|m| m.min(hoje)).unwrap_or(hoje),
        };
        Calendario {
            vista: Vista::Dias,
            ano: foco.year(),
            mes: foco.month(),
            foco,
            escolhida,
            maximo,
            hoje,
            sob: None,
        }
    }

    pub fn bloqueado(&self, d: NaiveDate) -> bool {
        self.maximo.is_some_and(|m| d > m)
    }

    pub fn chave(&self, d: NaiveDate) -> Chave {
        match self.vista {
            Vista::Dias => Chave::Dia(d),
            Vista::Meses => Chave::Mes(d.year(), d.month()),
            Vista::Anos => Chave::Ano(d.year()),
        }
    }

    /// A casa não se escolhe: depois do máximo, ou ano fora de 1900–2100.
    pub fn fora(&self, chave: Chave) -> bool {
        match chave {
            Chave::Dia(d) => self.bloqueado(d),
            Chave::Mes(a, m) => self.bloqueado(NaiveDate::from_ymd_opt(a, m, 1).unwrap()),
            Chave::Ano(a) => {
                !(ANO_MIN..=ANO_MAX).contains(&a)
                    || self.bloqueado(NaiveDate::from_ymd_opt(a, 1, 1).unwrap())
            }
        }
    }

    pub fn titulo(&self) -> String {
        match self.vista {
            Vista::Dias => format!("{} de {} ▾", MESES[(self.mes - 1) as usize], self.ano),
            Vista::Meses => format!("{} ▾", self.ano),
            Vista::Anos => {
                let a = anos_da_decada(self.ano);
                format!("{}–{}", a[1], a[10])
            }
        }
    }

    /// Para onde ◀ (-1) e ▶ (1) levam: o mês, o ano ou a década ao lado.
    /// None se não há para onde ir.
    pub fn ao_lado(&self, passo: i64) -> Option<(i32, u32)> {
        let mut mes = self.mes;
        let (ano, primeiro, ultimo, inicio) = match self.vista {
            Vista::Dias => {
                let (a, m) = mes_ao_lado(self.ano, self.mes, passo);
                mes = m;
                (a, a, a, NaiveDate::from_ymd_opt(a, m, 1)?)
            }
            Vista::Meses => {
                let a = self.ano + passo as i32;
                (a, a, a, NaiveDate::from_ymd_opt(a, 1, 1)?)
            }
            Vista::Anos => {
                let p = self.ano.div_euclid(10) * 10 + 10 * passo as i32;
                (p, p, p + 9, NaiveDate::from_ymd_opt(p, 1, 1)?)
            }
        };
        if ultimo < ANO_MIN || primeiro > ANO_MAX || (passo > 0 && self.bloqueado(inicio)) {
            return None;
        }
        Some((ano, mes))
    }

    pub fn avancar(&mut self, passo: i64) {
        if let Some((a, m)) = self.ao_lado(passo) {
            self.ano = a;
            self.mes = m;
        }
    }

    /// Título: da grade dos dias à dos meses do ano, e desta à dos anos.
    pub fn subir(&mut self) {
        self.vista = match self.vista {
            Vista::Dias => Vista::Meses,
            _ => Vista::Anos,
        };
        self.sob = None;
    }

    pub fn focar(&mut self, d: NaiveDate) {
        self.foco = d;
        self.ano = d.year();
        self.mes = d.month();
    }

    /// Abre a casa: o mês na grade dos dias, ou o ano na dos meses.
    pub fn entrar(&mut self, chave: Chave) {
        if self.vista == Vista::Dias || self.fora(chave) {
            return;
        }
        let (ano, mes, vista) = match chave {
            Chave::Mes(a, m) => (a, m, Vista::Dias),
            Chave::Ano(a) => (a, self.foco.month(), Vista::Meses),
            Chave::Dia(_) => return,
        };
        let novo = no_mes(ano, mes, self.foco.day());
        self.vista = vista;
        self.sob = None;
        self.focar(if self.bloqueado(novo) {
            self.maximo.unwrap()
        } else {
            novo
        });
    }

    pub fn mover_foco(&mut self, dias: i64) {
        let novo = self.foco + Duration::days(dias);
        if self.bloqueado(novo) || !(ANO_MIN..=ANO_MAX).contains(&novo.year()) {
            return;
        }
        self.focar(novo);
    }

    /// PageUp/PageDown (e as setas nas grades de meses e anos): o foco vai n
    /// meses para trás ou para a frente, no mesmo dia. Para a frente, para no máximo.
    pub fn mover_meses(&mut self, n: i64) {
        let (ano, mes) = mes_ao_lado(self.foco.year(), self.foco.month(), n);
        if !(ANO_MIN..=ANO_MAX).contains(&ano) {
            return;
        }
        let mut novo = no_mes(ano, mes, self.foco.day());
        if self.bloqueado(novo) {
            novo = self.maximo.unwrap();
            if n > 0 && novo <= self.foco {
                return;
            }
        }
        self.focar(novo);
    }

    /// Setas: o dia; nas outras grades, o mês ou o ano.
    pub fn seta(&mut self, n: i64) {
        if self.vista == Vista::Dias {
            self.mover_foco(n);
        } else {
            let casas = if n.abs() == 1 {
                n
            } else {
                COLUNAS * n.signum()
            };
            self.mover_meses(casas * if self.vista == Vista::Meses { 1 } else { 12 });
        }
    }

    pub fn pagina(&self) -> i64 {
        match self.vista {
            Vista::Dias => 1,
            Vista::Meses => 12,
            Vista::Anos => 120,
        }
    }

    pub fn escolher(&self, d: NaiveDate) -> Option<Resultado> {
        if self.bloqueado(d) {
            None
        } else {
            Some(Resultado::Escolheu(d))
        }
    }

    /// Enter: escolhe o dia do foco ou, nas outras grades, abre o mês ou o ano dele.
    pub fn confirmar(&mut self) -> Option<Resultado> {
        if self.vista == Vista::Dias {
            return self.escolher(self.foco);
        }
        let c = self.chave(self.foco);
        self.entrar(c);
        None
    }

    /// As casas da grade à mostra: (chave, texto, apagada).
    pub fn casas(&self) -> Vec<(Chave, String, bool)> {
        match self.vista {
            Vista::Dias => semanas(self.ano, self.mes)
                .into_iter()
                .flatten()
                .map(|d| (Chave::Dia(d), d.day().to_string(), d.month() != self.mes))
                .collect(),
            Vista::Meses => (1..=12)
                .map(|m| (Chave::Mes(self.ano, m), mes_curto(m), false))
                .collect(),
            Vista::Anos => {
                let a = anos_da_decada(self.ano);
                a.iter()
                    .enumerate()
                    .map(|(i, x)| {
                        let t = if (ANO_MIN..=ANO_MAX).contains(x) {
                            x.to_string()
                        } else {
                            String::new()
                        };
                        (Chave::Ano(*x), t, i == 0 || i == 11)
                    })
                    .collect()
            }
        }
    }

    /// Teclas, numa volta da janela.
    pub fn teclado(&mut self, ctx: &egui::Context) -> Option<Resultado> {
        use egui::Key;
        let mut r = None;
        ctx.input_mut(|i| {
            let shift = i.modifiers.shift;
            for (tecla, n) in [
                (Key::ArrowLeft, -1),
                (Key::ArrowRight, 1),
                (Key::ArrowUp, -7),
                (Key::ArrowDown, 7),
            ] {
                if i.consume_key(egui::Modifiers::NONE, tecla) {
                    self.seta(n);
                }
            }
            for (tecla, p) in [(Key::PageUp, -1), (Key::PageDown, 1)] {
                if i.consume_key(egui::Modifiers::SHIFT, tecla) {
                    self.mover_meses(p * self.pagina().max(12));
                } else if !shift && i.consume_key(egui::Modifiers::NONE, tecla) {
                    self.mover_meses(p * self.pagina());
                }
            }
            if i.consume_key(egui::Modifiers::NONE, Key::Home) {
                let n = (self.hoje - self.foco).num_days();
                self.mover_foco(n);
            }
            if i.consume_key(egui::Modifiers::NONE, Key::Enter) {
                r = self.confirmar();
            }
            if i.consume_key(egui::Modifiers::NONE, Key::Escape) {
                r = Some(Resultado::Fechou);
            }
        });
        r
    }

    /// Desenha o calendário; devolve o que a pessoa escolheu, se escolheu.
    pub fn ui(&mut self, ui: &mut egui::Ui) -> Option<Resultado> {
        let mut r = None;
        ui.spacing_mut().item_spacing = egui::vec2(2.0, 2.0);
        // largura fixa: trocar de grade (ou de mês) não muda o tamanho, e ◀ ▶ não saem do lugar
        let largura_total = 7.0 * 28.0 + 6.0 * 2.0;
        ui.set_width(largura_total);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    self.ao_lado(-1).is_some(),
                    egui::Button::new("◀").min_size(egui::vec2(26.0, 22.0)),
                )
                .clicked()
            {
                self.avancar(-1);
            }
            let largura = largura_total - 2.0 * 26.0 - 4.0;
            let titulo = ui.add_sized(
                [largura, 22.0],
                egui::Button::new(RichText::new(self.titulo()).strong()).frame(false),
            );
            if titulo.clicked() && self.vista != Vista::Anos {
                self.subir();
            }
            if ui
                .add_enabled(
                    self.ao_lado(1).is_some(),
                    egui::Button::new("▶").min_size(egui::vec2(26.0, 22.0)),
                )
                .clicked()
            {
                self.avancar(1);
            }
        });
        let colunas = if self.vista == Vista::Dias {
            7
        } else {
            COLUNAS as usize
        };
        let (lado, altura) = if self.vista == Vista::Dias {
            (28.0, 22.0)
        } else {
            ((largura_total - 3.0 * 2.0) / 4.0, 46.0)
        };
        let escolhida = self.escolhida.map(|e| self.chave(e));
        let foco = self.chave(self.foco);
        let hoje = self.chave(self.hoje);
        let casas = self.casas();
        egui::Grid::new("grade_calendario")
            .spacing([2.0, 2.0])
            .show(ui, |ui| {
                if self.vista == Vista::Dias {
                    for d in DIAS_SEMANA {
                        ui.add_sized(
                            [lado, 16.0],
                            egui::Label::new(RichText::new(d).strong().color(APAGADO)),
                        );
                    }
                    ui.end_row();
                }
                let mut novo_sob = None;
                for (i, (chave, texto, apagada)) in casas.iter().enumerate() {
                    let fora = self.fora(*chave);
                    let (mut fundo, mut cor) =
                        (Color32::TRANSPARENT, if *apagada { APAGADO } else { TEXTO });
                    let mut t = RichText::new(texto);
                    if fora {
                        cor = BLOQUEADO;
                    } else if Some(*chave) == escolhida {
                        fundo = AZUL;
                        cor = Color32::WHITE;
                        t = t.strong();
                    } else if *chave == foco || Some(*chave) == self.sob {
                        fundo = AZUL_CLARO;
                    }
                    let mut b = egui::Button::new(t.color(cor)).fill(fundo);
                    if *chave == hoje {
                        b = b.stroke(egui::Stroke::new(1.0, AZUL));
                    } else {
                        b = b.stroke(egui::Stroke::NONE);
                    }
                    let resp = ui.add_sized([lado, altura], b);
                    if resp.hovered() {
                        novo_sob = Some(*chave);
                    }
                    if resp.clicked() && !fora {
                        match chave {
                            Chave::Dia(d) => r = self.escolher(*d),
                            outra => self.entrar(*outra),
                        }
                    }
                    if (i + 1) % colunas == 0 {
                        ui.end_row();
                    }
                }
                self.sob = novo_sob;
            });
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!self.bloqueado(self.hoje), egui::Button::new("Hoje"))
                .clicked()
            {
                r = self.escolher(self.hoje);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Limpar").clicked() {
                    r = Some(Resultado::Limpou);
                }
            });
        });
        if ui.ui_contains_pointer() {
            let rolar = ui.input(|i| i.smooth_scroll_delta.y);
            if rolar.abs() > 0.5 {
                self.avancar(if rolar > 0.0 { -1 } else { 1 });
            }
        }
        r
    }
}

/// Botão com o ícone de calendário (desenhado aqui, como o da versão Tk).
pub fn botao_calendario(ui: &mut egui::Ui) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(22.0, 20.0), egui::Sense::click());
    let p = ui.painter();
    let fundo = if resp.hovered() {
        ui.visuals().widgets.hovered.weak_bg_fill
    } else {
        ui.visuals().widgets.inactive.weak_bg_fill
    };
    p.rect_filled(rect, 3.0, fundo);
    let c = rect.center();
    let icone = egui::Rect::from_center_size(c + egui::vec2(0.0, 1.0), egui::vec2(14.0, 13.0));
    let cinza = Color32::from_rgb(0x55, 0x55, 0x55);
    p.rect_filled(icone, 1.0, Color32::WHITE);
    p.rect_stroke(
        icone,
        1.0,
        egui::Stroke::new(1.0, cinza),
        egui::StrokeKind::Inside,
    );
    p.rect_filled(
        egui::Rect::from_min_size(icone.min, egui::vec2(14.0, 4.0)),
        1.0,
        Color32::from_rgb(0xc0, 0x39, 0x2b),
    );
    for dx in [-3.5, 3.5] {
        p.line_segment(
            [c + egui::vec2(dx, -7.0), c + egui::vec2(dx, -3.0)],
            egui::Stroke::new(1.5, cinza),
        );
    }
    for (x, y) in [(-4.0, 2.0), (0.0, 2.0), (4.0, 2.0), (-4.0, 5.0), (0.0, 5.0)] {
        p.rect_filled(
            egui::Rect::from_center_size(c + egui::vec2(x, y), egui::vec2(2.0, 2.0)),
            0.0,
            Color32::from_rgb(0x8a, 0x8a, 0x8a),
        );
    }
    resp
}

/// Um campo de data com o botão do calendário; o calendário aberto fica no
/// estado da aba (um por vez no programa).
pub struct CampoData {
    pub texto: String,
    pub calendario: Option<Calendario>,
    pub maximo_hoje: bool,
}

impl CampoData {
    pub fn novo(texto: &str, maximo_hoje: bool) -> Self {
        CampoData {
            texto: texto.into(),
            calendario: None,
            maximo_hoje,
        }
    }

    pub fn abrir(&mut self) {
        let hoje = datas::hoje();
        self.calendario = Some(Calendario::novo(
            &self.texto,
            if self.maximo_hoje { Some(hoje) } else { None },
            hoje,
        ));
    }

    pub fn aplicar(&mut self, r: Resultado) {
        match r {
            Resultado::Escolheu(d) => self.texto = datas::br(d),
            Resultado::Limpou => self.texto.clear(),
            Resultado::Fechou => {}
        }
        self.calendario = None;
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, largura: f32, id: &str) -> egui::Response {
        let campo = ui.add(
            egui::TextEdit::singleline(&mut self.texto)
                .desired_width(largura)
                .id_salt(id),
        );
        let botao = botao_calendario(ui).on_hover_text("Abrir o calendário");
        if botao.clicked() {
            if self.calendario.is_some() {
                self.calendario = None;
            } else {
                self.abrir();
            }
        }
        if let Some(cal) = self.calendario.as_mut() {
            let area = egui::Area::new(egui::Id::unique(("calendario", id)))
                .order(egui::Order::Foreground)
                .fixed_pos(campo.rect.left_bottom() + egui::vec2(0.0, 2.0))
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style())
                        .show(ui, |ui| cal.ui(ui))
                        .inner
                });
            let mut r = area.inner.or_else(|| cal.teclado(ui.ctx()));
            // clique fora fecha sem mudar nada
            let fora = ui.input(|i| {
                i.pointer.any_pressed()
                    && i.pointer
                        .interact_pos()
                        .is_some_and(|p| !area.response.rect.contains(p))
            });
            if r.is_none() && fora && !botao.hovered() {
                r = Some(Resultado::Fechou);
            }
            if let Some(r) = r {
                self.aplicar(r);
            }
        }
        campo
    }
}
