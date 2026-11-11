"""Formulário da aba "Busca" → argumentos de linha de comando do busca.py.

Fica fora do pacote gui (sem tkinter) para poder ser testado sozinho. A
interface só junta os valores dos campos num dicionário e chama montar_argv;
quem interpreta é o mesmo argparse do script, com as mesmas regras.
"""

import io
import json
import os
import re
import shlex
import unicodedata
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

# Fluxo da Abertura de Procedimento Licitatório (o descrito em
# verificar_planilha). Abre a lista de etapas da aba, para ela ter o que
# mostrar antes da primeira busca; as demais etapas vêm do cache.
ETAPAS_BASE = ("Validação de Requisição", "Correção de Documentação", "Pesquisa de Mercado",
               "Consulta Orçamentária", "Análise do Controle Interno", "Aprovação do COMPAC",
               "Etapa Final")
# campos de etapa da aba (lista suspensa); os de TEXTOS_MULTIPLOS aceitam várias
CAMPOS_ETAPA = ("etapa", "excluir_etapa", "antes_de", "a_partir_de", "passou_por",
                "mudanca_para", "mudanca_origem")

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
    "novo_processo_de": "--novo-processo-de",
    "novo_processo_ate": "--novo-processo-ate",
    "numeros_arquivo": "--numeros-arquivo",
}
INTEIROS = {
    "dias_etapa_min": "--dias-etapa-min",
    "dias_etapa_max": "--dias-etapa-max",
    "sem_mov_min": "--sem-mov-min",
}
# "Emitiu novo processo:" (lista Sim/Não; vazio = tanto faz) → opção
NOVO_PROCESSO = {"Sim": "--novo-processo", "Não": "--sem-novo-processo"}
BOOLEANOS = {
    "etapa_exata": "--etapa-exata",
    "offline": "--offline",
    "refazer": "--refazer",
    "relogar": "--relogar",
}


class FormularioInvalido(ValueError):
    pass


def _chave_etapa(nome):
    """Para comparar nomes de etapa: sem acento, sem caixa, espaços colapsados."""
    sem = "".join(c for c in unicodedata.normalize("NFD", nome) if unicodedata.category(c) != "Mn")
    return re.sub(r"\s+", " ", sem.casefold()).strip()


def ordenar_etapas(fluxos):
    """Nomes únicos de etapa: primeiro o fluxo da Abertura (ETAPAS_BASE), na
    ordem; depois as outras etapas, na ordem dos quadros em que aparecem.
    Vale a grafia lida no 1Doc quando houver."""
    grafia = {}
    for fluxo in fluxos:
        for nome in fluxo:
            grafia.setdefault(_chave_etapa(nome), nome)
    saida, vistos = [], set()
    for nome in [*ETAPAS_BASE, *(n for fluxo in fluxos for n in fluxo)]:
        k = _chave_etapa(nome)
        if k and k not in vistos:
            vistos.add(k)
            saida.append(grafia.get(k, nome))
    return saida


_memo_etapas = {}


def etapas_conhecidas(arquivo_cache=None):
    """Lista de etapas para os campos da aba: o fluxo da Abertura mais as
    etapas dos quadros que a busca já leu (cache de detalhes, 1doc_cache.json).
    Cresce a cada busca; o cache só é relido quando muda."""
    if arquivo_cache is None:
        from . import caminhos
        arquivo_cache = caminhos.arquivo_dados("1doc_cache.json")
    try:
        st = os.stat(arquivo_cache)
        marca = (arquivo_cache, st.st_mtime_ns, st.st_size)
    except OSError:
        marca = None
    if marca is not None and _memo_etapas.get("marca") == marca:
        return list(_memo_etapas["etapas"])
    fluxos, vistos = [], set()
    if marca is not None:
        try:
            with open(arquivo_cache, encoding="utf-8") as f:
                dados = json.load(f)
            det = dados.get("det") if isinstance(dados, dict) else None
            for rec in (det.values() if isinstance(det, dict) else ()):
                quadro = rec.get("etapas_quadro") if isinstance(rec, dict) else None
                fluxo = tuple(n.strip() for n in (quadro or []) if isinstance(n, str) and n.strip())
                if fluxo and fluxo not in vistos:
                    vistos.add(fluxo)
                    fluxos.append(fluxo)
        except (OSError, ValueError):
            fluxos = []
    etapas = ordenar_etapas(fluxos)
    _memo_etapas.update(marca=marca, etapas=etapas)
    return list(etapas)


def juntar_etapa(antes, escolhida, varios=True):
    """Texto de um campo de etapa depois de escolher `escolhida` na lista.
    Campo de várias etapas cujo texto `antes` termina em ';': ela se junta às
    que já estão lá (sem repetir). Nos outros casos, substitui o texto."""
    if varios and antes.rstrip().endswith(";"):
        itens = _varios(antes)
        if _chave_etapa(escolhida) not in {_chave_etapa(i) for i in itens}:
            itens.append(escolhida)
        return "; ".join(itens)
    return escolhida


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
    novo = (form.get("novo_processo") or "").strip()
    if novo:
        if novo not in NOVO_PROCESSO:
            raise FormularioInvalido(f"'{novo}' não vale em \"Emitiu novo processo\" (use Sim ou Não).")
        if novo == "Não" and any((form.get(c) or "").strip() for c in ("novo_processo_de", "novo_processo_ate")):
            raise FormularioInvalido("\"Emitiu novo processo: Não\" não combina com a data do novo processo: "
                                     "apague as datas ou escolha Sim.")
        argv.append(NOVO_PROCESSO[novo])
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
