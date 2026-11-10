"""Paridade Python × Rust, lado Python.

Cria as planilhas de entrada e roda as quatro ferramentas da versão Python
contra o 1Doc de mentira (tests/fake1doc.py), gravando as saídas em
<pasta>/python. O lado Rust (tests/paridade.rs) lê as mesmas entradas, usa
um 1Doc de mentira com o mesmo HTML e grava em <pasta>/rust; comparar.py
confere as duas.

    python rust/paridade/gerar_python.py PASTA
"""

import os
import shutil
import sys
from datetime import date

RAIZ = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, RAIZ)
sys.path.insert(0, os.path.join(RAIZ, "tests"))

from openpyxl import Workbook  # noqa: E402
from openpyxl.styles import Font, PatternFill  # noqa: E402
from openpyxl.worksheet.table import Table, TableStyleInfo  # noqa: E402


class Remendo:
    """O mínimo do monkeypatch do pytest."""

    def setattr(self, alvo, nome, valor):
        setattr(alvo, nome, valor)


def entradas(pasta):
    """Planilhas de entrada (as mesmas para as duas versões)."""
    # Acompanhamento simples (faltantes)
    wb = Workbook()
    ws = wb.active
    ws.title = "Acompanhamento"
    ws.append(["Nº Processo", "Prioridade", "Objeto", "Etapa", "Secretaria"])
    ws.append(["31.632/2026", "Alta", "objeto", "etapa", "Secretaria de Saúde"])
    wb.save(os.path.join(pasta, "acomp.xlsx"))

    # Planilha "difícil" para o verificar: saída de uma execução anterior
    # (colunas geradas e aba Mudaram), links, mesclagens, tabela do Excel,
    # fórmula, colunas agrupadas e formatação até muito abaixo dos dados.
    wb = Workbook()
    ws = wb.active
    ws.title = "Planilha1"
    ws.merge_cells("A1:C1")
    ws["A1"] = "Acompanhamento DCL"
    ws.append(["Nº Processo", "Objeto", "Valor", "Etapa atual (1Doc)", "Secretaria", "Novo processo"])
    linhas = [("31.700/2026", "Reforma", 10), ("31.632/2026", "Medicamentos", 20),
              ("12.345/2026", "Outro", 30), ("31.632/2026", "Repetido", 40)]
    for i, (n, o, v) in enumerate(linhas, 3):
        ws.append([n, o, v, "velha", "SS", "x"])
        ws.cell(i, 1).hyperlink = f"https://exemplo.invalid/{i}"
    ws["G3"] = "=C3*2"
    ws.merge_cells("B5:B6")
    ws.cell(2, 1).font = Font(bold=True, color="FFFFFF", size=12)
    ws.cell(2, 1).fill = PatternFill("solid", fgColor="1F4E78")
    ws.column_dimensions.group("A", "H", hidden=False)
    ws.column_dimensions["A"].width = 20
    t = Table(displayName="T1", ref="A2:F6")
    t.tableStyleInfo = TableStyleInfo(name="TableStyleLight1")
    ws.add_table(t)
    ws.cell(400, 2).fill = PatternFill("solid", fgColor="FFFF00")
    wb.create_sheet("Mudaram")
    wb.save(os.path.join(pasta, "verificar.xlsx"))

    # Sem cabeçalho
    wb = Workbook()
    ws = wb.active
    ws.append(["31.632/2026", "Medicamentos"])
    ws.append(["31.700/2026", "Reforma"])
    wb.save(os.path.join(pasta, "sem_cabecalho.xlsx"))


def main():
    pasta = os.path.abspath(sys.argv[1])
    entrada = os.path.join(pasta, "entrada")
    saida = os.path.join(pasta, "python")
    dados = os.path.join(pasta, "dados_python")
    for p in (entrada, saida, dados):
        shutil.rmtree(p, ignore_errors=True)
        os.makedirs(p)
    entradas(entrada)

    import fake1doc
    from ferramentas1doc import (busca, busca_argv, caminhos, config, faltantes, processos_do_dia,
                                 sessao, verificar_planilha)

    caminhos.pasta_dados = lambda: dados
    caminhos.area_de_trabalho = lambda: saida
    config.salvar({**config.PADRAO, "abrir_planilha": False, "pasta_saida": saida})
    for m in (processos_do_dia, faltantes, verificar_planilha):
        m.PAUSA_REQ = 0
    for m in (faltantes, verificar_planilha):
        m.ARQ_CACHE = os.path.join(dados, "hash_cache_1doc.json")
    fake1doc.instalar(Remendo())
    cred = sessao.Credenciais(cookies=[{"name": "PHPSESSID", "value": "abc",
                                        "domain": "montesclaros.1doc.com.br", "path": "/"}],
                              ua="Mozilla/5.0 (teste)", setor="SPOT-DCL-GP-CPPFI", email="teste@pmc")

    def guardar(caminho, nome):
        if caminho:
            shutil.copy(caminho, os.path.join(saida, nome))

    guardar(processos_do_dia.executar(date.today(), cred), "processos_do_dia.xlsx")
    argv = busca_argv.montar_argv({"caixas": ["entrada"], "etapa": "Pesquisa de Mercado"}) + ["--pausa", "0"]
    guardar(busca.main(argv, cred=cred)["arquivo"], "busca.xlsx")
    guardar(faltantes.executar(os.path.join(entrada, "acomp.xlsx"), None, cred), "faltantes.xlsx")
    guardar(verificar_planilha.executar(os.path.join(entrada, "verificar.xlsx"), "Planilha1", cred),
            "verificar.xlsx")
    guardar(verificar_planilha.executar(os.path.join(entrada, "sem_cabecalho.xlsx"), None, cred),
            "sem_cabecalho.xlsx")
    for f in os.listdir(saida):
        if f.endswith(".xlsx") and f not in ("processos_do_dia.xlsx", "busca.xlsx", "faltantes.xlsx",
                                             "verificar.xlsx", "sem_cabecalho.xlsx"):
            os.remove(os.path.join(saida, f))


if __name__ == "__main__":
    main()
