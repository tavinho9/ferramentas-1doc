"""Onde o programa guarda os próprios arquivos e onde entrega as planilhas.

Instalado, o programa fica numa pasta que não é lugar de dados (e, empacotado,
o __file__ dos módulos aponta para dentro do pacote). Por isso sessão, caches,
configuração e logs vão para a pasta de dados do usuário:

    Windows:  %LOCALAPPDATA%\\Ferramentas1Doc
    outros:   ~/.ferramentas1doc

As planilhas saem na pasta de saída configurada (padrão: a Área de Trabalho
real, inclusive quando redirecionada para o OneDrive).
"""

import os
import subprocess
import sys


def pasta_dados():
    if os.name == "nt":
        base = os.environ.get("LOCALAPPDATA") or os.path.join(os.path.expanduser("~"), "AppData", "Local")
        pasta = os.path.join(base, "Ferramentas1Doc")
    else:
        pasta = os.path.join(os.path.expanduser("~"), ".ferramentas1doc")
    os.makedirs(pasta, exist_ok=True)
    return pasta


def arquivo_dados(nome):
    """Caminho de um arquivo dentro da pasta de dados."""
    return os.path.join(pasta_dados(), nome)


def pasta_logs():
    pasta = os.path.join(pasta_dados(), "logs")
    os.makedirs(pasta, exist_ok=True)
    return pasta


def pasta_perfil_navegador():
    """Perfil próprio do navegador usado no login (não mexe no perfil pessoal)."""
    return os.path.join(pasta_dados(), "navegador")


def area_de_trabalho():
    """Pasta real da Área de Trabalho. No Windows lê o registro, que já
    aponta para o OneDrive quando a pasta é redirecionada; no Linux usa o
    xdg-user-dir. Se nada existir, cai na pasta pessoal."""
    candidatos = []
    if os.name == "nt":
        try:
            import winreg
            chave = r"Software\Microsoft\Windows\CurrentVersion\Explorer\User Shell Folders"
            with winreg.OpenKey(winreg.HKEY_CURRENT_USER, chave) as k:
                valor, _ = winreg.QueryValueEx(k, "Desktop")
            candidatos.append(os.path.expandvars(valor))
        except OSError:
            pass
        perfil = os.environ.get("USERPROFILE", os.path.expanduser("~"))
        for od in (os.environ.get("OneDrive"), os.path.join(perfil, "OneDrive")):
            if od:
                candidatos += [os.path.join(od, "Desktop"), os.path.join(od, "Área de Trabalho")]
    else:
        try:
            saida = subprocess.run(["xdg-user-dir", "DESKTOP"], capture_output=True,
                                   text=True, timeout=5).stdout.strip()
            if saida:
                candidatos.append(saida)
        except (OSError, subprocess.SubprocessError):
            pass
    casa = os.path.expanduser("~")
    candidatos += [os.path.join(casa, "Desktop"), os.path.join(casa, "Área de Trabalho")]
    return next((c for c in candidatos if os.path.isdir(c)), casa)


def abrir_arquivo(caminho):
    """Abre no programa padrão do sistema (Excel, no Windows) ou, se for
    pasta, no Explorador de Arquivos."""
    try:
        if os.name == "nt":
            os.startfile(caminho)
        else:
            subprocess.Popen(["open" if sys.platform == "darwin" else "xdg-open", caminho],
                             stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    except OSError as e:
        print(f"  ⚠ Não consegui abrir '{caminho}' ({e}); abra pelo caminho acima.")
