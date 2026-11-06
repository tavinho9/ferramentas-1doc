"""A janela de verdade, rodando uma ferramenta contra o 1Doc de mentira."""

import os
import sys
import time
from datetime import date

import pytest

tk = pytest.importorskip("tkinter")

import fake1doc  # noqa: E402


@pytest.fixture(scope="module")
def _janela_unica():
    """Uma janela só para todos os testes. No Windows, criar um segundo Tk no
    mesmo processo depois de fechar o primeiro às vezes quebra o Tcl
    ("invalid command name tcl_findLibrary"). O programa nunca faz isso: ele
    abre uma única janela por execução."""
    if os.name != "nt" and not os.environ.get("DISPLAY"):
        pytest.skip("sem tela para o Tk (no Linux, rode com xvfb-run)")
    from ferramentas1doc.gui import janela as mod
    from ferramentas1doc.gui.log import SaidaFila
    j = mod.Janela(SaidaFila())
    yield j
    j.destroy()


@pytest.fixture
def janela(_janela_unica, dados, cred, monkeypatch):
    from ferramentas1doc import processos_do_dia, sessao
    from ferramentas1doc.gui import janela as mod

    def obter_sessao(ao_esperar=None, **kw):
        ao_esperar(True)                    # como se o navegador tivesse aberto
        ao_esperar(False)
        sessao.salvar(cred)                 # como o obter_sessao real faz
        return cred
    monkeypatch.setattr(sessao, "obter_sessao", obter_sessao)
    monkeypatch.setattr(processos_do_dia, "PAUSA_REQ", 0)
    fake1doc.instalar(monkeypatch)
    avisos = []
    monkeypatch.setattr(mod.messagebox, "showwarning", lambda *a, **k: avisos.append(a))
    j = _janela_unica
    j.avisos = avisos
    j.ultimo_arquivo = None
    return j


def _esperar_fim(j, limite=30):
    fim = time.monotonic() + limite
    while j.executando is not None and time.monotonic() < fim:
        j.update()
        time.sleep(0.05)
    j.update()
    for _ in range(3):                      # deixa a fila de mensagens esvaziar
        time.sleep(0.12)
        j.update()


def test_executar_processos_do_dia_pela_janela(janela, dados, monkeypatch):
    # no programa, app.py faz isto; aqui, na fase de execução do teste (o
    # pytest reinstala a própria captura de saída entre a preparação e o teste)
    monkeypatch.setattr(sys, "stdout", janela.saida)
    aba = janela.abas[0]
    aba.dia.set("hoje")
    janela._executar(aba)
    assert str(aba.btn_executar["state"]) == "disabled"      # bloqueado enquanto roda
    _esperar_fim(janela)
    texto = janela.texto.get("1.0", "end")
    assert janela.lbl_estado.cget("text") == "Concluído.", texto
    assert "Processos exportados: 1" in texto
    assert janela.ultimo_arquivo == os.path.join(str(dados["saida"]),
                                                 f"processos novos {date.today():%d.%m}.xlsx")
    assert str(janela.btn_abrir["state"]) == "normal"
    assert str(aba.btn_executar["state"]) == "normal"
    assert "Conectado" in janela.lbl_sessao.cget("text")
    logs = os.listdir(os.path.join(str(dados["dados"]), "logs"))
    assert len(logs) == 1 and "Processos exportados: 1" in open(
        os.path.join(str(dados["dados"]), "logs", logs[0]), encoding="utf-8").read()


def test_data_invalida_avisa_sem_executar(janela):
    aba = janela.abas[0]
    aba.dia.set("31/02/2026")
    janela._executar(aba)
    assert janela.executando is None
    assert janela.avisos and "Data não reconhecida" in janela.avisos[0][1]


def test_busca_com_formulario_invalido(janela):
    aba = janela.abas[1]
    for v in aba.caixas.values():
        v.set(False)
    janela._executar(aba)
    assert janela.executando is None and "caixa" in janela.avisos[0][1]


def _processar(j, vezes=6):
    for _ in range(vezes):
        j.update()
        time.sleep(0.05)


def _dialogo_configuracoes(j):
    from ferramentas1doc.gui import janela as mod
    abertos = [w for w in j.winfo_children() if isinstance(w, mod.DialogoConfiguracoes)]
    assert len(abertos) == 1, abertos
    return abertos[0]


def _principal_desabilitada(j):
    return os.name == "nt" and bool(int(j.attributes("-disabled")))


def test_configuracoes_uma_so_e_a_principal_volta_ao_fechar(janela):
    j = janela
    j._configuracoes()
    _processar(j)
    d = _dialogo_configuracoes(j)
    j._configuracoes()                      # clicar de novo traz a mesma, não abre outra
    _processar(j)
    assert _dialogo_configuracoes(j) is d
    assert d.winfo_viewable()
    assert str(d.transient()) == str(j)     # presa à principal: minimiza e volta junto
    if os.name == "nt":                     # modal sem grab do Tk (ver Dialogo)
        assert _principal_desabilitada(j) and j.grab_current() is None
    else:
        assert j.grab_current() is d

    d.fechar()                              # Cancelar, Salvar, Esc e o X passam por aqui
    _processar(j)
    assert not d.winfo_exists()
    assert j.grab_current() is None and not _principal_desabilitada(j)

    j._configuracoes()                      # fechada, abre de novo
    _processar(j)
    d2 = _dialogo_configuracoes(j)
    assert d2 is not d
    d2.destroy()                            # destruída por outro caminho: a principal volta igual
    _processar(j)
    assert j.grab_current() is None and not _principal_desabilitada(j)


def test_opcoes_da_busca_uma_so_presa_a_principal(janela):
    aba = janela.abas[1]
    aba._ajuda()
    _processar(janela)
    o = aba.janela_ajuda
    aba._ajuda()
    _processar(janela)
    assert aba.janela_ajuda is o
    assert [w for w in aba.winfo_children() if isinstance(w, tk.Toplevel)] == [o]
    assert str(o.transient()) == str(janela)
    assert janela.grab_current() is None and not _principal_desabilitada(janela)   # não é modal
    o.destroy()
    _processar(janela)
    aba._ajuda()
    _processar(janela)
    assert aba.janela_ajuda is not o and aba.janela_ajuda.winfo_exists()
    aba.janela_ajuda.destroy()
    _processar(janela)


@pytest.mark.skipif(os.name != "nt", reason="minimizar e restaurar como o Windows faz (só no Windows)")
def test_com_configuracoes_aberta_o_programa_minimiza_e_volta(janela):
    """Com a janela de Configurações aberta, o programa minimizado tem de voltar
    pelo botão da barra de tarefas. Com um grab do Tk, o Tk do Windows descarta
    os comandos de sistema da janela principal (WM_SYSCOMMAND), inclusive o
    "restaurar": minimizada pelo Win+D, ela não voltava mais."""
    import ctypes
    from ctypes import wintypes
    user32 = ctypes.WinDLL("user32")
    user32.SendMessageW.argtypes = (wintypes.HWND, wintypes.UINT, wintypes.WPARAM, wintypes.LPARAM)
    user32.SendMessageW.restype = wintypes.LPARAM
    user32.ShowWindow.argtypes = (wintypes.HWND, ctypes.c_int)
    user32.IsIconic.argtypes = user32.IsWindowVisible.argtypes = (wintypes.HWND,)
    WM_SYSCOMMAND, SC_MINIMIZE, SC_RESTORE = 0x0112, 0xF020, 0xF120
    SW_MINIMIZE, SW_RESTORE = 6, 9

    j = janela
    principal = int(j.wm_frame(), 16)
    j._configuracoes()
    _processar(j)
    d = _dialogo_configuracoes(j)
    dialogo = int(d.wm_frame(), 16)
    try:
        # "Mostrar área de trabalho" (Win+D): minimiza sem passar por WM_SYSCOMMAND
        user32.ShowWindow(principal, SW_MINIMIZE)
        _processar(j)
        assert user32.IsIconic(principal)
        assert not user32.IsWindowVisible(dialogo)          # some junto com a principal
        # clique no botão do programa na barra de tarefas
        user32.SendMessageW(principal, WM_SYSCOMMAND, SC_RESTORE, 0)
        _processar(j)
        assert not user32.IsIconic(principal), "o programa ficou preso minimizado"
        assert j.state() == "normal"
        assert user32.IsWindowVisible(dialogo) and d.winfo_viewable()

        # minimizar pela barra de tarefas e restaurar de novo
        user32.SendMessageW(principal, WM_SYSCOMMAND, SC_MINIMIZE, 0)
        _processar(j)
        assert user32.IsIconic(principal), "o comando de minimizar foi ignorado"
        user32.SendMessageW(principal, WM_SYSCOMMAND, SC_RESTORE, 0)
        _processar(j)
        assert not user32.IsIconic(principal), "o programa ficou preso minimizado"
        assert user32.IsWindowVisible(dialogo) and d.winfo_viewable()
    finally:
        user32.ShowWindow(principal, SW_RESTORE)
        getattr(d, "fechar", d.destroy)()
        _processar(j)
