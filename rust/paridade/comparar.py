"""Paridade Python × Rust: compara as planilhas de <pasta>/python e <pasta>/rust.

    python rust/paridade/comparar.py PASTA

Confere, aba por aba: nomes das abas, valores (vazio e None contam igual),
links, mesclagens, autofiltro, painel congelado e tabelas. As larguras são
só informadas: o rust_xlsxwriter grava a largura como o Excel (com a margem
interna de ~0,7), o openpyxl grava o número pedido.
Sai com código 1 se houver diferença.
"""

import os
import re
import sys
from datetime import datetime

from openpyxl import load_workbook

# Valores que mudam de uma execução para outra (data/hora da posição)
VOLATEIS = {("busca.xlsx", "Parâmetros")}


def iguais(x, y):
    """Valores iguais; datas e horas estimadas a partir de "agora" (a
    entrada na etapa pelo "Está aqui N horas") podem diferir alguns minutos
    entre as duas execuções."""
    if isinstance(x, datetime) and isinstance(y, datetime):
        return abs((x - y).total_seconds()) <= 600
    if isinstance(x, str) and isinstance(y, str):
        return re.sub(r"\d\d:\d\d", "HH:MM", x) == re.sub(r"\d\d:\d\d", "HH:MM", y)
    return x == y


def normalizar(v):
    if v is None or v == "":
        return None
    if isinstance(v, float) and v.is_integer():
        return int(v)
    if isinstance(v, datetime):
        return v.replace(second=0, microsecond=0)
    return v


def larguras(ws):
    from openpyxl.utils import get_column_letter
    out = {}
    for d in ws.column_dimensions.values():
        if d.width and d.min:
            for c in range(d.min, (d.max or d.min) + 1):
                out[get_column_letter(c)] = round(d.width, 1)
    return out


def resumo(caminho):
    wb = load_workbook(caminho)
    out = {}
    for ws in wb.worksheets:
        valores = {}
        links = {}
        for linha in ws.iter_rows():
            for c in linha:
                v = normalizar(c.value)
                if v is not None:
                    valores[c.coordinate] = v
                if c.hyperlink is not None and c.hyperlink.target:
                    # o 1Doc de mentira do Rust roda num servidor local
                    links[c.coordinate] = re.sub(r"^https?://(montesclaros\.1doc\.com\.br|127\.0\.0\.1:\d+)",
                                                 "1DOC", c.hyperlink.target)
        out[ws.title] = {
            "valores": valores,
            "links": links,
            "mesclagens": sorted(str(r) for r in ws.merged_cells.ranges),
            "autofiltro": ws.auto_filter.ref,
            "congelado": ws.freeze_panes,
            "tabelas": sorted((t.displayName, t.ref) for t in ws.tables.values()),
            "larguras": larguras(ws),
        }
    return out


def utf8_no_console():
    """O console do Windows usa cp1252: os ✓ e ✗ das mensagens não caberiam."""
    for f in (sys.stdout, sys.stderr):
        try:
            f.reconfigure(encoding="utf-8", errors="replace")
        except (AttributeError, ValueError):
            pass


def main():
    utf8_no_console()
    pasta = os.path.abspath(sys.argv[1])
    falhas = 0
    nomes = sorted(set(os.listdir(os.path.join(pasta, "python"))) | set(os.listdir(os.path.join(pasta, "rust"))))
    for nome in [n for n in nomes if n.endswith(".xlsx")]:
        py, rs = (os.path.join(pasta, lado, nome) for lado in ("python", "rust"))
        if not (os.path.exists(py) and os.path.exists(rs)):
            print(f"✗ {nome}: só existe em {'python' if os.path.exists(py) else 'rust'}")
            falhas += 1
            continue
        a, b = resumo(py), resumo(rs)
        if list(a) != list(b):
            print(f"✗ {nome}: abas {list(a)} × {list(b)}")
            falhas += 1
            continue
        for aba in a:
            for campo in ("valores", "links", "mesclagens", "autofiltro", "congelado", "tabelas"):
                if (nome, aba) in VOLATEIS and campo == "valores":
                    continue
                x, y = a[aba][campo], b[aba][campo]
                if isinstance(x, dict):
                    difs = [k for k in sorted(set(x) | set(y)) if not iguais(x.get(k), y.get(k))]
                    for k in difs:
                        print(f"✗ {nome}/{aba} {campo} {k}: python={x.get(k)!r} rust={y.get(k)!r}")
                    falhas += bool(difs)
                elif x != y:
                    falhas += 1
                    print(f"✗ {nome}/{aba} {campo}: python={x!r} rust={y!r}")
            la, lb = a[aba]["larguras"], b[aba]["larguras"]
            dif = {k: (la.get(k), lb.get(k)) for k in set(la) | set(lb) if abs((la.get(k) or 0) - (lb.get(k) or 0)) > 0.8}
            if dif:
                print(f"  (larguras diferentes em {nome}/{aba}: {dif})")
        print(f"{'✓' if not falhas else '·'} {nome}")
    print("PARIDADE OK" if not falhas else f"{falhas} DIFERENÇA(S)")
    sys.exit(1 if falhas else 0)


if __name__ == "__main__":
    main()
