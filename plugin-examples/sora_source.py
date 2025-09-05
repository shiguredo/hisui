import argparse
import json
import sys
import threading
import time
from typing import Optional, Dict, Any, List
import queue
import numpy as np

from sora_sdk import (
    Sora,
    SoraConnection,
    SoraVideoSink,
    SoraMediaTrack,
    SoraVideoFrame,
)

# プログラムの開始時刻を記録
START_TIME = time.time()

class SoraReceiver:
    def __init__(self, channel_id: str, signaling_urls: list[str],
                 video_stream_names: List[str]):
        self.channel_id = channel_id
        self.signaling_urls = signaling_urls
        self.video_stream_names = video_stream_names
        self.sora: Optional[Sora] = None
        self.connection: Optional[SoraConnection] = None
        self.connected = False

        # ストリーム名とID のマッピング
        self.stream_name_to_id: Dict[str, int] = {}
        self.next_stream_id = 1

        # 受信データのキュー（映像のみ）
        self.video_queue = queue.Queue()
        self.finished_streams = set()

        # video sinks
        self.video_sinks: List[SoraVideoSink] = []

    def initialize(self):
        """Sora SDK を初期化し、接続を作成"""
        self.sora = Sora()

    def connect(self):
        """Sora に接続"""
        if not self.sora:
            self.initialize()

        # 接続を作成（映像のみ）
        self.connection = self.sora.create_connection(
            signaling_urls=self.signaling_urls,
            role="recvonly",
            channel_id=self.channel_id,
            audio=False,  # 音声は無効
            video=bool(self.video_stream_names),
        )

        # コールバックを設定
        self.connection.on_notify = self._on_notify
        self.connection.on_disconnect = self._on_disconnect
        self.connection.on_track = self._on_track

        # 接続
        self.connection.connect()

        # 接続を待機（簡易実装）
        time.sleep(1)
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

        # 全てのストリームを終了としてマーク
        for stream_name in self.video_stream_names:
            if stream_name in self.stream_name_to_id:
                stream_id = self.stream_name_to_id[stream_name]
                self.finished_streams.add(stream_id)

    def _on_track(self, track: SoraMediaTrack):
        """トラックを受信した時の処理"""
        if track.kind == "video" and self.video_stream_names:
            # 映像ストリーム名を順番に割り当て
            stream_name = self.video_stream_names[len(self.video_sinks) % len(self.video_stream_names)]
            stream_id = self.next_stream_id
            self.next_stream_id += 1
            self.stream_name_to_id[stream_name] = stream_id

            video_sink = SoraVideoSink(track)
            video_sink.on_frame = lambda frame: self._on_video_frame(stream_id, stream_name, frame)
            self.video_sinks.append(video_sink)
            print(f"映像トラックを受信: {stream_name} (stream_id: {stream_id})", file=sys.stderr)

    def _on_video_frame(self, stream_id: int, stream_name: str, frame: SoraVideoFrame):
        """映像フレームを受信した時の処理"""
        if not self.connected:
            return

        # フレームをBGR形式のnumpy配列として取得
        bgr_data = frame.data()

        # numpy配列の形状から幅と高さを取得 (H x W x BGR)
        height, width = bgr_data.shape[:2]

        # タイムスタンプとデュレーション（簡易実装）
        duration_us = int(1_000_000 / 30)  # 30 FPS と仮定
        timestamp_us = int((time.time() - START_TIME) * 1_000_000)  # プログラム開始時刻を起点にする

        self.video_queue.put({
            'stream_id': stream_id,
            'stream_name': stream_name,
            'width': width,
            'height': height,
            'timestamp_us': timestamp_us,
            'duration_us': duration_us,
            'data': bgr_data.flatten().tobytes()  # フラットな配列にしてバイト列に変換
        })


class HisuiSoraSourcePlugin:
    def __init__(self, channel_id: str, signaling_urls: list[str] = None,
                 video_stream_names: List[str] = None):
        if signaling_urls is None:
            signaling_urls = ["ws://localhost:3000/signaling"]
        if video_stream_names is None:
            video_stream_names = []

        self.receiver = SoraReceiver(channel_id, signaling_urls, video_stream_names)
        self.running = True

    def read_message(self):
        """標準入力から JSON-RPC メッセージを読み取り"""
        headers = {}
        while True:
            line = sys.stdin.buffer.readline().decode('utf-8')
            if not line:
                return None

            line = line.strip()
            if not line:  # 空行はヘッダーの終了を示す
                break

            if ':' in line:
                key, value = line.split(':', 1)
                headers[key.strip()] = value.strip()

        # コンテンツを読み取り
        content_length = int(headers.get('Content-Length', 0))
        if content_length == 0:
            return None

        content_bytes = sys.stdin.buffer.read(content_length)
        content = content_bytes.decode('utf-8')
        return content

    def send_response(self, response: dict):
        """標準出力に JSON-RPC レスポンスを送信"""
        response_json = json.dumps(response)

        print(f"Content-Length: {len(response_json)}")
        print("Content-Type: application/json")
        print()
        print(response_json, end='')
        sys.stdout.flush()

    def send_response_with_payload(self, response: dict, payload: bytes):
        """標準出力に JSON-RPC レスポンス（ペイロード付き）を送信"""
        response_json = json.dumps(response)
        print(f"res2: {response_json}", file=sys.stderr)

        print(f"Content-Length: {len(response_json)}")
        print("Content-Type: application/json")
        print()
        print(response_json, end='')

        # ペイロードのヘッダーとデータを送信
        print(f"Content-Length: {len(payload)}")
        print("Content-Type: application/octet-stream")
        print()
        sys.stdout.flush()

        sys.stdout.buffer.write(payload)
        sys.stdout.flush()

    def process_request(self, request_data: str):
        """JSON-RPC リクエストを処理"""
        try:
            request = json.loads(request_data)
        except json.JSONDecodeError:
            return

        method = request.get('method')
        request_id = request.get('id')

        if method == 'poll_output':
            # 映像データを処理
            if not self.receiver.video_queue.empty():
                video_data = self.receiver.video_queue.get_nowait()
                if request_id is not None:
                    response = {
                        "jsonrpc": "2.0",
                        "id": request_id,
                        "result": {
                            "type": "video_frame",
                            "stream_name": video_data['stream_name'],
                            "width": video_data['width'],
                            "height": video_data['height'],
                            "timestamp_us": video_data['timestamp_us'],
                            "duration_us": video_data['duration_us']
                        }
                    }
                    self.send_response_with_payload(response, video_data['data'])
                return

            # 全てのストリームが終了したかチェック
            expected_stream_count = len(self.receiver.video_stream_names)
            if len(self.receiver.finished_streams) >= expected_stream_count and expected_stream_count > 0:
                if request_id is not None:
                    response = {
                        "jsonrpc": "2.0",
                        "id": request_id,
                        "result": {"type": "finished"}
                    }
                    self.send_response(response)
            else:
                # データを待機中
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
            self.receiver.initialize()
            self.receiver.connect()

            # Hisui からのメッセージを処理
            while self.running:
                try:
                    message = self.read_message()
                    if message is None:
                        break

                    self.process_request(message)
                except Exception as e:
                    print(f"メッセージ処理エラー: {e}", file=sys.stderr)
        finally:
            self.receiver.disconnect()


def main():
    parser = argparse.ArgumentParser(description="Sora から映像を受信するための Hisui プラグイン")
    parser.add_argument("--channel-id", required=True, help="Sora チャンネル ID")
    parser.add_argument("--signaling-url", required=True, action="append",
                       help="Sora シグナリング URL（複数回指定可能）")
    parser.add_argument("--video-stream-name", action="append", default=[],
                       help="映像ストリーム名（複数回指定可能）")
    args = parser.parse_args()

    plugin = HisuiSoraSourcePlugin(
        args.channel_id,
        args.signaling_url,
        args.video_stream_name
    )
    plugin.run()


if __name__ == "__main__":
    main()

