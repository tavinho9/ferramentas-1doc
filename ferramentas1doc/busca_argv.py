"""Formulário da aba "Busca" → argumentos de linha de comando do busca.py.

Fica fora do pacote gui (sem tkinter) para poder ser testado sozinho. A
interface só junta os valores dos campos num dicionário e chama montar_argv;
quem interpreta é o mesmo argparse do script, com as mesmas regras.
"""

import io
import os
import re
import shlex
from contextlib import redirect_stderr
from datetime import datetime

CAIXAS = ("entrada", "saida", "arquivo", "favoritos")
PRIORIDADES = ("Urgente", "Alta", "Média", "Baixa")

# Instrução de datas mostrada na aba: os formatos que o busca aceita
# (busca.data_cli), um exemplo de cada conferido nos testes. Duas linhas,
# para caber no espaço livre do quadro de datas.
EXEMPLOS_DATA = ("05/11/2026", "2026-11-05", "hoje", "ontem", "-7")
DICA_DATAS = ("Datas: DD/MM/AAAA (05/11/2026), AAAA-MM-DD, hoje, ontem\n"
              "ou -N = N dias atrás (-7). Inclui o próprio dia informado.")

# campo do formulário → opção do busca (texto; aceita vários separados por ";")
TEXTOS_MULTIPLOS = {
    "etapa": "--etapa",
    "excluir_etapa": "--excluir-etapa",
    "objeto": "--objeto",
    "mudanca_para": "--mudanca-para",
    "mudanca_origem": "--mudanca-origem",
    "passou_por": "--passou-por",
}
# campo → opção (valor único)
TEXTOS = {
    "antes_de": "--antes-de",
    "a_partir_de": "--a-partir-de",
    "emissao_de": "--emissao-de",
    "emissao_ate": "--emissao-ate",
    "mudanca_de": "--mudanca-de",
    "mudanca_ate": "--mudanca-ate",
    "entrada_de": "--entrada-de",
    "entrada_ate": "--entrada-ate",
    "numeros_arquivo": "--numeros-arquivo",
}
INTEIROS = {
    "dias_etapa_min": "--dias-etapa-min",
    "dias_etapa_max": "--dias-etapa-max",
    "sem_mov_min": "--sem-mov-min",
}
BOOLEANOS = {
    "etapa_exata": "--etapa-exata",
    "offline": "--offline",
    "refazer": "--refazer",
    "relogar": "--relogar",
}


class FormularioInvalido(ValueError):
    pass


def dividir_extras(texto):
    """Opções extras digitadas como na linha de comando. Sem o modo POSIX do
    shlex, para que caminhos do Windows (C:\\Users\\...) não percam as barras;
    as aspas em volta de cada termo são retiradas."""
    termos = []
    for t in shlex.split(texto or "", posix=False):
        if len(t) >= 2 and t[0] == t[-1] and t[0] in "\"'":
            t = t[1:-1]
        termos.append(t)
    return termos


def _varios(valor):
    return [p.strip() for p in re.split(r"[;\n]", valor or "") if p.strip()]


def montar_argv(form, pasta_saida=None, agora=None, abrir=False):
    """form: dict com os campos da aba. Devolve a lista de argumentos."""
    agora = agora or datetime.now()
    argv = []

    caixas = [c for c in CAIXAS if c in (form.get("caixas") or [])]
    numeros = (form.get("numeros") or "").strip()
    if not caixas and not numeros and not form.get("numeros_arquivo") and not form.get("offline"):
        raise FormularioInvalido("Marque ao menos uma caixa (ou informe números de processo).")
    if caixas:
        argv += ["--caixas", "todas" if len(caixas) == len(CAIXAS) else ",".join(caixas)]
    if numeros:
        argv += ["--numeros", ",".join(n for n in re.split(r"[\s,;]+", numeros) if n)]

    for campo, opcao in TEXTOS.items():
        v = (form.get(campo) or "").strip()
        if v:
            argv += [opcao, v]
    for campo, opcao in TEXTOS_MULTIPLOS.items():
        for v in _varios(form.get(campo)):
            argv += [opcao, v]
    for campo, opcao in INTEIROS.items():
        v = str(form.get(campo) or "").strip()
        if v:
            if not re.fullmatch(r"\d+", v):
                raise FormularioInvalido(f"'{v}' não é um número inteiro ({opcao}).")
            argv += [opcao, v]
    fase = (form.get("fase") or "").strip()
    if fase:
        argv += ["--fase", fase]
    prios = [p for p in PRIORIDADES if p in (form.get("prioridade") or [])]
    if prios and len(prios) < len(PRIORIDADES):
        argv += ["--prioridade", ",".join(prios)]
    for campo, opcao in BOOLEANOS.items():
        if form.get(campo):
            argv.append(opcao)

    if form.get("csv"):
        nome = f"Busca 1Doc {agora:%d-%m-%Y %Hh%M}.csv"
        argv += ["--csv", os.path.join(pasta_saida, nome) if pasta_saida else nome]
    if abrir:
        argv.append("--abrir")
    argv += dividir_extras(form.get("extras"))
    return argv


def validar(argv):
    """Passa os argumentos pelo argparse do busca sem executar nada. Devolve
    None se estão certos ou a mensagem de erro."""
    from . import busca

    erro = io.StringIO()
    try:
        with redirect_stderr(erro):
            busca.normalizar_args(busca.montar_parser().parse_args(argv))
    except SystemExit as e:
        msg = erro.getvalue().strip().splitlines()
        if isinstance(e.code, str):
            return e.code
        return msg[-1].replace("busca: error: ", "") if msg else "argumentos inválidos"
    return None
