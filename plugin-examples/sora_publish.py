import argparse
import json
import sys
import threading
import time
from typing import Optional, Dict, Any
import numpy as np

from sora_sdk import (
    Sora,
    SoraConnection,
    SoraAudioSource,
    SoraVideoSource,
)


class SoraPublisher:
    def __init__(self, channel_id: str, signaling_urls: list[str], video_only: bool = False):
        self.channel_id = channel_id
        self.signaling_urls = signaling_urls
        self.video_only = video_only
        self.sora: Optional[Sora] = None
        self.connection: Optional[SoraConnection] = None
        self.audio_source: Optional[SoraAudioSource] = None
        self.video_source: Optional[SoraVideoSource] = None
        self.connected = False
        self.streams: Dict[int, str] = {}  # stream_id -> media_type のマッピング
        self.started = False

        # デフォルトの音声パラメータ
        self.audio_channels = 2
        self.audio_sample_rate = 48000

    def initialize(self):
        """Sora SDK を初期化し、接続を作成"""
        self.sora = Sora()

    def connect(self):
        """Sora に接続"""
        if not self.sora:
            self.initialize()

        # 接続を作成する前に音声・映像ソースを作成
        if not self.video_only:
            self.audio_source = self.sora.create_audio_source(
                self.audio_channels, self.audio_sample_rate
            )
        self.video_source = self.sora.create_video_source()

        # 接続を作成
        self.connection = self.sora.create_connection(
            signaling_urls=self.signaling_urls,
            role="sendonly",
            channel_id=self.channel_id,
            audio=not self.video_only,
            video=True,
            audio_source=self.audio_source if not self.video_only else None,
            video_source=self.video_source,
            video_bit_rate=1000,
        )

        # コールバックを設定
        self.connection.on_notify = self._on_notify
        self.connection.on_disconnect = self._on_disconnect

        # 接続
        self.connection.connect()

        # 接続を待機（簡易実装）
        # 実際の実装では、適切なイベントハンドリングが必要
        time.sleep(2)
        self.connected = True

    def disconnect(self):
        """Sora から切断"""
        if self.connection:
            self.connection.disconnect()
            self.connected = False

    def _on_notify(self, raw_message: str):
        """Sora の notify メッセージを処理"""
        message = json.loads(raw_message)
        if (message.get("type") == "notify" and
            message.get("event_type") == "connection.created"):
            print(f"Sora に接続しました: {message}", file=sys.stderr)

    def _on_disconnect(self, error_code, message: str):
        """Sora の切断を処理"""
        print(f"Sora から切断されました: {error_code} - {message}", file=sys.stderr)
        self.connected = False

    def handle_audio(self, stream_id: int, stereo: bool, sample_rate: int,
                    timestamp_us: int, duration_us: int, data: bytes):
        """Hisui からの音声サンプルを処理"""
        if self.video_only:
            return  # video-only モードでは音声を無視

        if not self.connected or not self.audio_source:
            return
        self.started = True

        # 生音声データを numpy 配列に変換
        # データは I16Be（ビッグエンディアン 16ビット PCM）形式
        audio_array_be = np.frombuffer(data, dtype='>i2')  # ビッグエンディアン 16ビット符号付き
        audio_array = audio_array_be.astype(np.int16)      # ネイティブエンディアン int16 に変換

        # チャンネル用にリシェイプ - Sora は (samples_per_channel, channels) を期待
        channels = 2 if stereo else 1
        if len(audio_array) % channels == 0:
            samples_per_channel = len(audio_array) // channels
            # (samples_per_channel, channels) にリシェイプし、連続メモリを確保するためにコピー
            audio_array = audio_array.reshape(samples_per_channel, channels).copy()
        else:
            print(f"警告: 音声データ長 {len(audio_array)} がチャンネル数 {channels} で割り切れません", file=sys.stderr)
            return

        # Sora に送信 - numpy 配列のオーバーロードを使用
        self.audio_source.on_data(audio_array)

    def handle_video(self, stream_id: int, width: int, height: int,
                    timestamp_us: int, duration_us: int, bgr_data: bytes):
        """Hisui からの映像フレームを処理"""
        if not self.connected or not self.video_source:
            return
        self.started = True

        # BGR データを numpy 配列に変換
        # BGR24 形式（ピクセルあたり3バイト）と仮定
        expected_size = width * height * 3
        if len(bgr_data) != expected_size:
            print(f"警告: {width}x{height} BGR に対して {expected_size} バイトを期待しましたが、{len(bgr_data)} バイトでした", file=sys.stderr)
            return

        frame = np.frombuffer(bgr_data, dtype=np.uint8).reshape((height, width, 3)).copy()

        # Sora に送信
        self.video_source.on_captured(frame)

    def handle_eos(self, stream_id: int):
        """ストリーム終了を処理"""
        if stream_id in self.streams:
            media_type = self.streams[stream_id]
            print(f"{media_type} のストリームが終了しました (stream_id: {stream_id})", file=sys.stderr)
            del self.streams[stream_id]


class HisuiSoraPlugin:
    def __init__(self, channel_id: str, signaling_urls: list[str] = None, video_only: bool = False):
        if signaling_urls is None:
            signaling_urls = ["ws://localhost:3000/signaling"]

        self.publisher = SoraPublisher(channel_id, signaling_urls, video_only)
        self.running = True

    def read_message(self):
        """標準入力から JSON-RPC メッセージを読み取り"""
        # バッファを使用してヘッダーを読み取り
        headers = {}
        while True:
            line = sys.stdin.buffer.readline().decode('utf-8')
            if not line:
                return None, None

            line = line.strip()
            if not line:  # 空行はヘッダーの終了を示す
                break

            if ':' in line:
                key, value = line.split(':', 1)
                headers[key.strip()] = value.strip()

        # コンテンツを読み取り
        content_length = int(headers.get('Content-Length', 0))
        if content_length == 0:
            return None, None

        content_bytes = sys.stdin.buffer.read(content_length)
        content = content_bytes.decode('utf-8')

        # バイナリデータが期待されるかチェックするため JSON を解析
        binary_data = None
        try:
            request = json.loads(content)
            method = request.get('method')

            # バイナリデータを期待するメソッドのみ読み取り
            if method in ['notify_audio', 'notify_video']:
                # バッファを使用してバイナリデータヘッダーを読み取り
                binary_headers = {}
                while True:
                    line = sys.stdin.buffer.readline().decode('utf-8')
                    if not line:
                        break

                    line = line.strip()
                    if not line:  # 空行はヘッダーの終了を示す
                        break

                    if ':' in line:
                        key, value = line.split(':', 1)
                        binary_headers[key.strip()] = value.strip()

                binary_length = int(binary_headers.get('Content-Length', 0))
                if binary_length > 0:
                    binary_data = sys.stdin.buffer.read(binary_length)

        except (json.JSONDecodeError, ValueError, KeyError) as e:
            print(f"JSON 解析またはバイナリデータ読み取りエラー: {e}", file=sys.stderr)

        return content, binary_data

    def send_response(self, response: dict):
        """標準出力に JSON-RPC レスポンスを送信"""
        response_json = json.dumps(response)
        print(f"Content-Length: {len(response_json)}")
        print("Content-Type: application/json")
        print()
        print(response_json, end='')
        sys.stdout.flush()

    def process_request(self, request_data: str, binary_data: bytes = None):
        """JSON-RPC リクエストを処理"""
        try:
            request = json.loads(request_data)
        except json.JSONDecodeError:
            return

        method = request.get('method')
        params = request.get('params', {})
        request_id = request.get('id')

        if method == 'notify_audio':
            stream_id = params['stream_id']
            stereo = params['stereo']
            sample_rate = params['sample_rate']
            timestamp_us = params['timestamp_us']
            duration_us = params['duration_us']

            if not self.publisher.video_only:
                self.publisher.streams[stream_id] = 'audio'
                if binary_data:
                    self.publisher.handle_audio(stream_id, stereo, sample_rate,
                                              timestamp_us, duration_us, binary_data)

        elif method == 'notify_video':
            stream_id = params['stream_id']
            width = params['width']
            height = params['height']
            timestamp_us = params['timestamp_us']
            duration_us = params['duration_us']

            self.publisher.streams[stream_id] = 'video'
            if binary_data:
                self.publisher.handle_video(stream_id, width, height,
                                          timestamp_us, duration_us, binary_data)

        elif method == 'notify_eos':
            stream_id = params['stream_id']
            self.publisher.handle_eos(stream_id)

        elif method == 'poll_output':
            # すべての入力ストリームが EOS に到達したかチェック
            if self.publisher.started and not self.publisher.streams:
                # すべての入力が EOS に到達、処理が完了したことを通知
                if request_id is not None:
                    response = {
                        "jsonrpc": "2.0",
                        "id": request_id,
                        "result": {"type": "finished"}
                    }
                    self.send_response(response)
            else:
                # まだアクティブなストリームがあり、入力を待機中
                if request_id is not None:
                    response = {
                        "jsonrpc": "2.0",
                        "id": request_id,
                        "result": {"type": "waiting_input_any"}
                    }
                    self.send_response(response)

    def run(self):
        """メインプラグインループ"""
        try:
            # Sora を初期化して接続
            self.publisher.initialize()
            self.publisher.connect()

            # Hisui からのメッセージを処理
            while self.running:
                try:
                    message, binary_data = self.read_message()
                    if message is None:
                        break

                    self.process_request(message, binary_data)
                except Exception as e:
                    print(f"メッセージ処理エラー: {e}", file=sys.stderr)
        finally:
            self.publisher.disconnect()


def main():
    parser = argparse.ArgumentParser(description="Sora に配信するための Hisui プラグイン")
    parser.add_argument("--channel-id", required=True, help="Sora チャンネル ID")
    parser.add_argument("--signaling-url", required=True, action="append",
                       help="Sora シグナリング URL（複数回指定可能）")
    parser.add_argument("--video-only", action="store_true",
                       help="映像のみを配信（音声を無効化）")
    args = parser.parse_args()

    plugin = HisuiSoraPlugin(args.channel_id, args.signaling_url, video_only=args.video_only)
    plugin.run()


if __name__ == "__main__":
    main()

