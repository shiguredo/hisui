"""
Hisui - Python wrapper for Recording Composition Tool
"""

import json
import subprocess
from pathlib import Path
from typing import Any


class HisuiError(Exception):
    """Error during Hisui execution"""

    pass


class Hisui:
    """
    Wrapper class for hisui command

    Example:
        with Hisui() as hisui:
            # Get recording file information
            info = hisui.inspect("input.webm")
            print(info)

            # List available codecs
            codecs = hisui.list_codecs()
            print(codecs)
    """

    def __init__(self, verbose: bool = False):
        """
        Initialize Hisui wrapper

        Args:
            verbose: Enable verbose logging
        """
        # When installed via maturin, the binary should be in PATH
        self.binary_path = "hisui"
        self.verbose = verbose

    def __enter__(self):
        return self

    def __exit__(self, exc_type, exc_val, exc_tb):
        pass

    def _run_command(self, args: list[str], capture_output: bool = True) -> str | None:
        """
        Execute hisui command

        Args:
            args: List of command-line arguments
            capture_output: Whether to capture command output

        Returns:
            Command output (if capture_output is True)

        Raises:
            HisuiError: If command execution fails
        """
        cmd = [self.binary_path]
        if self.verbose:
            cmd.append("--verbose")
        cmd.extend(args)

        try:
            result = subprocess.run(
                cmd, capture_output=capture_output, text=True, check=True
            )

            if capture_output:
                return result.stdout
            return None

        except subprocess.CalledProcessError as e:
            error_msg = f"Command failed with exit code {e.returncode}"
            if e.stderr:
                error_msg += f"\nError output:\n{e.stderr}"
            raise HisuiError(error_msg) from e
        except FileNotFoundError:
            raise HisuiError(f"hisui binary not found at: {self.binary_path}") from None

    def inspect(self, input_file: str, decode: bool = False) -> dict[str, Any]:
        """
        Get recording file information

        Args:
            input_file: Input file path
            decode: Whether to perform decoding

        Returns:
            File information as dictionary
        """
        args = ["inspect", input_file]
        if decode:
            args.append("--decode")

        output = self._run_command(args)
        if output:
            return json.loads(output)
        return {}

    def list_codecs(self, openh264: str | None = None) -> dict[str, Any]:
        """
        Get list of available codecs

        Args:
            openh264: Path to OpenH264 shared library

        Returns:
            Codec information as dictionary
        """
        args = ["list-codecs"]
        if openh264:
            args.extend(["--openh264", openh264])

        output = self._run_command(args)
        if output:
            return json.loads(output)
        return {}


__all__ = ["Hisui", "HisuiError"]
