# -*- mode: python ; coding: utf-8 -*-
# PyInstaller: programa em pasta (abre mais rápido e alarma menos o antivírus
# que o --onefile), sem janela de console.
#   pyinstaller --noconfirm --clean build/Ferramentas1Doc.spec --distpath dist --workpath build/tmp
import os

from PyInstaller.utils.hooks import collect_submodules

raiz = os.path.abspath(os.path.join(SPECPATH, ".."))
icone = os.path.join(raiz, "ferramentas1doc", "gui", "icone.ico")

# Sem Selenium nem driver: o Chrome/Edge instalado é controlado por CDP (websocket-client).
datas = [(icone, ".")]
binaries = []
hiddenimports = collect_submodules("ferramentas1doc") + ["lxml.etree", "lxml._elementpath"]

a = Analysis(
    [os.path.join(raiz, "app.py")],
    pathex=[raiz],
    binaries=binaries,
    datas=datas,
    hiddenimports=hiddenimports,
    excludes=["pytest", "_pytest", "PIL", "numpy"],
    noarchive=False,
)
pyz = PYZ(a.pure)
exe = EXE(
    pyz,
    a.scripts,
    [],
    exclude_binaries=True,
    name="Ferramentas1Doc",
    console=False,
    icon=icone,
    upx=False,
)
coll = COLLECT(exe, a.binaries, a.datas, strip=False, upx=False, name="Ferramentas1Doc")
