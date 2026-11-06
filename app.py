"""Ferramentas 1Doc — ponto de entrada.

    Ferramentas1Doc.exe                    abre o programa
    Ferramentas1Doc.exe --autoteste        confere o pacote e sai (0 = tudo certo)
        [--relatorio ARQ] [--sem-navegador] [--sem-janela]
"""

import argparse
import os
import sys
import tempfile
import traceback


def _ajustar_dpi():
    """Sem isto o Windows amplia a janela do Tk como imagem e o texto fica borrado."""
    if os.name == "nt":
        try:
            import ctypes
            ctypes.windll.shcore.SetProcessDpiAwareness(1)
        except Exception:
            pass


def autoteste(relatorio=None, navegador=True, janela=True):
    linhas, falhas = [], 0

    def item(nome, func):
        nonlocal falhas
        try:
            detalhe = func()
            linhas.append(f"OK     {nome}" + (f" — {detalhe}" if detalhe else ""))
        except BaseException as e:      # noqa: BLE001 — o autoteste precisa registrar tudo
            falhas += 1
            linhas.append(f"FALHA  {nome} — {type(e).__name__}: {e}")
            linhas.append("       " + traceback.format_exc().replace("\n", "\n       "))

    def importar():
        import importlib
        import ferramentas1doc
        from ferramentas1doc import caminhos
        for m in ("busca", "busca_argv", "config", "faltantes", "processos_do_dia", "sessao",
                  "verificar_planilha", "gui.janela", "gui.abas", "gui.log", "gui.secundaria"):
            importlib.import_module(f"ferramentas1doc.{m}")
        return f"versão {ferramentas1doc.VERSAO}, dados em {caminhos.pasta_dados()}"

    def parser_html():
        from bs4 import BeautifulSoup
        from ferramentas1doc import processos_do_dia
        html = ('<div id="emissao_principal"><div class="kanban_estagios" data-current="true">'
                '<div class="kanban_titulo"><p>Pesquisa de Mercado</p></div><div class="kanban_itens">'
                '<p class="kanban_item">Esta aqui 2 horas</p></div></div></div>')
        assert processos_do_dia.ler_etapa_atual(html) == ("Pesquisa de Mercado", "Esta aqui 2 horas")
        BeautifulSoup("<p>x</p>", "lxml")
        return f"parser {processos_do_dia.PARSER}"

    def argumentos_busca():
        from ferramentas1doc import busca_argv
        argv = busca_argv.montar_argv({"caixas": ["entrada"], "etapa": "Pesquisa de Mercado",
                                       "emissao_de": "-30"})
        erro = busca_argv.validar(argv)
        assert erro is None, erro
        return " ".join(argv)

    def planilha():
        from openpyxl import Workbook, load_workbook
        caminho = os.path.join(tempfile.mkdtemp(), "teste.xlsx")
        wb = Workbook()
        wb.active["A1"] = "Nº Processo"
        wb.save(caminho)
        assert load_workbook(caminho).active["A1"].value == "Nº Processo"

    def selenium_manager():
        from selenium.webdriver.common.selenium_manager import SeleniumManager
        binario = SeleniumManager()._get_binary()
        assert os.path.isfile(binario), binario
        return str(binario)

    def abrir_navegador():
        from ferramentas1doc import sessao
        drv, nome = sessao.abrir_navegador(os.environ.get("AUTOTESTE_NAVEGADOR", "auto"),
                                           headless=True, usar_perfil=False)
        try:
            drv.get("data:text/html,<title>ok</title><input id='sis_usr_email' value='x'>")
            assert drv.title == "ok"
            return f"{nome} {drv.capabilities.get('browserVersion', '')}"
        finally:
            drv.quit()

    def montar_janela():
        from ferramentas1doc.gui.janela import Janela
        from ferramentas1doc.gui.log import SaidaFila
        j = Janela(SaidaFila())
        j.update()
        abas = [j.notebook.tab(i, "text").strip() for i in range(len(j.notebook.tabs()))]
        j.after(500, j.destroy)
        j.mainloop()
        return ", ".join(abas)

    item("módulos", importar)
    item("leitura de HTML", parser_html)
    item("argumentos da busca", argumentos_busca)
    item("planilha .xlsx", planilha)
    item("selenium-manager no pacote", selenium_manager)
    if navegador:
        item("navegador (invisível) pelo Selenium Manager", abrir_navegador)
    if janela:
        item("janela principal", montar_janela)
    linhas.append(f"\n{'TUDO CERTO' if not falhas else f'{falhas} FALHA(S)'}")
    texto = "\n".join(linhas) + "\n"
    if relatorio:
        with open(relatorio, "w", encoding="utf-8") as f:
            f.write(texto)
    if sys.stdout is not None:
        try:
            sys.stdout.write(texto)
        except Exception:
            pass
    return 1 if falhas else 0


def main():
    ap = argparse.ArgumentParser(add_help=False)
    ap.add_argument("--autoteste", action="store_true")
    ap.add_argument("--relatorio")
    ap.add_argument("--sem-navegador", action="store_true")
    ap.add_argument("--sem-janela", action="store_true")
    args, _ = ap.parse_known_args()
    _ajustar_dpi()
    if args.autoteste:
        sys.exit(autoteste(args.relatorio, not args.sem_navegador, not args.sem_janela))

    from ferramentas1doc.gui.janela import Janela
    from ferramentas1doc.gui.log import SaidaFila

    # Sem console (programa empacotado), sys.stdout é None: todo print dos
    # scripts vai para a caixa de mensagens.
    saida = SaidaFila()
    sys.stdout = sys.stderr = saida
    Janela(saida).mainloop()


if __name__ == "__main__":
    main()
