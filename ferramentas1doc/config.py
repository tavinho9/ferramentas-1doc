"""Configurações do usuário (config.json na pasta de dados)."""

import json
import os

from . import caminhos

PADRAO = {
    "navegador": "auto",        # auto (Chrome, senão Edge), chrome ou edge
    "pasta_saida": "",          # vazio = Área de Trabalho
    "abrir_planilha": True,     # abre a planilha no Excel ao terminar
    "minutos_login": 10,        # quanto tempo esperar o login no navegador
    "planilha_faltantes": "",   # última planilha usada em cada aba
    "planilha_verificar": "",
    "faltantes_de": "",         # período de emissão da aba Faltantes, como digitado
    "faltantes_ate": "",        # (vazio = padrão; "-30", "hoje" valem no dia da execução)
}

NAVEGADORES = {"auto": "Automático (Chrome; se não houver, Edge)", "chrome": "Google Chrome",
               "edge": "Microsoft Edge"}


def _arquivo():
    return caminhos.arquivo_dados("config.json")


def carregar():
    dados = dict(PADRAO)
    try:
        with open(_arquivo(), encoding="utf-8") as f:
            lido = json.load(f)
        if isinstance(lido, dict):
            dados.update({k: v for k, v in lido.items() if k in PADRAO})
    except (OSError, ValueError):
        pass
    if dados["navegador"] not in NAVEGADORES:
        dados["navegador"] = "auto"
    for chave in ("planilha_faltantes", "planilha_verificar", "faltantes_de", "faltantes_ate"):
        if not isinstance(dados[chave], str):
            dados[chave] = ""
    try:
        dados["minutos_login"] = max(1, int(dados["minutos_login"]))
    except (TypeError, ValueError):
        dados["minutos_login"] = PADRAO["minutos_login"]
    return dados


def salvar(dados):
    with open(_arquivo(), "w", encoding="utf-8") as f:
        json.dump({k: dados.get(k, v) for k, v in PADRAO.items()}, f, ensure_ascii=False, indent=2)


def pasta_saida(dados=None):
    """Pasta de saída válida: a configurada, se existir; senão a Área de Trabalho."""
    dados = dados or carregar()
    p = (dados.get("pasta_saida") or "").strip()
    if p and os.path.isdir(p):
        return p
    return caminhos.area_de_trabalho()
