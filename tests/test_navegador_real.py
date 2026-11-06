"""Login com um navegador de verdade contra um "1Doc" local.

A página de login deste servidor tem um script que faz o papel da pessoa:
depois de alguns segundos ele preenche os campos e envia o formulário. O
programa só pode esperar — o teste confere que ele não detecta o login antes
do envio, que os valores enviados são os da "pessoa" e que os cookies
capturados valem nas requisições.

Roda só quando há navegador e driver configurados:
    FERRAMENTAS1DOC_CHROME=/caminho/chrome  FERRAMENTAS1DOC_CHROMEDRIVER=/caminho/chromedriver
ou, no Windows (CI), com AUTOTESTE_NAVEGADOR_REAL=1 (Selenium Manager acha o driver).
"""

import os
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs

import pytest

from ferramentas1doc import sessao

pytestmark = pytest.mark.skipif(
    not (os.environ.get("FERRAMENTAS1DOC_CHROME") or os.environ.get("AUTOTESTE_NAVEGADOR_REAL")),
    reason="sem navegador real configurado")

LOGIN = """<html><body>
<div class="tab-content">(a página de login também tem abas: não pode contar como logada)</div>
<form id="f" method="post" action="/login">
  <input name="identifier" type="email"><input name="senha" type="password"><button type="submit">Entrar</button>
</form>
<script>
  // "a pessoa" digita e entra depois de 2,5 s
  setTimeout(function () {
    document.querySelector('[name=identifier]').value = 'pessoa@montesclaros.mg.gov.br';
    document.querySelector('[name=senha]').value = 'segredo-da-pessoa';
    document.getElementById('f').submit();
  }, 2500);
</script></body></html>"""

PAINEL = """<html><body><input type="hidden" id="sis_usr_email" value="pessoa@montesclaros.mg.gov.br">
<input type="hidden" id="s_s" value="SPOT-DCL-GP-CPPFI"><div class="tab-content">caixas</div></body></html>"""


class Servidor(BaseHTTPRequestHandler):
    envios = []
    inicio = None

    def log_message(self, *a):
        pass

    def _html(self, html, extra=None):
        corpo = html.encode()
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        for k, v in (extra or {}).items():
            self.send_header(k, v)
        self.send_header("Content-Length", str(len(corpo)))
        self.end_headers()
        self.wfile.write(corpo)

    def do_GET(self):
        if "PHPSESSID=ok" in (self.headers.get("Cookie") or ""):
            self._html(PAINEL)
        else:
            Servidor.inicio = Servidor.inicio or time.monotonic()
            self._html(LOGIN)

    def do_POST(self):
        n = int(self.headers.get("Content-Length") or 0)
        Servidor.envios.append((time.monotonic(), parse_qs(self.rfile.read(n).decode())))
        self.send_response(302)
        self.send_header("Set-Cookie", "PHPSESSID=ok; Path=/; HttpOnly")
        self.send_header("Location", "/?pg=painel/listar")
        self.end_headers()


def test_login_real_no_navegador(dados, monkeypatch):
    srv = ThreadingHTTPServer(("127.0.0.1", 0), Servidor)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    base = f"http://127.0.0.1:{srv.server_port}/"
    monkeypatch.setattr(sessao, "BASE", base)
    monkeypatch.setattr(sessao, "URL_LOGIN", base + "?pg=painel/listar")
    try:
        cred = sessao.aguardar_login(
            minutos=1, avisar=print,
            fabrica=lambda: sessao.abrir_navegador(os.environ.get("AUTOTESTE_NAVEGADOR", "chrome"),
                                                   headless=True, usar_perfil=False),
            intervalo=0.3, intervalo_validacao=1)
        detectado = time.monotonic()
    finally:
        srv.shutdown()

    assert len(Servidor.envios) == 1                      # um envio, o da "pessoa"
    envio_t, campos = Servidor.envios[0]
    assert campos == {"identifier": ["pessoa@montesclaros.mg.gov.br"], "senha": ["segredo-da-pessoa"]}
    assert envio_t - Servidor.inicio >= 2.0              # o programa não enviou nada antes
    assert detectado >= envio_t                          # e só detectou depois do envio
    assert cred.email == "pessoa@montesclaros.mg.gov.br" and cred.setor == "SPOT-DCL-GP-CPPFI"
    assert any(c["name"] == "PHPSESSID" and c["value"] == "ok" for c in cred.cookies)


def test_navegador_invisivel_com_cookies_da_sessao(dados, monkeypatch):
    """Fallback do 'Localizar': o navegador invisível já entra logado com os cookies."""
    srv = ThreadingHTTPServer(("127.0.0.1", 0), Servidor)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    base = f"http://127.0.0.1:{srv.server_port}/"
    monkeypatch.setattr(sessao, "BASE", base)
    cred = sessao.Credenciais(cookies=[{"name": "PHPSESSID", "value": "ok", "domain": "127.0.0.1", "path": "/",
                                        "httpOnly": True, "secure": False, "sameSite": "Lax"}], ua="UA")
    nav = sessao.NavegadorSobDemanda(cred, os.environ.get("AUTOTESTE_NAVEGADOR", "chrome"))
    try:
        drv = nav.obter()
        assert drv is not None
        drv.get(base + "?pg=painel/listar")
        assert drv.find_element("id", "sis_usr_email").get_attribute("value") == "pessoa@montesclaros.mg.gov.br"
        assert nav.obter() is drv                           # reaproveita, não abre outro
    finally:
        nav.fechar()
        srv.shutdown()
