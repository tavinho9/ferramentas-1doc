//! Leitura de HTML (o BeautifulSoup dos scripts) sobre o `scraper`.

use ego_tree::NodeRef;
use scraper::{ElementRef, Html, Node, Selector};

/// Seletor CSS compilado uma vez: `sel!("a.link_emissao_a")`.
#[macro_export]
macro_rules! sel {
    ($css:expr) => {{
        static S: std::sync::LazyLock<scraper::Selector> =
            std::sync::LazyLock::new(|| scraper::Selector::parse($css).expect($css));
        &*S
    }};
}

/// Documento HTML. Fragmentos que começam por linhas de tabela ganham um
/// `<table>` em volta: o parser HTML5 descartaria `<tr>`/`<td>` soltos, que
/// o lxml mantinha.
pub fn doc(html: &str) -> Html {
    let ini = html.trim_start();
    let ini_min: String = ini.chars().take(4).collect::<String>().to_ascii_lowercase();
    if ini_min.starts_with("<tr") || ini_min.starts_with("<td") || ini_min.starts_with("<th") {
        return Html::parse_document(&format!("<table>{html}</table>"));
    }
    Html::parse_document(html)
}

/// Elemento raiz (`<html>`) do documento.
pub fn raiz(d: &Html) -> ElementRef<'_> {
    d.root_element()
}

/// `el.select_one(css)` (o próprio elemento não conta, como no bs4).
pub fn um<'a>(el: ElementRef<'a>, s: &Selector) -> Option<ElementRef<'a>> {
    el.select(s).next()
}

/// `soup.select_one(css)` no documento inteiro.
pub fn um_doc<'a>(d: &'a Html, s: &Selector) -> Option<ElementRef<'a>> {
    d.select(s).next()
}

pub fn todos<'a>(el: ElementRef<'a>, s: &Selector) -> Vec<ElementRef<'a>> {
    el.select(s).collect()
}

pub fn attr<'a>(el: ElementRef<'a>, nome: &str) -> Option<&'a str> {
    el.value().attr(nome)
}

/// `el.get(nome) or ""`.
pub fn attr_ou_vazio<'a>(el: ElementRef<'a>, nome: &str) -> &'a str {
    el.value().attr(nome).unwrap_or("")
}

pub fn tem_attr(el: ElementRef<'_>, nome: &str) -> bool {
    el.value().attr(nome).is_some()
}

pub fn tem_classe(el: ElementRef<'_>, classe: &str) -> bool {
    el.value().classes().any(|c| c == classe)
}

pub fn nome_tag<'a>(el: ElementRef<'a>) -> &'a str {
    el.value().name()
}

/// Strings que o `get_text()` do bs4 ignora: as de script, style e template.
fn conteiner_ignorado(nome: &str) -> bool {
    matches!(nome, "script" | "style" | "template" | "rt" | "rp")
}

fn coletar<'a>(
    no: NodeRef<'a, Node>,
    pular: &dyn Fn(ElementRef<'a>) -> bool,
    partes: &mut Vec<&'a str>,
) {
    for filho in no.children() {
        match filho.value() {
            Node::Text(t) => partes.push(t),
            Node::Element(e) => {
                if conteiner_ignorado(e.name()) {
                    continue;
                }
                if let Some(er) = ElementRef::wrap(filho)
                    && pular(er)
                {
                    continue;
                }
                coletar(filho, pular, partes);
            }
            _ => {}
        }
    }
}

/// `el.get_text(sep)`.
pub fn texto(el: ElementRef<'_>, sep: &str) -> String {
    texto_pulando(el, sep, &|_| false)
}

/// `get_text(sep)` depois de `decompose()` dos descendentes para os quais
/// `pular` é verdadeiro.
pub fn texto_pulando<'a>(
    el: ElementRef<'a>,
    sep: &str,
    pular: &dyn Fn(ElementRef<'a>) -> bool,
) -> String {
    let mut partes = Vec::new();
    coletar(*el, pular, &mut partes);
    partes.join(sep)
}

/// `get_text(sep)` depois de `decompose()` dos descendentes que casam com `s`.
pub fn texto_sem(el: ElementRef<'_>, s: &Selector, sep: &str) -> String {
    let removidos: Vec<_> = el.select(s).map(|e| e.id()).collect();
    texto_pulando(el, sep, &|e| removidos.contains(&e.id()))
}

/// Texto do documento inteiro (`soup.get_text(sep)`).
pub fn texto_doc(d: &Html, sep: &str) -> String {
    let mut partes = Vec::new();
    coletar(d.tree.root(), &|_| false, &mut partes);
    partes.join(sep)
}

/// Só os nós de texto filhos diretos.
pub fn textos_diretos<'a>(el: ElementRef<'a>) -> Vec<&'a str> {
    el.children()
        .filter_map(|n| match n.value() {
            Node::Text(t) => Some(&**t),
            _ => None,
        })
        .collect()
}

/// `str(tag)`: o HTML do elemento.
pub fn html_de(el: ElementRef<'_>) -> String {
    el.html()
}

/// Primeiro ancestral (sem contar o próprio elemento) que satisfaz `pred`
/// (`find_parent`).
pub fn ancestral<'a>(
    el: ElementRef<'a>,
    pred: impl Fn(ElementRef<'a>) -> bool,
) -> Option<ElementRef<'a>> {
    el.ancestors()
        .filter_map(ElementRef::wrap)
        .find(|e| pred(*e))
}

pub fn ancestral_tag<'a>(el: ElementRef<'a>, tag: &str) -> Option<ElementRef<'a>> {
    ancestral(el, |e| e.value().name() == tag)
}

const BLOCOS: [&str; 10] = ["p", "div", "li", "tr", "h1", "h2", "h3", "h4", "h5", "h6"];

fn quebras<'a>(no: NodeRef<'a, Node>, blocos: bool, partes: &mut Vec<&'a str>) {
    for filho in no.children() {
        match filho.value() {
            Node::Text(t) => partes.push(t),
            Node::Element(e) => {
                let nome = e.name();
                if nome == "br" {
                    partes.push("\n");
                    continue;
                }
                if conteiner_ignorado(nome) {
                    continue;
                }
                quebras(filho, blocos, partes);
                if blocos && BLOCOS.contains(&nome) {
                    partes.push("\n");
                }
            }
            _ => {}
        }
    }
}

/// `get_text()` com cada `<br>` trocado por quebra de linha e, com `blocos`,
/// uma quebra no fim de cada bloco (p, div, li, tr, h1–h6).
pub fn texto_com_quebras(el: ElementRef<'_>, blocos: bool) -> String {
    let mut partes = Vec::new();
    quebras(*el, blocos, &mut partes);
    partes.concat()
}

/// `get_text(strip=True)` não vazio.
pub fn tem_texto(el: ElementRef<'_>) -> bool {
    el.text().any(|t| !t.trim().is_empty())
}
