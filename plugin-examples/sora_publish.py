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
    def __init__(self, channel_id: str, signaling_urls: list[str]):
        self.channel_id = channel_id
        self.signaling_urls = signaling_urls
        self.sora: Optional[Sora] = None
        self.connection: Optional[SoraConnection] = None
        self.audio_source: Optional[SoraAudioSource] = None
        self.video_source: Optional[SoraVideoSource] = None
        self.connected = False
        self.streams: Dict[int, str] = {}  # stream_id -> media_type mapping
        self.started = False 

        # Default audio/video parameters
        self.audio_channels = 2
        self.audio_sample_rate = 48000
        self.video_width = 640
        self.video_height = 480

    def initialize(self):
        """Initialize Sora SDK and create connection."""
        self.sora = Sora()

    def connect(self):
        """Connect to Sora."""
        if not self.sora:
            self.initialize()

        # Create audio and video sources before creating connection
        self.audio_source = self.sora.create_audio_source(
            self.audio_channels, self.audio_sample_rate
        )
        self.video_source = self.sora.create_video_source()

        # Create connection
        self.connection = self.sora.create_connection(
            signaling_urls=self.signaling_urls,
            role="sendonly",
            channel_id=self.channel_id,
            audio=True,
            video=True,
            audio_source=self.audio_source,
            video_source=self.video_source,
        )

        # Set up callbacks
        self.connection.on_notify = self._on_notify
        self.connection.on_disconnect = self._on_disconnect

        # Connect
        self.connection.connect()

        # Wait for connection (simple implementation)
        # In a real implementation, you'd want proper event handling
        time.sleep(2)
        self.connected = True

    def disconnect(self):
        """Disconnect from Sora."""
        if self.connection:
            self.connection.disconnect()
            self.connected = False

    def _on_notify(self, raw_message: str):
        """Handle Sora notify messages."""
        message = json.loads(raw_message)
        if (message.get("type") == "notify" and
            message.get("event_type") == "connection.created"):
            print(f"Connected to Sora: {message}", file=sys.stderr)

    def _on_disconnect(self, error_code, message: str):
        """Handle Sora disconnect."""
        print(f"Disconnected from Sora: {error_code} - {message}", file=sys.stderr)
        self.connected = False

    def handle_audio(self, stream_id: int, stereo: bool, sample_rate: int,
                    timestamp_us: int, duration_us: int, data: bytes):
        """Handle audio sample from Hisui."""
        if not self.connected or not self.audio_source:
            return
        self.started = True

        # Convert raw audio data to numpy array
        # Assuming 16-bit PCM audio data
        audio_array = np.frombuffer(data, dtype=np.int16)

        # Reshape for channels
        channels = 2 if stereo else 1
        if len(audio_array) % channels == 0:
            audio_array = audio_array.reshape(-1, channels)

        # Send to Sora
        self.audio_source.on_data(audio_array)

    def handle_video(self, stream_id: int, width: int, height: int,
                    timestamp_us: int, duration_us: int, rgb_data: bytes):
        """Handle video frame from Hisui."""
        if not self.connected or not self.video_source:
            return
        self.started = True

        # Convert RGB data to numpy array
        # Assuming RGB24 format (3 bytes per pixel)
        expected_size = width * height * 3
        if len(rgb_data) != expected_size:
            print(f"Warning: Expected {expected_size} bytes for {width}x{height} RGB, got {len(rgb_data)}", file=sys.stderr)
            return

        frame = np.frombuffer(rgb_data, dtype=np.uint8).reshape((height, width, 3))

        # Send to Sora
        self.video_source.on_captured(frame)

    def handle_eos(self, stream_id: int):
        """Handle end of stream."""
        if stream_id in self.streams:
            media_type = self.streams[stream_id]
            print(f"End of stream for {media_type} (stream_id: {stream_id})", file=sys.stderr)
            del self.streams[stream_id]


class HisuiSoraPlugin:
    def __init__(self, channel_id: str, signaling_urls: list[str] = None):
        if signaling_urls is None:
            signaling_urls = ["ws://localhost:3000/signaling"]

        self.publisher = SoraPublisher(channel_id, signaling_urls)
        self.running = True

    def read_message(self):
        """Read a JSON-RPC message from stdin."""
        # Read headers using buffer
        headers = {}
        while True:
            line = sys.stdin.buffer.readline().decode('utf-8')
            if not line:
                return None, None

            line = line.strip()
            if not line:  # Empty line marks end of headers
                break

            if ':' in line:
                key, value = line.split(':', 1)
                headers[key.strip()] = value.strip()

        # Read content
        content_length = int(headers.get('Content-Length', 0))
        if content_length == 0:
            return None, None

        content_bytes = sys.stdin.buffer.read(content_length)
        content = content_bytes.decode('utf-8')

        # Parse JSON to check if binary data is expected
        binary_data = None
        try:
            request = json.loads(content)
            method = request.get('method')

            # Only read binary data for methods that expect it
            if method in ['notify_audio', 'notify_video']:
                # Read binary data headers using buffer
                binary_headers = {}
                while True:
                    line = sys.stdin.buffer.readline().decode('utf-8')
                    if not line:
                        break

                    line = line.strip()
                    if not line:  # Empty line marks end of headers
                        break

                    if ':' in line:
                        key, value = line.split(':', 1)
                        binary_headers[key.strip()] = value.strip()

                binary_length = int(binary_headers.get('Content-Length', 0))
                print(f"binary len: {binary_length}", file=sys.stderr)
                print(f"headers: {binary_headers}", file=sys.stderr)
                if binary_length > 0:
                    # ここでブロックする理由を検討してください
                    # rust側とpython側でデータの長さはあっています
                    binary_data = sys.stdin.buffer.read(binary_length)
                    print(f"read binary data: {len(binary_data)} bytes", file=sys.stderr)
                print(f"headers: {binary_headers}", file=sys.stderr)

        except (json.JSONDecodeError, ValueError, KeyError) as e:
            print(f"Error parsing JSON or reading binary data: {e}", file=sys.stderr)

        return content, binary_data

    def send_response(self, response: dict):
        """Send a JSON-RPC response to stdout."""
        response_json = json.dumps(response)
        print(f"res: {response_json}", file=sys.stderr)
        print(f"Content-Length: {len(response_json)}")
        print("Content-Type: application/json")
        print()
        print(response_json, end='')
        sys.stdout.flush()

    def process_request(self, request_data: str, binary_data: bytes = None):
        """Process a JSON-RPC request."""
        print(f"req: {request_data}", file=sys.stderr)
 
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
            # Check if all input streams have reached EOS
            if self.publisher.started and not self.publisher.streams:
                # All inputs have reached EOS, signal that processing is finished
                if request_id is not None:
                    response = {
                        "jsonrpc": "2.0",
                        "id": request_id,
                        "result": {"type": "finished"}
                    }
                    self.send_response(response)
            else:
                # Still have active streams, waiting for any input
                if request_id is not None:
                    response = {
                        "jsonrpc": "2.0",
                        "id": request_id,
                        "result": {"type": "waiting_input_any"}
                    }
                    self.send_response(response)

    def run(self):
        """Main plugin loop."""
        try:
            # Initialize and connect to Sora
            self.publisher.initialize()
            self.publisher.connect()

            # Process messages from Hisui
            while self.running:
                try:
                    message, binary_data = self.read_message()
                    if message is None:
                        break

                    self.process_request(message, binary_data)
                except Exception as e:
                    print(f"Error processing message: {e}", file=sys.stderr)

        except KeyboardInterrupt:
            print("Interrupted", file=sys.stderr)
        finally:
            self.publisher.disconnect()


def main():
    parser = argparse.ArgumentParser(description="Hisui plugin for publishing to Sora")
    parser.add_argument("--channel-id", required=True, help="Sora channel ID")
    parser.add_argument("--signaling-url", action="append",
                       help="Sora signaling URL (can be specified multiple times)")

    args = parser.parse_args()

    signaling_urls = args.signaling_url or ["ws://localhost:3000/signaling"]

    plugin = HisuiSoraPlugin(args.channel_id, signaling_urls)
    plugin.run()


if __name__ == "__main__":
    main()

