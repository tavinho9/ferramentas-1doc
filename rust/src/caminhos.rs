//! Onde o programa guarda os próprios arquivos e onde entrega as planilhas.
//!
//! Sessão, caches, configuração e logs vão para a pasta de dados do usuário
//! (a mesma da versão Python, para as duas compartilharem sessão e caches):
//!
//! ```text
//!     Windows:  %LOCALAPPDATA%\Ferramentas1Doc
//!     outros:   ~/.ferramentas1doc
//! ```
//!
//! As planilhas saem na pasta de saída configurada (padrão: a Área de
//! Trabalho real, inclusive quando redirecionada para o OneDrive).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::saida::Saida;

pub fn pasta_pessoal() -> PathBuf {
    #[cfg(windows)]
    {
        if let Some(p) = std::env::var_os("USERPROFILE") {
            return PathBuf::from(p);
        }
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Pasta de dados padrão (sem criá-la).
pub fn pasta_dados_padrao() -> PathBuf {
    if cfg!(windows) {
        let base = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| pasta_pessoal().join("AppData").join("Local"));
        base.join("Ferramentas1Doc")
    } else {
        pasta_pessoal().join(".ferramentas1doc")
    }
}

/// Expande `%VAR%` (os valores do registro vêm como REG_EXPAND_SZ).
pub fn expandir_variaveis(texto: &str) -> String {
    let mut saida = String::new();
    let mut resto = texto;
    while let Some(i) = resto.find('%') {
        saida.push_str(&resto[..i]);
        let depois = &resto[i + 1..];
        match depois.find('%') {
            Some(j) if j > 0 => {
                let nome = &depois[..j];
                match std::env::var(nome) {
                    Ok(v) => saida.push_str(&v),
                    Err(_) => {
                        saida.push('%');
                        saida.push_str(nome);
                        saida.push('%');
                    }
                }
                resto = &depois[j + 1..];
            }
            _ => {
                saida.push('%');
                resto = depois;
            }
        }
    }
    saida.push_str(resto);
    saida
}

/// Pasta real da Área de Trabalho. No Windows lê o registro, que já aponta
/// para o OneDrive quando a pasta é redirecionada; no Linux usa o
/// xdg-user-dir. Se nada existir, cai na pasta pessoal.
pub fn area_de_trabalho() -> PathBuf {
    let mut candidatos: Vec<PathBuf> = Vec::new();
    #[cfg(windows)]
    {
        use winreg::RegKey;
        use winreg::enums::HKEY_CURRENT_USER;
        let chave = r"Software\Microsoft\Windows\CurrentVersion\Explorer\User Shell Folders";
        if let Ok(k) = RegKey::predef(HKEY_CURRENT_USER).open_subkey(chave)
            && let Ok(valor) = k.get_value::<String, _>("Desktop")
        {
            candidatos.push(PathBuf::from(expandir_variaveis(&valor)));
        }
        let perfil = pasta_pessoal();
        let mut onedrives: Vec<PathBuf> = Vec::new();
        if let Some(od) = std::env::var_os("OneDrive") {
            onedrives.push(PathBuf::from(od));
        }
        onedrives.push(perfil.join("OneDrive"));
        for od in onedrives {
            candidatos.push(od.join("Desktop"));
            candidatos.push(od.join("Área de Trabalho"));
        }
    }
    #[cfg(not(windows))]
    {
        if let Ok(s) = Command::new("xdg-user-dir")
            .arg("DESKTOP")
            .stderr(Stdio::null())
            .output()
        {
            let t = String::from_utf8_lossy(&s.stdout).trim().to_string();
            if !t.is_empty() {
                candidatos.push(PathBuf::from(t));
            }
        }
    }
    let casa = pasta_pessoal();
    candidatos.push(casa.join("Desktop"));
    candidatos.push(casa.join("Área de Trabalho"));
    candidatos.into_iter().find(|c| c.is_dir()).unwrap_or(casa)
}

/// Abre no programa padrão do sistema (Excel, no Windows) ou, se for pasta,
/// no Explorador de Arquivos.
pub fn abrir_arquivo(saida: &Saida, caminho: &Path) {
    let r = if cfg!(windows) {
        opener::open(caminho).map_err(|e| e.to_string())
    } else {
        let prog = if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        Command::new(prog)
            .arg(caminho)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    };
    if let Err(e) = r {
        saida.linha(&format!(
            "  ⚠ Não consegui abrir '{}' ({e}); abra pelo caminho acima.",
            caminho.display()
        ));
    }
}
