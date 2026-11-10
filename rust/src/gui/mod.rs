//! A janela do programa (egui/eframe).

pub mod abas;
pub mod calendario;
pub mod janela;

use crate::ambiente::Ambiente;
use crate::saida::Saida;
use crate::{NOME_APP, VERSAO};

pub use janela::Janela;

/// Fontes do Windows (Segoe UI, Consolas), com as do egui de reserva para
/// os símbolos (✓ ✗ ⚠ ▾ ◀ ▶).
pub fn configurar_fontes(ctx: &egui::Context) {
    let mut fontes = egui::FontDefinitions::default();
    let pasta = std::env::var_os("WINDIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "C:\\Windows".into())
        .join("Fonts");
    let mut carregar = |nome: &str, arquivo: &str, familia: egui::FontFamily, primeiro: bool| {
        if let Ok(b) = std::fs::read(pasta.join(arquivo)) {
            fontes.font_data.insert(
                nome.into(),
                std::sync::Arc::new(egui::FontData::from_owned(b)),
            );
            let lista = fontes.families.entry(familia).or_default();
            if primeiro {
                lista.insert(0, nome.into())
            } else {
                lista.push(nome.into())
            }
        }
    };
    if cfg!(windows) {
        carregar("segoe", "segoeui.ttf", egui::FontFamily::Proportional, true);
        carregar("consolas", "consola.ttf", egui::FontFamily::Monospace, true);
        carregar(
            "segoe-simbolos",
            "seguisym.ttf",
            egui::FontFamily::Proportional,
            false,
        );
        carregar(
            "segoe-simbolos-mono",
            "seguisym.ttf",
            egui::FontFamily::Monospace,
            false,
        );
    } else {
        // fora do Windows (desenvolvimento): DejaVu, se houver, para setas e símbolos
        let pasta = std::path::Path::new("/usr/share/fonts/truetype/dejavu");
        for (nome, arquivo, familia) in [
            ("dejavu", "DejaVuSans.ttf", egui::FontFamily::Proportional),
            (
                "dejavu-mono",
                "DejaVuSansMono.ttf",
                egui::FontFamily::Monospace,
            ),
            (
                "dejavu-mono-2",
                "DejaVuSans.ttf",
                egui::FontFamily::Monospace,
            ),
        ] {
            if let Ok(b) = std::fs::read(pasta.join(arquivo)) {
                fontes.font_data.insert(
                    nome.into(),
                    std::sync::Arc::new(egui::FontData::from_owned(b)),
                );
                fontes
                    .families
                    .entry(familia)
                    .or_default()
                    .push(nome.into());
            }
        }
    }
    ctx.set_fonts(fontes);
    // caractere que nenhuma fonte tem vira um quadradinho, nunca um erro
    ctx.set_missing_glyph_policy(egui::epaint::text::MissingGlyphPolicy::Tofu);
    ctx.set_visuals(egui::Visuals::light());
}

/// Ícone da janela: a imagem 64×64 (PNG) de dentro do icone.ico.
pub fn icone() -> Option<egui::IconData> {
    let ico: &[u8] = include_bytes!("../../../ferramentas1doc/gui/icone.ico");
    let n = u16::from_le_bytes([ico[4], ico[5]]) as usize;
    let mut melhor: Option<(u32, &[u8])> = None;
    for i in 0..n {
        let e = &ico[6 + 16 * i..22 + 16 * i];
        let lado = if e[0] == 0 { 256 } else { e[0] as u32 };
        let tam = u32::from_le_bytes([e[8], e[9], e[10], e[11]]) as usize;
        let ini = u32::from_le_bytes([e[12], e[13], e[14], e[15]]) as usize;
        let png = ico.get(ini..ini + tam)?;
        if lado <= 64 && melhor.is_none_or(|m| lado > m.0) {
            melhor = Some((lado, png));
        }
    }
    eframe::icon_data::from_png_bytes(melhor?.1).ok()
}

fn opcoes(renderer: eframe::Renderer) -> eframe::NativeOptions {
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(format!(
            "{NOME_APP} — Prefeitura de Montes Claros  (v{VERSAO})"
        ))
        .with_inner_size([1100.0, 920.0])
        .with_min_inner_size([960.0, 700.0]);
    if let Some(i) = icone() {
        viewport = viewport.with_icon(std::sync::Arc::new(i));
    }
    eframe::NativeOptions {
        viewport,
        renderer,
        ..Default::default()
    }
}

/// O desenho da janela: wgpu (DirectX 12, com o WARP em software onde não
/// há placa de vídeo) se houver adaptador; senão OpenGL. Decidido antes de
/// abrir a janela: o winit não deixa abrir um segundo laço de eventos no
/// mesmo processo para tentar de novo. `FERRAMENTAS1DOC_DESENHO=glow|wgpu` força.
pub fn escolher_desenho() -> eframe::Renderer {
    match std::env::var("FERRAMENTAS1DOC_DESENHO").as_deref() {
        Ok("glow") => return eframe::Renderer::Glow,
        Ok("wgpu") => return eframe::Renderer::Wgpu,
        _ => {}
    }
    let tem = std::panic::catch_unwind(|| {
        let instancia = egui_wgpu::wgpu::Instance::default();
        pollster::block_on(
            instancia.request_adapter(&egui_wgpu::wgpu::RequestAdapterOptions::default()),
        )
        .is_ok()
    });
    if tem.unwrap_or(false) {
        eframe::Renderer::Wgpu
    } else {
        eframe::Renderer::Glow
    }
}

pub fn rodar_com(
    criar: &dyn Fn(&eframe::CreationContext) -> Box<dyn eframe::App>,
) -> eframe::Result {
    eframe::run_native(
        NOME_APP,
        opcoes(escolher_desenho()),
        Box::new(|cc| Ok(criar(cc))),
    )
}

/// Abre a janela e roda até ela fechar.
pub fn rodar() -> eframe::Result {
    let amb = Ambiente::padrao(Saida::nada());
    rodar_com(&move |cc| {
        configurar_fontes(&cc.egui_ctx);
        let ctx = cc.egui_ctx.clone();
        let avisar: Box<dyn Fn() + Send + Sync> = Box::new(move || ctx.request_repaint());
        Box::new(App(Janela::nova(&amb, Some(avisar))))
    })
}

pub struct App(pub Janela);

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.0.ui(ui);
    }
}
