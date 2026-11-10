//! Saída das ferramentas (o `print` dos scripts) → janela, terminal e log.
//!
//! As ferramentas escrevem com `msg!`/`msg_sem_quebra!`, inclusive com `\r`
//! para atualizar uma linha de progresso ("página 3/12…"). Na janela, cada
//! pedaço vai para um canal que ela consome; no programa de linha de
//! comando, para o stdout. Durante uma execução da janela, tudo é espelhado
//! no `.log` dela (com cada atualização de progresso numa linha).

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

type Avisar = Box<dyn Fn() + Send + Sync>;

enum Destino {
    Stdout,
    Fila(Mutex<Sender<String>>, Option<Avisar>),
    Memoria(Mutex<String>),
    Nada,
}

struct Interno {
    destino: Destino,
    arquivo: Mutex<Option<File>>,
}

#[derive(Clone)]
pub struct Saida(Arc<Interno>);

impl Saida {
    fn nova(destino: Destino) -> Self {
        Saida(Arc::new(Interno {
            destino,
            arquivo: Mutex::new(None),
        }))
    }

    /// Terminal (programa de linha de comando).
    pub fn stdout() -> Self {
        Self::nova(Destino::Stdout)
    }

    /// Fila consumida pela janela; `avisar` acorda a janela (request_repaint).
    pub fn fila(tx: Sender<String>, avisar: Option<Avisar>) -> Self {
        Self::nova(Destino::Fila(Mutex::new(tx), avisar))
    }

    /// Guarda tudo em memória (testes: `capsys`).
    pub fn memoria() -> Self {
        Self::nova(Destino::Memoria(Mutex::new(String::new())))
    }

    pub fn nada() -> Self {
        Self::nova(Destino::Nada)
    }

    /// Texto acumulado (só para `Saida::memoria`).
    pub fn texto(&self) -> String {
        match &self.0.destino {
            Destino::Memoria(m) => m.lock().unwrap().clone(),
            _ => String::new(),
        }
    }

    pub fn escrever(&self, texto: &str) {
        if texto.is_empty() {
            return;
        }
        match &self.0.destino {
            Destino::Stdout => {
                let mut o = std::io::stdout().lock();
                let _ = o.write_all(texto.as_bytes());
                let _ = o.flush();
            }
            Destino::Fila(tx, avisar) => {
                let _ = tx.lock().unwrap().send(texto.to_string());
                if let Some(a) = avisar {
                    a();
                }
            }
            Destino::Memoria(m) => m.lock().unwrap().push_str(texto),
            Destino::Nada => {}
        }
        let mut arq = self.0.arquivo.lock().unwrap();
        if let Some(f) = arq.as_mut() {
            let t = texto.replace("\r\n", "\n").replace('\r', "\n");
            let _ = f.write_all(t.as_bytes()).and_then(|_| f.flush());
        }
    }

    /// `print(texto)`.
    pub fn linha(&self, texto: &str) {
        let mut t = String::with_capacity(texto.len() + 1);
        t.push_str(texto);
        t.push('\n');
        self.escrever(&t);
    }

    /// Começa a espelhar a saída no arquivo `caminho` (None = parar).
    pub fn gravar_em(&self, caminho: Option<&Path>) -> std::io::Result<()> {
        let mut arq = self.0.arquivo.lock().unwrap();
        *arq = None;
        if let Some(c) = caminho {
            *arq = Some(OpenOptions::new().create(true).append(true).open(c)?);
        }
        Ok(())
    }

    /// Só no arquivo (ex.: detalhes técnicos), não na janela.
    pub fn gravar_no_log(&self, texto: &str) {
        let mut arq = self.0.arquivo.lock().unwrap();
        if let Some(f) = arq.as_mut() {
            let _ = f.write_all(texto.as_bytes()).and_then(|_| f.flush());
        }
    }
}

/// `print(...)` das ferramentas: `msg!(amb, "texto {x}")`.
#[macro_export]
macro_rules! msg {
    ($amb:expr) => { $amb.saida.linha("") };
    ($amb:expr, $($arg:tt)*) => { $amb.saida.linha(&format!($($arg)*)) };
}

/// `print(..., end="", flush=True)`.
#[macro_export]
macro_rules! msg_sem_quebra {
    ($amb:expr, $($arg:tt)*) => { $amb.saida.escrever(&format!($($arg)*)) };
}

/// Aplica pedaços de texto com `\r` e `\n` como um terminal: `\r` sem `\n`
/// faz o próximo texto substituir a linha atual. As operações reais (apagar
/// a linha corrente, inserir texto) ficam com quem usa; assim a regra é
/// testável sem janela.
#[derive(Default)]
pub struct Terminal {
    voltar: bool,
}

pub trait Tela {
    fn apagar_linha(&mut self);
    fn inserir(&mut self, texto: &str);
}

impl Terminal {
    pub fn alimentar(&mut self, texto: &str, tela: &mut impl Tela) {
        let mut resto = texto;
        while !resto.is_empty() {
            let corte = resto.find(['\r', '\n']).unwrap_or(resto.len());
            if corte > 0 {
                if self.voltar {
                    tela.apagar_linha();
                    self.voltar = false;
                }
                tela.inserir(&resto[..corte]);
                resto = &resto[corte..];
                continue;
            }
            if resto.starts_with("\r\n") {
                tela.inserir("\n");
                self.voltar = false;
                resto = &resto[2..];
            } else if resto.starts_with('\n') {
                tela.inserir("\n");
                self.voltar = false;
                resto = &resto[1..];
            } else {
                self.voltar = true;
                resto = &resto[1..];
            }
        }
    }
}

/// Terminal em memória (testes e caixa de mensagens da janela).
#[derive(Default)]
pub struct TelaEmMemoria {
    pub texto: String,
}

impl Tela for TelaEmMemoria {
    fn apagar_linha(&mut self) {
        let inicio = self.texto.rfind('\n').map(|i| i + 1).unwrap_or(0);
        self.texto.truncate(inicio);
    }

    fn inserir(&mut self, texto: &str) {
        self.texto.push_str(texto);
    }
}
