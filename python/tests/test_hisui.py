import pytest
from hisui import Hisui, HisuiError


def test_list_codecs():
    """list_codecs メソッドがコーデック情報を返すことをテスト"""
    with Hisui() as h:
        codecs = h.list_codecs()

        # 結果が辞書であることを確認
        assert isinstance(codecs, dict)

        # 'codecs' キーが存在することを確認
        assert "codecs" in codecs

        # codecs がリストであることを確認
        assert isinstance(codecs["codecs"], list)

        # 利用可能なコーデックが存在することを確認
        assert len(codecs["codecs"]) > 0

        # コーデックの構造を確認
        for codec in codecs["codecs"]:
            assert "name" in codec
            assert "type" in codec
            assert codec["type"] in ["audio", "video"]


def test_list_codecs_verbose():
    """verbose モードが有効な状態での list_codecs のテスト"""
    with Hisui(verbose=True) as h:
        codecs = h.list_codecs()

        assert isinstance(codecs, dict)
        assert "codecs" in codecs


def test_inspect_invalid_file():
    """inspect で無効なファイルを指定した場合のエラーハンドリングをテスト"""
    with Hisui() as h:
        with pytest.raises(HisuiError):
            # 存在しないファイルを inspect しようとする
            h.inspect("/nonexistent/file.mp4")
