"""Login com um navegador de verdade contra um "1Doc" local.

A página de login deste servidor tem um script que faz o papel da pessoa:
depois de alguns segundos ele preenche os campos e envia o formulário. O
programa só pode esperar — o teste confere que ele não detecta o login antes
do envio, que os valores enviados são os da "pessoa" e que os cookies
capturados valem nas requisições.

Não há driver: o navegador é controlado por CDP. Roda só quando há navegador
configurado:
    FERRAMENTAS1DOC_CHROME=/caminho/chrome
ou, no Windows (CI), com AUTOTESTE_NAVEGADOR_REAL=1 (usa o Chrome/Edge
instalado; AUTOTESTE_NAVEGADOR=edge testa o Edge).
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

LOGADO = """<input type="hidden" id="sis_usr_email" value="pessoa@montesclaros.mg.gov.br">
<input type="hidden" id="s_s" value="SPOT-DCL-GP-CPPFI">"""
PAINEL = f"""<html><body>{LOGADO}<div class="tab-content">caixas</div></body></html>"""

# "Localizar" do painel: como no 1Doc, o <select> fica escondido (select2) e o
# resultado chega depois de uma ida ao servidor, trocando a URL da aba.
PAINEL_LOCALIZAR = f"""<html><body>{LOGADO}
<form class="form_localiza" onsubmit="return localizar(event)">
  <select id="id_documento_rapido" style="display:none">
    <option value="">--</option><option value="3">Proc. Administrativo</option></select>
  <input id="num_codigo"><input id="localiza_ano"><button type="submit">Localizar</button>
</form>
<script>
  function localizar(ev) {{
    ev.preventDefault();
    var tipo = document.getElementById('id_documento_rapido').value,
        num = document.getElementById('num_codigo').value,
        ano = document.getElementById('localiza_ano').value,
        hash = (tipo === '3' && num === '31632' && ano === '2026') ? 'AAAA1111BBBB2222CCCC' : '';
    setTimeout(function () {{ location.href = '/?pg=doc/ver&hash=' + hash + '&itd=3'; }}, 300);
    return false;
  }}
</script></body></html>"""


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
            self._html(PAINEL_LOCALIZAR if "caixa=entrada" in self.path else PAINEL)
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
        drv.ir(base + "?pg=painel/listar")
        aba = drv.pagina()
        email = "document.getElementById('sis_usr_email') && document.getElementById('sis_usr_email').value"
        for _ in range(50):
            if drv.avaliar(aba, email):
                break
            time.sleep(0.1)
        assert drv.avaliar(aba, email) == "pessoa@montesclaros.mg.gov.br"
        assert nav.obter() is drv                           # reaproveita, não abre outro
    finally:
        nav.fechar()
        srv.shutdown()
    assert drv.processo.poll() is not None                  # fechou o navegador


def test_localizar_pelo_navegador_invisivel(dados, monkeypatch):
    """Último recurso dos verificadores: o formulário do painel, preenchido no navegador."""
    from ferramentas1doc import faltantes, verificar_planilha

    srv = ThreadingHTTPServer(("127.0.0.1", 0), Servidor)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    base = f"http://127.0.0.1:{srv.server_port}/"
    cred = sessao.Credenciais(cookies=[{"name": "PHPSESSID", "value": "ok", "domain": "127.0.0.1", "path": "/",
                                        "httpOnly": True}], ua="UA")
    nav = sessao.NavegadorSobDemanda(cred, os.environ.get("AUTOTESTE_NAVEGADOR", "chrome"))
    try:
        drv = nav.obter()
        assert drv is not None
        for modulo in (faltantes, verificar_planilha):
            monkeypatch.setattr(modulo, "URL_PAINEL", base + "?pg=painel/listar&meu=0&trocar=1&caixa=entrada")
            assert modulo.localizar_navegador(drv, {"digitos": "31632", "ano": "2026"}) == ["AAAA1111BBBB2222CCCC"]
            assert modulo.localizar_navegador(drv, {"digitos": "99999", "ano": "2026"}) == []
    finally:
        nav.fechar()
        srv.shutdown()
