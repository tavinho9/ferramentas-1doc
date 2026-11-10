//! Leitura e gravação dos arquivos JSON da pasta de dados.

use std::path::Path;

use serde_json::Value;

/// Grava `texto` em `caminho` sem deixar um arquivo pela metade: escreve num
/// temporário ao lado e troca pelo definitivo.
pub fn gravar_atomico(caminho: &Path, texto: &str) -> std::io::Result<()> {
    let pasta = caminho.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(pasta)?;
    let mut tmp = tempfile::NamedTempFile::new_in(pasta)?;
    std::io::Write::write_all(&mut tmp, texto.as_bytes())?;
    tmp.as_file().sync_all().ok();
    match tmp.persist(caminho) {
        Ok(_) => Ok(()),
        Err(e) => {
            // No Windows o destino pode estar aberto por outro programa: grava direto.
            drop(e.file);
            std::fs::write(caminho, texto)
        }
    }
}

/// JSON do arquivo; None se não existir ou não for JSON válido.
pub fn ler_json(caminho: &Path) -> Option<Value> {
    let bytes = std::fs::read(caminho).ok()?;
    let texto = String::from_utf8_lossy(&bytes);
    let texto = texto.strip_prefix('\u{feff}').unwrap_or(&texto);
    serde_json::from_str(texto).ok()
}

/// JSON como o `json.dump(..., ensure_ascii=False, indent=2)` do Python.
pub fn json_indentado(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_default()
}

/// JSON compacto como o `json.dump(..., ensure_ascii=False)` do Python
/// (`", "` e `": "` como separadores).
pub fn json_python(v: &Value) -> String {
    let mut s = String::new();
    escrever_python(v, &mut s);
    s
}

fn escrever_python(v: &Value, s: &mut String) {
    match v {
        Value::Array(a) => {
            s.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    s.push_str(", ");
                }
                escrever_python(x, s);
            }
            s.push(']');
        }
        Value::Object(o) => {
            s.push('{');
            for (i, (k, x)) in o.iter().enumerate() {
                if i > 0 {
                    s.push_str(", ");
                }
                s.push_str(&serde_json::to_string(k).unwrap_or_default());
                s.push_str(": ");
                escrever_python(x, s);
            }
            s.push('}');
        }
        outro => s.push_str(&serde_json::to_string(outro).unwrap_or_default()),
    }
}

/// Valor "verdadeiro" no sentido do Python (`bool(x)`).
pub fn verdadeiro(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|x| x != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// Texto de um campo, "" se faltar ou não for texto.
pub fn texto(v: &Value, chave: &str) -> String {
    v.get(chave)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}
