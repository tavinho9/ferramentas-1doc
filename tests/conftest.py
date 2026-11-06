import os
import sys

import pytest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))


@pytest.fixture
def dados(tmp_path, monkeypatch):
    """Pasta de dados e de saída temporárias; config com a planilha não abrindo."""
    from ferramentas1doc import caminhos, config, sessao

    pasta = tmp_path / "dados"
    pasta.mkdir()
    saida = tmp_path / "saida"
    saida.mkdir()
    monkeypatch.setattr(caminhos, "pasta_dados", lambda: str(pasta))
    monkeypatch.setattr(caminhos, "area_de_trabalho", lambda: str(saida))
    config.salvar({**config.PADRAO, "abrir_planilha": False, "pasta_saida": str(saida)})
    sessao.CANCELAR.clear()
    yield {"dados": pasta, "saida": saida}
    sessao.CANCELAR.clear()


@pytest.fixture
def cred():
    from ferramentas1doc.sessao import Credenciais
    return Credenciais(cookies=[{"name": "PHPSESSID", "value": "abc", "domain": "montesclaros.1doc.com.br",
                                 "path": "/"}], ua="Mozilla/5.0 (teste)", setor="SPOT-DCL-GP-CPPFI",
                       email="teste@montesclaros.mg.gov.br")
