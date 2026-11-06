"""A janela de verdade, rodando uma ferramenta contra o 1Doc de mentira."""

import os
import sys
import time
from datetime import date

import pytest

tk = pytest.importorskip("tkinter")

import fake1doc  # noqa: E402


@pytest.fixture
def janela(dados, cred, monkeypatch):
    from ferramentas1doc import processos_do_dia, sessao
    from ferramentas1doc.gui import janela as mod
    from ferramentas1doc.gui.log import SaidaFila
    try:
        tk.Tk().destroy()
    except tk.TclError:
        pytest.skip("sem tela para o Tk")

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
    j = mod.Janela(SaidaFila())
    j.avisos = avisos
    yield j
    j.destroy()


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
