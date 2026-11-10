//! Argumentos da busca (o `montar_parser()` do busca.py), com as mesmas
//! opções, padrões e abreviações. Também lê `@arquivo` (um argumento por linha).

use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use clap::{ArgAction, Parser};

use crate::erro::{Erro, Res};
use crate::texto::normtxt;
use crate::{datas, re};

pub const EPILOGO: &str = r#"Exemplos (no programa, escreva só as opções no campo "Outras opções")
  Em Pesquisa de Mercado agora (caixa Em aberto):
    --etapa "Pesquisa de Mercado"
  O que mudou de etapa em outubro, em todas as caixas:
    --caixas todas --mudanca-de 01/10/2026 --mudanca-ate 31/10/2026
  Quem foi para Correção de Documentação nos últimos 7 dias:
    --mudanca-para "Correção de Documentação" --mudanca-de -7
  Entraram na etapa atual hoje:
    --entrada-de hoje
  Fase de Abertura há 15+ dias na mesma etapa, urgentes primeiro:
    --fase Abertura --dias-etapa-min 15 --ordenar prioridade,dias_etapa:desc
  Regra da CPPFI (antes da Pesquisa de Mercado, ou depois mas com o último despacho citando a CPPFI):
    --antes-de "Pesquisa de Mercado" --excecao-cita-setor SPOT-DCL-GP-CPPFI --pessoa "João Otávio"
  Processos de uma planilha/lista, sem varrer as caixas:
    --numeros-arquivo acompanhamento.xlsx
  Refiltrar sem acessar o 1Doc (usa o cache da última coleta):
    --offline --etapa "Validação de Requisição"
  Parâmetros num arquivo (um por linha):  @consulta_cppfi.txt

Datas: DD/MM/AAAA, AAAA-MM-DD, hoje, ontem ou -N (N dias atrás). Limites inclusivos.
Textos: sem distinção de maiúsculas e acentos; "contém". Valores repetidos de um mesmo filtro = OU;
filtros diferentes = E."#;

const MODELO: &str = "{before-help}uso: {usage}\n\n{about}\n\n{all-args}{after-help}";

fn tipo_fase(s: &str) -> Result<String, String> {
    let n = normtxt(s);
    if n.starts_with("elab") || n.starts_with("prep") {
        return Ok("Elaboração".into());
    }
    if n.starts_with("abert") {
        return Ok("Abertura".into());
    }
    Err("fase: Elaboração ou Abertura".into())
}

#[derive(Parser, Debug, Clone)]
#[command(
    name = "busca",
    about = "Busca processos no 1Doc (PMC) por etapa, fase, datas e mudanças de etapa. Somente leitura.",
    after_help = EPILOGO,
    help_template = MODELO,
    disable_help_flag = true,
    disable_version_flag = true,
    infer_long_args = true,
    term_width = 110,
    override_usage = "busca [opções]",
    allow_negative_numbers = true,
    next_help_heading = "Sessão (o login é sempre feito por você, na janela do navegador)"
)]
pub struct Args {
    #[arg(short = 'h', long = "help", action = ArgAction::Help, help = "mostra esta ajuda e sai")]
    pub ajuda: Option<bool>,

    /// ignora a sessão salva e abre o navegador para novo login
    #[arg(long)]
    pub relogar: bool,
    /// padrão: o das Configurações do programa
    #[arg(long, value_parser = ["auto", "chrome", "edge"])]
    pub navegador: Option<String>,
    /// segundos esperando o login (padrão: o das Configurações)
    #[arg(long = "timeout-login", value_name = "S")]
    pub timeout_login: Option<i64>,
    // Aceitos por compatibilidade com consultas antigas (@arquivo), sem efeito.
    #[arg(long, hide = true, default_value = "https://montesclaros.1doc.com.br/")]
    pub base: String,
    #[arg(long, hide = true)]
    pub cookies: Option<String>,
    #[arg(long, hide = true)]
    pub perfil: Option<String>,

    /// entrada, saida, arquivo, favoritos (vírgula) ou todas
    #[arg(
        long,
        default_value = "entrada",
        help_heading = "Universo (o que coletar)"
    )]
    pub caixas: String,
    /// processos específicos: 31.632/2026,5712/2025
    #[arg(long, value_name = "N", action = ArgAction::Append, help_heading = "Universo (o que coletar)")]
    pub numeros: Vec<String>,
    /// .txt/.csv/.xlsx de onde extrair os números NN.NNN/AAAA
    #[arg(long = "numeros-arquivo", value_name = "ARQ", action = ArgAction::Append, help_heading = "Universo (o que coletar)")]
    pub numeros_arquivo: Vec<String>,
    /// tipo de documento (3 = Proc. Administrativo; ex.: 3,24)
    #[arg(long, default_value = "3", help_heading = "Universo (o que coletar)")]
    pub itd: String,
    #[arg(
        long = "max-paginas",
        default_value_t = 200,
        value_name = "MAX_PAGINAS",
        help_heading = "Universo (o que coletar)"
    )]
    pub max_paginas: i64,

    /// etapa atual contém TXT
    #[arg(long, value_name = "TXT", action = ArgAction::Append, help_heading = "Filtros de etapa e posição")]
    pub etapa: Vec<String>,
    /// --etapa/--excluir-etapa exigem igualdade
    #[arg(long = "etapa-exata", help_heading = "Filtros de etapa e posição")]
    pub etapa_exata: bool,
    #[arg(long = "excluir-etapa", value_name = "TXT", action = ArgAction::Append, help_heading = "Filtros de etapa e posição")]
    pub excluir_etapa: Vec<String>,
    /// Elaboração ou Abertura
    #[arg(long, value_name = "FASE", value_parser = tipo_fase, help_heading = "Filtros de etapa e posição")]
    pub fase: Option<String>,
    /// quadro/assunto do 1Doc contém TXT
    #[arg(long, value_name = "TXT", action = ArgAction::Append, help_heading = "Filtros de etapa e posição")]
    pub quadro: Vec<String>,
    /// etapa atual anterior a ETAPA no quadro
    #[arg(
        long = "antes-de",
        value_name = "ETAPA",
        help_heading = "Filtros de etapa e posição"
    )]
    pub antes_de: Option<String>,
    /// etapa atual igual ou posterior a ETAPA
    #[arg(
        long = "a-partir-de",
        value_name = "ETAPA",
        help_heading = "Filtros de etapa e posição"
    )]
    pub a_partir_de: Option<String>,
    /// já esteve em ETAPA (histórico)
    #[arg(long = "passou-por", value_name = "ETAPA", action = ArgAction::Append, help_heading = "Filtros de etapa e posição")]
    pub passou_por: Vec<String>,
    #[arg(long = "nao-passou-por", value_name = "ETAPA", action = ArgAction::Append, help_heading = "Filtros de etapa e posição")]
    pub nao_passou_por: Vec<String>,
    /// setor responsável pela etapa atual
    #[arg(long = "setor-etapa", value_name = "COD", action = ArgAction::Append, help_heading = "Filtros de etapa e posição")]
    pub setor_etapa: Vec<String>,
    /// etapa atual reservada ao setor logado (pelo id do setor no quadro, não pelo texto)
    #[arg(
        long = "etapa-do-meu-setor",
        help_heading = "Filtros de etapa e posição"
    )]
    pub etapa_do_meu_setor: bool,
    #[arg(
        long = "dias-etapa-min",
        value_name = "N",
        help_heading = "Filtros de etapa e posição"
    )]
    pub dias_etapa_min: Option<i64>,
    #[arg(
        long = "dias-etapa-max",
        value_name = "N",
        help_heading = "Filtros de etapa e posição"
    )]
    pub dias_etapa_max: Option<i64>,

    /// entrou na etapa ATUAL a partir de DATA
    #[arg(
        long = "entrada-de",
        value_name = "DATA",
        help_heading = "Datas e momento de mudança de etapa"
    )]
    pub entrada_de: Option<String>,
    #[arg(
        long = "entrada-ate",
        value_name = "DATA",
        help_heading = "Datas e momento de mudança de etapa"
    )]
    pub entrada_ate: Option<String>,
    /// teve mudança de etapa a partir de DATA
    #[arg(
        long = "mudanca-de",
        value_name = "DATA",
        help_heading = "Datas e momento de mudança de etapa"
    )]
    pub mudanca_de: Option<String>,
    #[arg(
        long = "mudanca-ate",
        value_name = "DATA",
        help_heading = "Datas e momento de mudança de etapa"
    )]
    pub mudanca_ate: Option<String>,
    /// …com destino a ETAPA
    #[arg(long = "mudanca-para", value_name = "ETAPA", action = ArgAction::Append, help_heading = "Datas e momento de mudança de etapa")]
    pub mudanca_para: Vec<String>,
    /// …saindo de ETAPA
    #[arg(long = "mudanca-origem", value_name = "ETAPA", action = ArgAction::Append, help_heading = "Datas e momento de mudança de etapa")]
    pub mudanca_origem: Vec<String>,
    /// criado a partir de DATA (filtra no servidor)
    #[arg(
        long = "emissao-de",
        value_name = "DATA",
        help_heading = "Datas e momento de mudança de etapa"
    )]
    pub emissao_de: Option<String>,
    #[arg(
        long = "emissao-ate",
        value_name = "DATA",
        help_heading = "Datas e momento de mudança de etapa"
    )]
    pub emissao_ate: Option<String>,
    #[arg(
        long = "ult-mov-de",
        value_name = "DATA",
        help_heading = "Datas e momento de mudança de etapa"
    )]
    pub ult_mov_de: Option<String>,
    #[arg(
        long = "ult-mov-ate",
        value_name = "DATA",
        help_heading = "Datas e momento de mudança de etapa"
    )]
    pub ult_mov_ate: Option<String>,
    /// dias sem movimentação ≥ N
    #[arg(
        long = "sem-mov-min",
        value_name = "N",
        help_heading = "Datas e momento de mudança de etapa"
    )]
    pub sem_mov_min: Option<i64>,
    #[arg(
        long = "sem-mov-max",
        value_name = "N",
        help_heading = "Datas e momento de mudança de etapa"
    )]
    pub sem_mov_max: Option<i64>,

    /// Urgente, Alta, Média, Baixa (vírgula)
    #[arg(long, value_name = "P", action = ArgAction::Append, help_heading = "Outros filtros")]
    pub prioridade: Vec<String>,
    /// setor entre os envolvidos da coluna 'Para' da caixa (código inteiro ou último nível)
    #[arg(long, value_name = "COD", action = ArgAction::Append, help_heading = "Outros filtros")]
    pub com: Vec<String>,
    /// objeto contém TXT
    #[arg(long, value_name = "TXT", action = ArgAction::Append, help_heading = "Outros filtros")]
    pub objeto: Vec<String>,
    #[arg(long, value_name = "TXT", action = ArgAction::Append, help_heading = "Outros filtros")]
    pub situacao: Vec<String>,
    /// últimos N despachos citam o setor (ex.: SPOT-DCL-GP-CPPFI)
    #[arg(
        long = "cita-setor",
        value_name = "COD",
        help_heading = "Outros filtros"
    )]
    pub cita_setor: Option<String>,
    /// variantes de nome que contam como o setor
    #[arg(long, value_name = "NOME", action = ArgAction::Append, help_heading = "Outros filtros")]
    pub pessoa: Vec<String>,
    #[arg(
        long = "ultimos-despachos",
        value_name = "N",
        default_value_t = 1,
        help_heading = "Outros filtros"
    )]
    pub ultimos_despachos: i64,
    /// inclui quem falhou só nos filtros de posição se os últimos N despachos citarem COD
    #[arg(
        long = "excecao-cita-setor",
        value_name = "COD",
        help_heading = "Outros filtros"
    )]
    pub excecao_cita_setor: Option<String>,

    /// padrão: 'Busca 1Doc DD-MM-AAAA HHhMM.xlsx' na pasta de saída das Configurações
    #[arg(long, value_name = "ARQ.xlsx", help_heading = "Saída e desempenho")]
    pub saida: Option<String>,
    /// abre a planilha ao terminar
    #[arg(long, help_heading = "Saída e desempenho")]
    pub abrir: bool,
    /// também grava CSV (separador ;)
    #[arg(long, value_name = "ARQ.csv", help_heading = "Saída e desempenho")]
    pub csv: Option<String>,
    #[arg(long = "sem-xlsx", help_heading = "Saída e desempenho")]
    pub sem_xlsx: bool,
    /// campos: numero, emissao, prioridade, fase, etapa, entrada, dias_etapa, ult_mov, dias_sem_mov (sufixo :desc)
    #[arg(
        long,
        default_value = "fase,etapa,dias_etapa:desc",
        help_heading = "Saída e desempenho"
    )]
    pub ordenar: String,
    /// linhas no terminal
    #[arg(
        long,
        value_name = "N",
        default_value_t = 30,
        help_heading = "Saída e desempenho"
    )]
    pub mostrar: i64,
    /// cache de detalhes (padrão: 1doc_cache.json na pasta de dados)
    #[arg(long, help_heading = "Saída e desempenho")]
    pub cache: Option<String>,
    #[arg(long = "sem-cache", help_heading = "Saída e desempenho")]
    pub sem_cache: bool,
    /// não acessa o 1Doc: filtra o cache
    #[arg(long, help_heading = "Saída e desempenho")]
    pub offline: bool,
    /// reabre todos os processos, ignorando o cache
    #[arg(long, help_heading = "Saída e desempenho")]
    pub refazer: bool,
    /// só a listagem (rápido; sem etapa)
    #[arg(long = "sem-detalhe", help_heading = "Saída e desempenho")]
    pub sem_detalhe: bool,
    /// processos abertos em paralelo
    #[arg(
        long,
        value_name = "N",
        default_value_t = 3,
        help_heading = "Saída e desempenho"
    )]
    pub paralelo: i64,
    /// pausa entre requisições por conexão
    #[arg(
        long,
        value_name = "S",
        default_value_t = 0.15,
        help_heading = "Saída e desempenho"
    )]
    pub pausa: f64,
}

/// Argumentos já normalizados: datas resolvidas contra a data de posição.
#[derive(Debug, Clone)]
pub struct Opcoes {
    pub a: Args,
    pub entrada_de: Option<NaiveDateTime>,
    pub entrada_ate: Option<NaiveDateTime>,
    pub mudanca_de: Option<NaiveDateTime>,
    pub mudanca_ate: Option<NaiveDateTime>,
    pub emissao_de: Option<NaiveDateTime>,
    pub emissao_ate: Option<NaiveDateTime>,
    pub ult_mov_de: Option<NaiveDateTime>,
    pub ult_mov_ate: Option<NaiveDateTime>,
    /// id do setor logado (da sessão; no --offline, o da última coleta)
    pub id_setor: String,
}

/// DD/MM/AAAA, AAAA-MM-DD, 'hoje', 'ontem' ou -N (N dias atrás). `fim`
/// devolve 23:59:59 do dia (limite superior inclusivo).
pub fn data_cli(s: &str, hoje: NaiveDate, fim: bool) -> Result<NaiveDateTime, String> {
    let t = s.trim().to_lowercase();
    let invalida = || format!("data inválida: '{s}' (use DD/MM/AAAA, hoje, ontem ou -N)");
    let d = if t == "hoje" {
        hoje
    } else if t == "ontem" {
        datas::ontem(hoje)
    } else if re!(r"^-\d+$").is_match(&t) {
        datas::dias_atras(hoje, &t[1..]).ok_or_else(invalida)?
    } else if let Some(m) = re!(r"^(\d{4})-(\d{2})-(\d{2})$").captures(&t) {
        NaiveDate::from_ymd_opt(
            m[1].parse().unwrap_or(0),
            m[2].parse().unwrap_or(0),
            m[3].parse().unwrap_or(0),
        )
        .ok_or_else(invalida)?
    } else if let Some(m) = re!(r"^(\d{1,2})/(\d{1,2})/(\d{4})$").captures(&t) {
        NaiveDate::from_ymd_opt(
            m[3].parse().unwrap_or(0),
            m[2].parse().unwrap_or(0),
            m[1].parse().unwrap_or(0),
        )
        .ok_or_else(invalida)?
    } else {
        return Err(invalida());
    };
    let hora = if fim {
        NaiveTime::from_hms_opt(23, 59, 59).unwrap()
    } else {
        NaiveTime::MIN
    };
    Ok(d.and_time(hora))
}

/// `_lista`: valores repetidos e separados por vírgula → lista.
pub fn lista(v: &[String]) -> Vec<String> {
    v.iter()
        .flat_map(|x| x.split(','))
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect()
}

/// Lê `@arquivo` (um argumento por linha), como o `fromfile_prefix_chars="@"`.
fn expandir_arquivos(argv: &[String]) -> Res<Vec<String>> {
    let mut out = Vec::new();
    for a in argv {
        if let Some(c) = a.strip_prefix('@').filter(|c| !c.is_empty()) {
            let t = std::fs::read_to_string(c)
                .map_err(|e| Erro::encerrar(format!("busca: error: [Errno 2] {e}: '{c}'")))?;
            let linhas: Vec<String> = crate::texto::linhas(&t)
                .into_iter()
                .map(str::to_string)
                .collect();
            out.extend(expandir_arquivos(&linhas)?);
        } else {
            out.push(a.clone());
        }
    }
    Ok(out)
}

/// Mensagem de erro do clap em português, numa linha.
fn traduzir(e: &clap::Error) -> String {
    use clap::error::{ContextKind, ContextValue, ErrorKind};
    let ctx = |k: ContextKind| {
        e.get(k).map(|v| match v {
            ContextValue::String(s) => s.clone(),
            ContextValue::Strings(s) => s.join(", "),
            outro => outro.to_string(),
        })
    };
    let arg = ctx(ContextKind::InvalidArg).unwrap_or_default();
    let arg = arg.split_whitespace().next().unwrap_or("").to_string();
    let valor = ctx(ContextKind::InvalidValue).unwrap_or_default();
    match e.kind() {
        ErrorKind::UnknownArgument => format!("argumentos não reconhecidos: {arg}"),
        ErrorKind::InvalidValue if valor.is_empty() => {
            format!("argumento {arg}: precisa de um valor")
        }
        ErrorKind::InvalidValue => {
            let validos = ctx(ContextKind::ValidValue)
                .map(|v| format!(" (opções: {v})"))
                .unwrap_or_default();
            format!("argumento {arg}: valor inválido: '{valor}'{validos}")
        }
        ErrorKind::ValueValidation => {
            let causa = std::error::Error::source(e)
                .map(|c| c.to_string())
                .unwrap_or_default();
            if causa.starts_with("fase") {
                format!("argumento {arg}: {causa}")
            } else {
                format!("argumento {arg}: valor inválido: '{valor}'")
            }
        }
        ErrorKind::NoEquals | ErrorKind::TooFewValues | ErrorKind::WrongNumberOfValues => {
            format!("argumento {arg}: precisa de um valor")
        }
        ErrorKind::ArgumentConflict => {
            format!("argumento {arg}: não pode ser usado com outro já informado")
        }
        _ => {
            let t = e.to_string();
            t.lines()
                .next()
                .unwrap_or("argumentos inválidos")
                .trim_start_matches("error: ")
                .to_string()
        }
    }
}

/// O que acontece ao analisar os argumentos: opções prontas, a ajuda pedida
/// (`-h`), ou um erro com a mensagem para a pessoa.
pub enum Analise {
    Ok(Box<Args>),
    Ajuda(String),
}

/// Os trechos em inglês da ajuda do clap, em português.
fn em_portugues(t: &str) -> String {
    let t = re!(r"\[default: ([^\]]*)\]").replace_all(t, "(padrão: $1)");
    let t = re!(r"\[possible values: ([^\]]*)\]").replace_all(&t, "(opções: $1)");
    t.into_owned()
}

pub fn analisar(argv: &[String]) -> Res<Analise> {
    let mut v = vec!["busca".to_string()];
    v.extend(expandir_arquivos(argv)?);
    match Args::try_parse_from(&v) {
        Ok(a) => Ok(Analise::Ok(Box::new(a))),
        Err(e) if e.kind() == clap::error::ErrorKind::DisplayHelp => {
            Ok(Analise::Ajuda(em_portugues(&e.to_string())))
        }
        Err(e) => Err(Erro::encerrar(traduzir(&e))),
    }
}

/// Texto do `--help` da busca.
pub fn ajuda() -> String {
    use clap::CommandFactory;
    em_portugues(&Args::command().render_help().to_string())
}

/// Resolve as datas contra a data de posição da execução (hoje, ontem, -N
/// são relativos a ela) e limpa as listas (`normalizar_args`).
pub fn normalizar(mut a: Args, hoje: NaiveDate) -> Res<Opcoes> {
    let data = |nome: &str, v: &Option<String>, fim: bool| -> Res<Option<NaiveDateTime>> {
        match v.as_deref().filter(|s| !s.is_empty()) {
            None => Ok(None),
            Some(s) => data_cli(s, hoje, fim)
                .map(Some)
                .map_err(|e| Erro::encerrar(format!("--{nome}: {e}"))),
        }
    };
    let entrada_de = data("entrada-de", &a.entrada_de, false)?;
    let mudanca_de = data("mudanca-de", &a.mudanca_de, false)?;
    let emissao_de = data("emissao-de", &a.emissao_de, false)?;
    let ult_mov_de = data("ult-mov-de", &a.ult_mov_de, false)?;
    let entrada_ate = data("entrada-ate", &a.entrada_ate, true)?;
    let mudanca_ate = data("mudanca-ate", &a.mudanca_ate, true)?;
    let emissao_ate = data("emissao-ate", &a.emissao_ate, true)?;
    let ult_mov_ate = data("ult-mov-ate", &a.ult_mov_ate, true)?;
    a.prioridade = lista(&a.prioridade);
    for v in [
        &mut a.etapa,
        &mut a.excluir_etapa,
        &mut a.quadro,
        &mut a.passou_por,
        &mut a.nao_passou_por,
        &mut a.setor_etapa,
        &mut a.mudanca_para,
        &mut a.mudanca_origem,
        &mut a.com,
        &mut a.objeto,
        &mut a.situacao,
        &mut a.pessoa,
    ] {
        *v = v
            .iter()
            .map(|x| x.trim().to_string())
            .filter(|x| !x.is_empty())
            .collect();
    }
    if a.offline && a.sem_cache {
        return Err(Erro::encerrar(
            "--offline precisa do cache; retire --sem-cache.",
        ));
    }
    Ok(Opcoes {
        a,
        entrada_de,
        entrada_ate,
        mudanca_de,
        mudanca_ate,
        emissao_de,
        emissao_ate,
        ult_mov_de,
        ult_mov_ate,
        id_setor: String::new(),
    })
}
